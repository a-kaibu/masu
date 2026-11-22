//! The saves: what the game changed of the namespace the store gave, as records the page keeps
//! (web/src/saves.ts) and the next start puts back ("@saves", Fs::load). A file in memory is
//! its bytes, as before; what else changed is said against the namespace as loaded
//! (`Fs::base`): a name the store gave that is gone (deleted, or renamed away), a directory, a
//! file from the store, or a link at a name where the store had none or had something else
//! (made, renamed there, its mode or time changed), and a file in memory's mode. Before, only
//! the files in memory were kept: a file from the store deleted or renamed came back at the
//! next start, a renamed one under its old name only (review of 6852be1, §1).
//!
//! A gone name carries what it took away: put back, it takes away only that (the same
//! directory, the same chunks, the same link), never what a newer tree has at that name.
//!
//! Records: a file in memory is path length (u32), path, mtime (i64), length (u32), bytes, as
//! the page has always kept them. Any other is TAG | its kind (u32: no path is 4 GiB long),
//! path length (u32), path, body length (u32), body; the page reads both (file-records.ts).
use super::*;

/// The first word of a record that is not a file in memory's (its low byte the kind).
pub(super) const TAG: u32 = 0xffff_ff00;

/// A record's kind past TAG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    /// the name is gone: body an [`Entry`], what the store had there
    Gone,
    /// the name is this: body an [`Entry`]
    Put,
    /// a file in memory's mode: body the mode (u32)
    Mode,
    /// a store file written to (vfs/over.rs): body the store's file as an [`Entry`] (u32
    /// length, the entry: the file's mode and time), then the pages written over it
    /// (`Over::put_shape`)
    Over,
}

impl Tag {
    fn word(self) -> u32 {
        TAG | match self {
            Tag::Gone => 1,
            Tag::Put => 2,
            Tag::Mode => 3,
            Tag::Over => 4,
        }
    }
    fn of(w: u32) -> Option<Tag> {
        match w ^ TAG {
            1 => Some(Tag::Gone),
            2 => Some(Tag::Put),
            3 => Some(Tag::Mode),
            4 => Some(Tag::Over),
            _ => None,
        }
    }
}

/// What a name is, but a file in memory (its bytes are their own record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Dir,
    Lower { size: u64, chunks: Arc<Vec<Option<Hash>>>, starts: Arc<Vec<u64>> },
    Link(String),
}

/// A name as the store gave it or as it is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) kind: Kind,
    pub(super) mode: u32,
    pub(super) mtime: i64,
}

/// What can stand at one name: the records of one name replace each other within these only (a
/// directory is not made a file by writing it: it goes first).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Dir,
    File,
    Link,
}

impl Kind {
    fn family(&self) -> Family {
        match self {
            Kind::Dir => Family::Dir,
            Kind::Lower { .. } => Family::File,
            Kind::Link(_) => Family::Link,
        }
    }
}

impl Entry {
    /// `n` as an entry; None for a file in memory.
    pub(super) fn of(n: &Node) -> Option<Entry> {
        let kind = match &n.data {
            Data::Dir(_) => Kind::Dir,
            Data::Link(t) => Kind::Link(t.clone()),
            Data::File(Content::Lower { size, chunks, starts }) => Kind::Lower { size: *size, chunks: chunks.clone(), starts: starts.clone() },
            Data::File(Content::Upper(_) | Content::Over(_)) => return None,
        };
        Some(Entry { kind, mode: n.mode, mtime: n.mtime })
    }

    fn put(&self, out: &mut Vec<u8>) {
        out.extend(self.mode.to_le_bytes());
        out.extend(self.mtime.to_le_bytes());
        match &self.kind {
            Kind::Dir => out.push(1),
            Kind::Lower { size, chunks, starts } => {
                out.push(2);
                out.extend(size.to_le_bytes());
                out.extend(u32::try_from(chunks.len()).unwrap_or(u32::MAX).to_le_bytes());
                for (h, end) in chunks.iter().zip(starts.iter().skip(1)) {
                    match h {
                        Some(h) => {
                            out.push(1);
                            out.extend(h);
                        }
                        None => out.push(0),
                    }
                    out.extend(end.to_le_bytes());
                }
            }
            Kind::Link(t) => {
                out.push(3);
                out.extend(t.as_bytes());
            }
        }
    }

    /// An entry as `put` wrote it, all of `b` (None: not one; a chunk list whose ends do not
    /// rise to its size).
    fn get(b: &[u8]) -> Option<Entry> {
        let (mode, b) = b.split_first_chunk::<4>()?;
        let (mtime, b) = b.split_first_chunk::<8>()?;
        let (kind, mut b) = b.split_first()?;
        let kind = match kind {
            1 if b.is_empty() => Kind::Dir,
            2 => {
                let (size, rest) = b.split_first_chunk::<8>()?;
                let (n, rest) = rest.split_first_chunk::<4>()?;
                let (size, n) = (u64::from_le_bytes(*size), usize::try_from(u32::from_le_bytes(*n)).ok()?);
                b = rest;
                // (each chunk at least 9 bytes: no list longer than what is there)
                if n > b.len() / 9 {
                    return None;
                }
                let mut chunks = Vec::with_capacity(n);
                let mut starts = vec![0u64];
                for _ in 0..n {
                    let (has, rest) = b.split_first()?;
                    let (h, rest) = match has {
                        0 => (None, rest),
                        1 => {
                            let (h, rest) = rest.split_first_chunk::<16>()?;
                            (Some(*h), rest)
                        }
                        _ => return None,
                    };
                    let (end, rest) = rest.split_first_chunk::<8>()?;
                    let end = u64::from_le_bytes(*end);
                    if end <= *starts.last()? {
                        return None;
                    }
                    chunks.push(h);
                    starts.push(end);
                    b = rest;
                }
                if !b.is_empty() || *starts.last()? != size {
                    return None;
                }
                Kind::Lower { size, chunks: Arc::new(chunks), starts: Arc::new(starts) }
            }
            3 => Kind::Link(String::from_utf8(b.to_vec()).ok()?),
            _ => return None,
        };
        Some(Entry { kind, mode: u32::from_le_bytes(*mode), mtime: i64::from_le_bytes(*mtime) })
    }
}

/// One record of the saves, read back.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Record {
    File { path: String, data: Vec<u8>, mtime: i64 },
    Gone { path: String, was: Entry },
    Put { path: String, entry: Entry },
    Mode { path: String, mode: u32 },
    Over { path: String, mode: u32, mtime: i64, over: Box<over::Over> },
}

impl Record {
    #[cfg(test)]
    pub(super) fn path(&self) -> &str {
        match self {
            Record::File { path, .. } | Record::Gone { path, .. } | Record::Put { path, .. } | Record::Mode { path, .. } | Record::Over { path, .. } => path,
        }
    }
}

/// The records of `b` as the page kept them, up to the first one cut short or not one.
pub(super) fn parse_records(mut b: &[u8]) -> Vec<Record> {
    let mut out = Vec::new();
    // (a record cut short ends them: split by what is there, no sum that wraps on wasm32 for
    // a length near 4 GiB)
    while let Some((r, rest)) = record(b) {
        out.push(r);
        b = rest;
    }
    out
}

fn path_of(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn take_u32(b: &[u8]) -> Option<(usize, &[u8])> {
    let (n, b) = b.split_first_chunk::<4>()?;
    Some((usize::try_from(u32::from_le_bytes(*n)).ok()?, b))
}

/// The first record of `b` and what follows it.
fn record(b: &[u8]) -> Option<(Record, &[u8])> {
    let (first, rest) = b.split_first_chunk::<4>()?;
    let first = u32::from_le_bytes(*first);
    if first & TAG != TAG {
        // a file in memory: path, mtime, length, bytes
        let (path, b) = rest.split_at_checked(usize::try_from(first).ok()?)?;
        let (mtime, b) = b.split_first_chunk::<8>()?;
        let (size, b) = take_u32(b)?;
        let (data, b) = b.split_at_checked(size)?;
        return Some((Record::File { path: path_of(path), data: data.to_vec(), mtime: i64::from_le_bytes(*mtime) }, b));
    }
    let tag = Tag::of(first)?;
    let (n, b) = take_u32(rest)?;
    let (path, b) = b.split_at_checked(n)?;
    let (n, b) = take_u32(b)?;
    let (body, b) = b.split_at_checked(n)?;
    let path = path_of(path);
    let r = match tag {
        Tag::Gone => Record::Gone { path, was: Entry::get(body)? },
        Tag::Put => Record::Put { path, entry: Entry::get(body)? },
        Tag::Mode => Record::Mode { path, mode: u32::from_le_bytes(*body.first_chunk::<4>().filter(|_| body.len() == 4)?) },
        Tag::Over => {
            let (n, body) = take_u32(body)?;
            let (entry, shape) = body.split_at_checked(n)?;
            let Entry { kind: Kind::Lower { size, chunks, starts }, mode, mtime } = Entry::get(entry)? else { return None };
            let (over, rest) = over::Over::get_shape(over::Store { size, chunks, starts }, shape)?;
            if !rest.is_empty() {
                return None;
            }
            Record::Over { path, mode, mtime, over: Box::new(over) }
        }
    };
    Some((r, b))
}

/// One record to write: what it says, borrowed from the namespace.
/// (each its own: the bytes of a file in memory its pages, shared with the file until one of
/// them is written, crate::pages: taken under the VFS lock, written out after it is let go)
enum Out {
    File { mtime: i64, data: crate::pages::Pages },
    Gone(Entry),
    Put(Entry),
    Mode(u32),
    /// a store file written to, its mode and time
    Over { mode: u32, mtime: i64, over: Box<over::Over> },
}

/// A record and its name, in the order they are put back: names gone first (deepest first: a
/// directory's names before it), then directories made (parents first), the other names,
/// the files in memory, their modes.
pub(super) struct Saved {
    rank: (u8, std::cmp::Reverse<String>, String),
    path: String,
    what: Out,
}

impl Saved {
    fn new(path: String, what: Out) -> Saved {
        let rank = match &what {
            Out::Gone(_) => (0, std::cmp::Reverse(path.clone()), String::new()),
            Out::Put(Entry { kind: Kind::Dir, .. }) => (1, std::cmp::Reverse(String::new()), path.clone()),
            Out::Put(_) => (2, std::cmp::Reverse(String::new()), path.clone()),
            Out::File { .. } | Out::Over { .. } => (3, std::cmp::Reverse(String::new()), path.clone()),
            Out::Mode(_) => (4, std::cmp::Reverse(String::new()), path.clone()),
        };
        Saved { rank, path, what }
    }

    fn body(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match &self.what {
            Out::Gone(e) => e.put(&mut out),
            Out::Put(e) => e.put(&mut out),
            Out::Mode(m) => out.extend(m.to_le_bytes()),
            Out::File { .. } | Out::Over { .. } => {}
        }
        out
    }

    /// An Over record's store entry (its body starts with it).
    fn store_entry(mode: u32, mtime: i64, over: &over::Over) -> Vec<u8> {
        let s = over.store();
        let mut e = Vec::new();
        Entry { kind: Kind::Lower { size: s.size, chunks: s.chunks.clone(), starts: s.starts.clone() }, mode, mtime }.put(&mut e);
        e
    }

    /// The record's bytes.
    fn size(&self) -> usize {
        match &self.what {
            Out::File { data, .. } => self.path.len().saturating_add(16).saturating_add(usize::try_from(data.len()).unwrap_or(usize::MAX)),
            Out::Gone(_) | Out::Put(_) | Out::Mode(_) => self.path.len().saturating_add(12).saturating_add(self.body().len()),
            Out::Over { mode, mtime, over } => self.path.len().saturating_add(16).saturating_add(Saved::store_entry(*mode, *mtime, over).len()).saturating_add(over.shape_len()),
        }
    }

    /// The bytes a file in memory holds (what the page lists: 0 for the other records).
    pub(super) fn len(&self) -> u64 {
        match &self.what {
            Out::File { data, .. } => data.len(),
            Out::Over { over, .. } => over.len(),
            Out::Gone(_) | Out::Put(_) | Out::Mode(_) => 0,
        }
    }

    pub(super) fn path(&self) -> &str {
        &self.path
    }

    fn write(&self, out: &mut dyn super::Out) {
        let n = u32::try_from(self.path.len()).unwrap_or(u32::MAX).to_le_bytes();
        let tag = match &self.what {
            Out::File { mtime, data } => {
                out.put(&n);
                out.put(self.path.as_bytes());
                out.put(&mtime.to_le_bytes());
                out.put(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
                data.for_each(|b| out.put(b));
                return;
            }
            Out::Over { mode, mtime, over } => {
                // (the pages written straight out of the file's pages: no copy of them first)
                let entry = Saved::store_entry(*mode, *mtime, over);
                let body = entry.len().saturating_add(4).saturating_add(over.shape_len());
                out.put(&Tag::Over.word().to_le_bytes());
                out.put(&n);
                out.put(self.path.as_bytes());
                out.put(&u32::try_from(body).unwrap_or(u32::MAX).to_le_bytes());
                out.put(&u32::try_from(entry.len()).unwrap_or(u32::MAX).to_le_bytes());
                out.put(&entry);
                over.put_shape(&mut |b| out.put(b));
                return;
            }
            Out::Gone(_) => Tag::Gone,
            Out::Put(_) => Tag::Put,
            Out::Mode(_) => Tag::Mode,
        };
        let body = self.body();
        out.put(&tag.word().to_le_bytes());
        out.put(&n);
        out.put(self.path.as_bytes());
        out.put(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.put(&body);
    }
}

/// `p` under (or at) `root`.
fn under(p: &str, root: &str) -> bool {
    root == "/" || p == root || p.strip_prefix(root).is_some_and(|r| r.starts_with('/'))
}

/// A new file's mode (open_handle's 0o644 for put_file).
const FILE_MODE: u32 = 0o644;

impl Fs {
    /// Every name of the namespace as it is, but the files in memory (what `base` keeps after
    /// loading).
    pub(super) fn entries(&self) -> BTreeMap<String, Entry> {
        let mut out = BTreeMap::new();
        self.each_under("/", &mut |p, n| {
            if let Some(e) = Entry::of(n) {
                out.insert(p.to_owned(), e);
            }
        });
        out
    }

    #[cfg(test)]
    pub(super) fn each_under_for_tests<'a>(&'a self, path: &str, f: &mut dyn FnMut(&str, &'a Node)) {
        self.each_under(path, f)
    }

    /// Each name under `path` (it too) and its node, not through links.
    fn each_under<'a>(&'a self, path: &str, f: &mut dyn FnMut(&str, &'a Node)) {
        let Some((at, rel)) = self.split_mount(path) else { return };
        let Some(ino) = self.walk(at, rel) else { return };
        let mut todo = vec![(ino, path.to_owned())];
        while let Some((ino, p)) = todo.pop() {
            let Some(n) = self.node(ino) else { continue };
            f(&p, n);
            if let Data::Dir(ch) = &n.data {
                let base = if p == "/" { "" } else { p.as_str() };
                todo.extend(ch.iter().map(|(name, &i)| (i, format!("{base}/{name}"))));
            }
        }
    }

    /// The records for what changed under each of `roots`, each once, in the order they go back.
    pub(super) fn saved(&self, roots: &[String]) -> Vec<Saved> {
        let mut out: BTreeMap<String, Vec<Saved>> = BTreeMap::new();
        for root in roots {
            let mut now: HashMap<String, Family> = HashMap::new();
            let mut here = Vec::new();
            self.each_under(root, &mut |p, n| {
                let was = self.base.get(p);
                let family = match Entry::of(n) {
                    Some(e) => {
                        let f = e.kind.family();
                        if was != Some(&e) {
                            here.push(Saved::new(p.to_owned(), Out::Put(e)));
                        }
                        f
                    }
                    None => {
                        if let Data::File(Content::Over(o)) = &n.data {
                            // (the pages written and the store file under them: never all its bytes)
                            here.push(Saved::new(p.to_owned(), Out::Over { mode: n.mode, mtime: n.mtime, over: o.clone() }));
                            now.insert(p.to_owned(), Family::File);
                            return;
                        }
                        let Data::File(Content::Upper(data)) = &n.data else { return };
                        here.push(Saved::new(p.to_owned(), Out::File { mtime: n.mtime, data: data.clone() }));
                        // (put_file keeps the mode of the file it writes into, else gives FILE_MODE)
                        let given = was.filter(|w| w.kind.family() == Family::File).map_or(FILE_MODE, |w| w.mode);
                        if n.mode != given {
                            here.push(Saved::new(p.to_owned(), Out::Mode(n.mode)));
                        }
                        Family::File
                    }
                };
                now.insert(p.to_owned(), family);
            });
            let gone = self.base.range::<str, _>((std::ops::Bound::Included(root.as_str()), std::ops::Bound::Unbounded)).take_while(|(p, _)| under(p, root));
            for (p, was) in gone {
                if now.get(p) != Some(&was.kind.family()) {
                    here.push(Saved::new(p.clone(), Out::Gone(was.clone())));
                }
            }
            for s in here {
                let same = out.entry(s.path.clone()).or_default();
                if !same.iter().any(|x| x.rank.0 == s.rank.0) {
                    same.push(s);
                }
            }
        }
        let mut all: Vec<Saved> = out.into_values().flatten().collect();
        all.sort_by(|a, b| a.rank.cmp(&b.rank));
        all
    }

    /// The records' byte count.
    pub(super) fn saved_size(saved: &[Saved]) -> usize {
        saved.iter().map(Saved::size).fold(0, usize::saturating_add)
    }

    pub(super) fn write_saved(saved: &[Saved], out: &mut dyn super::Out) {
        for s in saved {
            s.write(out);
        }
    }

    /// Puts a record back (at the start, before the program runs).
    pub(super) fn put_back(&mut self, r: Record) {
        match r {
            Record::File { path, data, mtime } => self.put_file(&path, data, mtime),
            Record::Gone { path, was } => self.put_gone(&path, &was),
            Record::Put { path, entry } => self.put_entry(&path, entry),
            Record::Over { path, mode, mtime, over } => self.put_data(&path, Data::File(Content::Over(over)), mode, mtime),
            Record::Mode { path, mode } => {
                if let Some(ino) = self.split_mount(&path).and_then(|(at, rel)| self.walk(at, rel)) && let Some(n) = self.meta_mut(ino) {
                    n.mode = mode;
                }
            }
        }
    }

    /// The directory holding `path` and its last name (a mount's own root has none).
    fn parent_of<'p>(&self, path: &'p str) -> Option<(Ino, &'p str)> {
        let (dir, name) = path.rsplit_once('/')?;
        let dir = if dir.is_empty() { "/" } else { dir };
        let (at, rel) = self.split_mount(dir)?;
        Some((self.walk(at, rel)?, name)).filter(|(_, n)| !n.is_empty())
    }

    /// Takes `path` away if it is still what the store had there (`was`); a directory only
    /// once nothing is in it.
    fn put_gone(&mut self, path: &str, was: &Entry) {
        let Some((parent, name)) = self.parent_of(path) else { return };
        let Some(Data::Dir(ch)) = self.node(parent).map(|n| &n.data) else { return };
        let Some(&ino) = ch.get(name) else { return };
        if self.mounts.iter().any(|m| m.1 == ino) {
            return;
        }
        let Some(n) = self.node(ino) else { return };
        let same = match (&n.data, &was.kind) {
            (Data::Dir(c), Kind::Dir) => c.is_empty(),
            (Data::File(Content::Lower { size, chunks, starts }), Kind::Lower { size: s, chunks: c, starts: t }) => size == s && chunks == c && starts == t,
            (Data::Link(t), Kind::Link(w)) => t == w,
            (Data::Dir(_), Kind::Lower { .. } | Kind::Link(_))
            | (Data::File(_), Kind::Dir | Kind::Link(_))
            | (Data::File(Content::Upper(_) | Content::Over(_)), Kind::Lower { .. })
            | (Data::Link(_), Kind::Dir | Kind::Lower { .. }) => false,
        };
        if !same {
            return;
        }
        if was.kind == Kind::Dir {
            // (as rmdir takes it: its own links and its parent's ".." link)
            if let Some(n) = self.meta_mut(ino) {
                n.nlink = 0;
            }
            if let Some(n) = self.meta_mut(parent) {
                n.nlink = n.nlink.saturating_sub(1);
            }
        }
        self.unlink_name(parent, name);
    }

    /// Makes `path` the entry (its directories made where missing); a directory there stays one
    /// (its mode and time set), a file there is replaced, anything else there stays.
    fn put_entry(&mut self, path: &str, e: Entry) {
        let data = match e.kind {
            Kind::Dir => Data::Dir(BTreeMap::new()),
            Kind::Lower { size, chunks, starts } => Data::File(Content::Lower { size, chunks, starts }),
            Kind::Link(t) => Data::Link(t),
        };
        self.put_data(path, data, e.mode, e.mtime)
    }

    /// put_entry of what a node holds, its mode and time.
    fn put_data(&mut self, path: &str, data: Data, mode: u32, mtime: i64) {
        let Some((dir, name)) = path.rsplit_once('/') else { return };
        let Some((at, rel)) = self.split_mount(if dir.is_empty() { "/" } else { dir }) else { return };
        let parent = self.mkdir_p(at, rel);
        let old = match self.node(parent).map(|n| &n.data) {
            Some(Data::Dir(ch)) => ch.get(name).copied(),
            _ => return,
        };
        // (the root of "/" has no name in a directory)
        let old = if path == "/" { self.mount_at("/") } else { old };
        let dir = matches!(data, Data::Dir(_));
        match old.map(|i| (i, self.node(i).map(|n| matches!(n.data, Data::Dir(_))), dir)) {
            Some((i, Some(true), true)) => {
                if let Some(n) = self.meta_mut(i) {
                    (n.mode, n.mtime) = (mode, mtime);
                }
            }
            Some((i, Some(false), false)) => {
                if let Some(n) = self.node_mut(i) {
                    (n.data, n.mode, n.mtime) = (data, mode, mtime);
                }
            }
            // (a directory where a file goes, or the other way: the gone record before it took
            // away the store's; one still there is a name the game made since)
            Some((_, Some(true), false) | (_, Some(false), true) | (_, None, true | false)) => {}
            None => {
                let i = self.add(data, mode, mtime);
                if let Some(Data::Dir(ch)) = self.data_mut(parent) {
                    ch.insert(name.to_owned(), i);
                }
                if dir && let Some(n) = self.meta_mut(parent) {
                    n.nlink = n.nlink.saturating_add(1);
                }
            }
        }
    }
}

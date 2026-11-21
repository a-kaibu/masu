//! A packed tree's list, `trees/<name>.tsv` (written by `masu pack`, read by masu's server and
//! kura's file system: crates/masu/src/main.rs includes this file). One parser and one writer
//! for every reader; no dependencies.
//!
//! ```text
//! vnvm-tree	2	<average chunk size>
//! d	<mode>	<mtime>	<path>
//! f	<mode>	<mtime>	<path>	<size>	<hash>:<length>,...     (":<length>": all zero)
//! l	<mode>	<mtime>	<path>	<target>
//! ```
//!
//! Paths are relative to the tree, parents before children; modes in octal; a path or target
//! has its tab, newline and backslash escaped (`\t`, `\n`, `\\`). A hash is the first 16 bytes
//! of the chunk's BLAKE3, in hex. A file's chunks' lengths add up to its size: [`Chunks`] holds
//! the file's size as their sum, so a list whose sums are off is not one that can be made.

use std::fmt;
use std::num::NonZeroU64;

/// The list's first line's first field.
pub const MAGIC: &str = "vnvm-tree";
/// The version `masu pack` writes.
pub const VERSION: u32 = 2;
/// A chunk's length at most (FastCDC's longest is 4x its average, which pack.rs caps at 64 MiB).
pub const CHUNK_MAX: u64 = 512 << 20;

/// The first line: `vnvm-tree	2	<average chunk size>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub avg: u64,
}

/// (the tests read a list's first line back with it; the readers take the lines after it: good_lines)
#[cfg(test)]
impl Header {
    pub fn parse(line: &str) -> Option<Header> {
        let mut f = line.split('\t');
        let (magic, version, avg) = (f.next()?, f.next()?, f.next()?);
        if magic != MAGIC || version.parse::<u32>().ok()? != VERSION || f.next().is_some() {
            return None;
        }
        Some(Header { avg: avg.parse().ok()? })
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{MAGIC}\t{VERSION}\t{}", self.avg)
    }
}

/// A chunk's name in the store: the first 16 bytes of the BLAKE3 of its raw bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkId(pub [u8; 16]);

impl ChunkId {
    /// From the hash of a chunk's bytes.
    pub fn of_hash(h: &[u8; 32]) -> ChunkId {
        let mut id = [0; 16];
        id.copy_from_slice(&h[..16]);
        ChunkId(id)
    }

    /// 32 hex digits (either case).
    pub fn parse(s: &str) -> Option<ChunkId> {
        let b = s.as_bytes();
        if b.len() != 32 {
            return None;
        }
        let mut id = [0; 16];
        for (o, pair) in id.iter_mut().zip(b.chunks_exact(2)) {
            let [hi, lo] = pair else { return None };
            *o = (hexval(*hi)? << 4) | hexval(*lo)?;
        }
        Some(ChunkId(id))
    }

    /// Where it is in the store: `chunks/<first two digits>/<hex>`.
    pub fn store_path(&self) -> String {
        let hex = self.to_string();
        format!("chunks/{}/{hex}", hex.get(..2).unwrap_or_default())
    }
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

/// A chunk's length: 1 to [`CHUNK_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkLen(NonZeroU64);

impl ChunkLen {
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl TryFrom<u64> for ChunkLen {
    type Error = Bad;
    fn try_from(n: u64) -> Result<ChunkLen, Bad> {
        NonZeroU64::new(n).filter(|n| n.get() <= CHUNK_MAX).map(ChunkLen).ok_or(Bad::ChunkLen)
    }
}

/// One chunk of a file: in the store, or all zeros (not stored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub id: Option<ChunkId>,
    pub len: ChunkLen,
}

/// A file's chunks, and where each starts (the last entry of `starts` is the file's size).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunks {
    list: Vec<Chunk>,
    starts: Vec<u64>,
}

impl Chunks {
    /// None when the file would be bigger than a u64.
    pub fn new(list: Vec<Chunk>) -> Option<Chunks> {
        let mut starts = Vec::with_capacity(list.len() + 1);
        starts.push(0u64);
        let mut at = 0u64;
        for c in &list {
            at = at.checked_add(c.len.get())?;
            starts.push(at);
        }
        Some(Chunks { list, starts })
    }

    pub fn list(&self) -> &[Chunk] {
        &self.list
    }

    /// Where each chunk starts, the file's size last.
    pub fn starts(&self) -> &[u64] {
        &self.starts
    }

    pub fn size(&self) -> u64 {
        self.starts.last().copied().unwrap_or(0)
    }

    /// The field: `<hash>:<length>,...`, and the size it must add up to.
    fn parse(field: &str, size: u64) -> Result<Chunks, Bad> {
        let mut list = Vec::new();
        for c in field.split(',').filter(|c| !c.is_empty()) {
            let (h, n) = c.split_once(':').ok_or(Bad::Chunk)?;
            let id = if h.is_empty() { None } else { Some(ChunkId::parse(h).ok_or(Bad::Chunk)?) };
            let len = ChunkLen::try_from(n.parse::<u64>().map_err(|_| Bad::ChunkLen)?)?;
            list.push(Chunk { id, len });
        }
        let c = Chunks::new(list).ok_or(Bad::Sum)?;
        if c.size() == size { Ok(c) } else { Err(Bad::Sum) }
    }
}

impl fmt::Display for Chunks {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, c) in self.list.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            if let Some(id) = c.id {
                write!(f, "{id}")?;
            }
            write!(f, ":{}", c.len.get())?;
        }
        Ok(())
    }
}

/// A file mode's permission bits (at most 0o7777), written in octal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode(u32);

impl Mode {
    pub const DIR: Mode = Mode(0o755);
    #[cfg(test)]
    pub const FILE: Mode = Mode(0o644);
    pub const LINK: Mode = Mode(0o777);

    /// The permission bits of `m` (the file type's left out).
    pub fn of(m: u32) -> Mode {
        Mode(m & 0o7777)
    }
}

impl From<Mode> for u32 {
    fn from(m: Mode) -> u32 {
        m.0
    }
}

/// Why a line is not one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// not `d`, `f` or `l`, or not that kind's number of fields
    Fields,
    Mode,
    Mtime,
    Size,
    /// a chunk not `<32 hex digits or nothing>:<length>`
    Chunk,
    /// a chunk of length 0 or more than CHUNK_MAX
    ChunkLen,
    /// the chunks' lengths not adding up to the size
    Sum,
}

/// What a line lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File(Chunks),
    Link(String),
}

/// One line after the header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub mode: Mode,
    pub mtime: i64,
    /// as itself (unescaped)
    pub path: String,
    pub kind: Kind,
}

impl Line {
    pub fn parse(l: &str) -> Result<Line, Bad> {
        let f: Vec<&str> = l.split('\t').collect();
        let (kind, mode, mtime, path, rest) = match f.as_slice() {
            [k, m, t, p, rest @ ..] => (*k, *m, *t, *p, rest),
            _ => return Err(Bad::Fields),
        };
        let mode = u32::from_str_radix(mode, 8).ok().filter(|&m| m <= 0o7777).map(Mode).ok_or(Bad::Mode)?;
        let mtime = mtime.parse::<i64>().map_err(|_| Bad::Mtime)?;
        let kind = match (kind, rest) {
            ("d", []) => Kind::Dir,
            ("f", [size, chunks]) => Kind::File(Chunks::parse(chunks, size.parse().map_err(|_| Bad::Size)?)?),
            ("l", [target]) => Kind::Link(unesc(target)),
            _ => return Err(Bad::Fields),
        };
        Ok(Line { mode, mtime, path: unesc(path), kind })
    }

    /// Its chunks, for a file.
    pub fn chunks(&self) -> Option<&Chunks> {
        match &self.kind {
            Kind::File(c) => Some(c),
            Kind::Dir | Kind::Link(_) => None,
        }
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let (mode, mtime, path) = (self.mode.0, self.mtime, esc(&self.path));
        match &self.kind {
            Kind::Dir => write!(f, "d\t{mode:o}\t{mtime}\t{path}"),
            Kind::File(c) => write!(f, "f\t{mode:o}\t{mtime}\t{path}\t{}\t{c}", c.size()),
            Kind::Link(t) => write!(f, "l\t{mode:o}\t{mtime}\t{path}\t{}", esc(t)),
        }
    }
}

/// A list's lines after its first (whatever that is: readers take a list of any header), with
/// what each is or why not; empty lines left out.
pub fn lines(text: &str) -> impl Iterator<Item = (&str, Result<Line, Bad>)> {
    text.lines().skip(1).filter(|l| !l.is_empty()).map(|l| (l, Line::parse(l)))
}

/// The lines that are ones (the others are left out, as every reader did).
pub fn good_lines(text: &str) -> impl Iterator<Item = Line> + '_ {
    lines(text).filter_map(|(_, l)| l.ok())
}

/// Every chunk id a list's file lines name, whether or not the line is whole (what a store
/// must keep: a damaged line's chunks are not taken for unused); in order, repeats kept.
pub fn listed_ids(text: &str) -> impl Iterator<Item = ChunkId> + '_ {
    text.lines()
        .skip(1)
        .filter(|l| l.starts_with("f\t"))
        .filter_map(|l| l.split('\t').nth(5))
        .flat_map(|c| c.split(','))
        .filter_map(|c| ChunkId::parse(c.split_once(':').map_or(c, |(h, _)| h)))
}

/// Escapes a path or link target: tab, newline and backslash.
pub fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

/// [`esc`] undone (a backslash before anything else is dropped; a last lone one too).
pub fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The readers before this file, kept as the oracle (differential): masu's treefiles.rs
    /// (its `unesc` was replace-chained: wrong for a backslash before `t` or `n`) and kura's
    /// vfs.rs `load_tree` (any hash not 32 digits taken for zeros, damaged modes as 0o644).
    mod old {
        pub fn masu_treefiles(l: &str) -> Option<(String, u64, Vec<String>, Vec<u64>)> {
            let unesc = |s: &str| s.replace("\\t", "\t").replace("\\n", "\n").replace("\\\\", "\\");
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() != 6 || f[0] != "f" {
                return None;
            }
            let size: u64 = f[4].parse().ok()?;
            let (mut hashes, mut starts, mut at) = (Vec::new(), vec![0u64], 0u64);
            for c in f[5].split(',').filter(|c| !c.is_empty()) {
                let (h, n) = c.split_once(':')?;
                if !(h.is_empty() || h.len() > 2 && h.bytes().all(|b| b.is_ascii_hexdigit())) {
                    return None;
                }
                at = at.checked_add(n.parse::<u64>().ok().filter(|&n| n > 0 && n <= super::CHUNK_MAX)?)?;
                hashes.push(h.to_string());
                starts.push(at);
            }
            (at == size).then_some((unesc(f[3]), size, hashes, starts))
        }

        /// (kind, mode, mtime, path, target or chunks)
        pub fn kura_load_tree(line: &str) -> Option<(char, u32, i64, String, Option<String>, Option<(Vec<Option<[u8; 16]>>, Vec<u64>)>)> {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 4 {
                return None;
            }
            let mode = u32::from_str_radix(f[1], 8).unwrap_or(0o644);
            let mtime = f[2].parse().unwrap_or(0);
            let path = super::unesc(f[3]);
            let hex16 = |s: &str| -> Option<[u8; 16]> {
                if s.len() != 32 {
                    return None;
                }
                let mut h = [0; 16];
                for i in 0..16 {
                    h[i] = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
                }
                Some(h)
            };
            match f[0] {
                "d" => Some(('d', mode, mtime, path, None, None)),
                "l" if f.len() > 4 => Some(('l', mode, mtime, path, Some(super::unesc(f[4])), None)),
                "f" if f.len() > 5 => {
                    let size: u64 = f[4].parse().unwrap_or(0);
                    let (mut hashes, mut starts, mut at) = (Vec::new(), vec![0u64], 0u64);
                    for c in f[5].split(',').filter(|c| !c.is_empty()) {
                        let (h, n) = c.split_once(':')?;
                        at += n.parse::<u64>().ok().filter(|&n| n > 0)?;
                        hashes.push(hex16(h));
                        starts.push(at);
                    }
                    (at == size).then(|| ('f', mode, mtime, path, None, Some((hashes, starts))))
                }
                _ => None,
            }
        }

        /// masu pack's writer (pack.rs before), for one line.
        pub fn pack_line(kind: u8, mode: u32, mtime: i64, path: &str, size: u64, hashes: &[String], target: &str) -> String {
            let esc = |s: &str| s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n");
            match kind {
                0 => format!("d\t{mode:o}\t{mtime}\t{}", esc(path)),
                1 => format!("f\t{mode:o}\t{mtime}\t{}\t{size}\t{}", esc(path), hashes.join(",")),
                _ => format!("l\t{mode:o}\t{mtime}\t{}\t{}", esc(path), esc(target)),
            }
        }
    }

    fn chunk() -> impl Strategy<Value = Chunk> {
        (proptest::option::weighted(0.8, any::<[u8; 16]>()), 1u64..=(1 << 20)).prop_map(|(id, n)| Chunk { id: id.map(ChunkId), len: ChunkLen::try_from(n).expect("in range") })
    }

    fn line() -> impl Strategy<Value = Line> {
        let kind = prop_oneof![Just(Kind::Dir), proptest::collection::vec(chunk(), 0..6).prop_map(|c| Kind::File(Chunks::new(c).expect("small"))), "\\PC{0,12}".prop_map(Kind::Link),];
        (0u32..=0o7777, any::<i64>(), "(\\PC|\\\\|\t|\n){0,16}", kind).prop_map(|(m, mtime, path, kind)| Line { mode: Mode(m), mtime, path, kind })
    }

    #[test]
    fn golden() {
        let h = "0123456789abcdef0123456789ABCDEF";
        let text = format!("vnvm-tree\t2\t4194304\nd\t755\t0\tdir\nf\t644\t7\tdir/a\\tb\\\\n\t15\t{h}:10,:5\nf\t600\t0\tempty\t0\t\nl\t777\t-1\tlink\tdir/a\n\nx\tjunk\n");
        assert_eq!(text.lines().next().and_then(Header::parse), Some(Header { avg: 4194304 }));
        let got: Vec<_> = lines(&text).map(|(_, l)| l).collect();
        let id = ChunkId::parse(h).expect("hex");
        assert_eq!(id.to_string(), h.to_lowercase());
        assert_eq!(id.store_path(), format!("chunks/01/{}", h.to_lowercase()));
        let len = |n| ChunkLen::try_from(n).expect("in range");
        assert_eq!(
            got,
            vec![
                Ok(Line { mode: Mode(0o755), mtime: 0, path: "dir".into(), kind: Kind::Dir }),
                Ok(Line { mode: Mode(0o644), mtime: 7, path: "dir/a\tb\\n".into(), kind: Kind::File(Chunks::new(vec![Chunk { id: Some(id), len: len(10) }, Chunk { id: None, len: len(5) }]).expect("small")) }),
                Ok(Line { mode: Mode(0o600), mtime: 0, path: "empty".into(), kind: Kind::File(Chunks::new(vec![]).expect("empty")) }),
                Ok(Line { mode: Mode(0o777), mtime: -1, path: "link".into(), kind: Kind::Link("dir/a".into()) }),
                Err(Bad::Fields),
            ]
        );
        assert_eq!(Header { avg: 65536 }.to_string(), "vnvm-tree\t2\t65536");
        for (l, why) in [
            ("f\t644\t0\ta\t3\tab:3", Bad::Chunk),
            ("f\t644\t0\ta\t3\t:4", Bad::Sum),
            ("f\t644\t0\ta\t0\t:0", Bad::ChunkLen),
            ("f\t644\t0\ta\tx\t", Bad::Size),
            ("f\t9\t0\ta\t0\t", Bad::Mode),
            ("f\t17777\t0\ta\t0\t", Bad::Mode),
            ("d\t755\tnow\ta", Bad::Mtime),
            ("d\t755\t0\ta\tb", Bad::Fields),
            ("l\t777\t0\ta", Bad::Fields),
            ("f\t644\t0\ta\t1", Bad::Fields),
            ("f\t644\t0\ta\t1\t:1\tx", Bad::Fields),
            ("f\t644\t0\ta\t1\tnocolon", Bad::Chunk),
            (&format!("f\t644\t0\ta\t{}\t:{}", CHUNK_MAX + 1, CHUNK_MAX + 1), Bad::ChunkLen),
            (&format!("f\t644\t0\ta\t0\t:{0},:{0}", u64::MAX), Bad::ChunkLen),
        ] {
            assert_eq!(Line::parse(l), Err(why), "{l}");
        }
        assert_eq!(Header::parse("vnvm-tree\t1\t262144"), None);
        assert_eq!(Header::parse("tree"), None);
    }

    proptest! {
        /// Lengths that add past u64: refused, not wrapped to the size.
        #[test]
        fn sums_never_wrap(n in 1u32..40) {
            let field: Vec<String> = (0..n).map(|_| format!(":{CHUNK_MAX}")).collect();
            let total = u128::from(CHUNK_MAX) * u128::from(n);
            let size = u64::try_from(total % (1u128 << 64)).expect("below 2^64");
            prop_assert_eq!(Chunks::parse(&field.join(","), size).is_ok(), total < 1u128 << 64);
        }

        /// Written and read again: the same line, and the same text when written again.
        #[test]
        fn round_trip(l in line()) {
            let text = l.to_string();
            prop_assert!(!text.contains('\n'));
            let back = Line::parse(&text);
            prop_assert_eq!(back.as_ref(), Ok(&l));
            prop_assert_eq!(back.map(|b| b.to_string()), Ok(text));
        }

        #[test]
        fn escaping(s in "(\\PC|\\\\|\t|\n|\\\\t|\\\\n){0,24}") {
            let e = esc(&s);
            prop_assert!(!e.contains('\t') && !e.contains('\n'));
            prop_assert_eq!(unesc(&e), s);
        }

        /// Anything: no panic, and what reads as a line writes as text that reads as it again.
        #[test]
        fn any_text(s in "((d|f|l|x|\t|:|,|[0-9]|[a-f]|\\\\|\\PC){0,40}|(d|f|l)\t[0-7]{1,5}\t-?[0-9]{1,3}\t[a\\\\tn]{0,4}(\t[0-9]{1,3}(\t(([0-9a-f]{32})?:[0-9]{1,3},?){0,3})?)?)") {
            if let Ok(l) = Line::parse(&s) {
                prop_assert_eq!(Line::parse(&l.to_string()), Ok(l));
            }
        }

        /// The writer as pack.rs had it, for what it wrote: the same text.
        #[test]
        fn writes_as_pack_did(l in line()) {
            let (kind, size, hashes, target) = match &l.kind {
                Kind::Dir => (0, 0, vec![], ""),
                Kind::File(c) => (1, c.size(), c.list().iter().map(|c| format!("{}:{}", c.id.map_or_default(|i| i.to_string()), c.len.get())).collect(), ""),
                Kind::Link(t) => (2, 0, vec![], t.as_str()),
            };
            prop_assert_eq!(l.to_string(), old::pack_line(kind, u32::from(l.mode), l.mtime, &l.path, size, &hashes, target));
        }

        /// The readers before, on any line pack writes: the same file, chunks and starts; the
        /// old masu reader differed only where its unescaping was wrong.
        #[test]
        fn reads_as_before(l in line()) {
            let text = l.to_string();
            let new = Line::parse(&text).expect("written");
            let k = old::kura_load_tree(&text).expect("kura read it");
            prop_assert_eq!((k.1, k.2, &k.3), (u32::from(new.mode), new.mtime, &new.path));
            match &new.kind {
                Kind::Dir => prop_assert_eq!((k.0, &k.4, &k.5), ('d', &None, &None)),
                Kind::Link(t) => prop_assert_eq!((k.0, k.4.as_ref()), ('l', Some(t))),
                Kind::File(c) => {
                    let ids: Vec<Option<[u8; 16]>> = c.list().iter().map(|c| c.id.map(|i| i.0)).collect();
                    prop_assert_eq!(k.0, 'f');
                    prop_assert_eq!(k.5, Some((ids, c.starts().to_vec())));
                    let m = old::masu_treefiles(&text).expect("masu read it");
                    let hex: Vec<String> = c.list().iter().map(|c| c.id.map_or_default(|i| i.to_string())).collect();
                    prop_assert_eq!((m.1, &m.2, &m.3[..]), (c.size(), &hex, c.starts()));
                    if !esc(&new.path).contains("\\\\") {
                        prop_assert_eq!(&m.0, &new.path);
                    }
                }
            }
        }
    }

    proptest! {
        /// A whole list's ids are what its lines' chunks name; a damaged line's too.
        #[test]
        fn listed_ids_of_lines(ls in proptest::collection::vec(line(), 0..8), junk in "\\PC{0,30}") {
            let text = format!("{}\n{}\n", Header { avg: 64 }, ls.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n"));
            let want: Vec<ChunkId> = ls.iter().filter_map(Line::chunks).flat_map(|c| c.list().iter().filter_map(|c| c.id)).collect();
            prop_assert_eq!(listed_ids(&text).collect::<Vec<_>>(), want.clone());
            let id = ChunkId([7; 16]);
            let damaged = format!("{text}f\t644\t0\tx\t1\t{id}:5,{junk}\n");
            let got: Vec<ChunkId> = listed_ids(&damaged).collect();
            prop_assert_eq!(got.get(..want.len()), Some(&want[..]));
            prop_assert_eq!(got.get(want.len()), Some(&id));
        }
    }

    /// The case the old masu reader got wrong: a file named `a\tb` (backslash, t) read as a
    /// tab, so `GET /tree/<name>/a%5Ctb` missed it.
    #[test]
    fn old_masu_unescaping_was_wrong() {
        let l = Line { mode: Mode::FILE, mtime: 0, path: "a\\tb".into(), kind: Kind::File(Chunks::new(vec![]).expect("empty")) };
        let text = l.to_string();
        assert_eq!(text, "f\t644\t0\ta\\\\tb\t0\t");
        assert_eq!(old::masu_treefiles(&text).map(|m| m.0), Some("a\\\tb".to_string()));
        assert_eq!(Line::parse(&text).map(|l| l.path), Ok("a\\tb".to_string()));
    }
}

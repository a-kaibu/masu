//! What one operation changed, kept while the namespace is shared (vfs/shared.rs) so that the
//! operation can be taken back whole when what it changed does not reach the shared buffer: the
//! process then answers ENOSPC, as a full disk does, and keeps exactly the namespace the others
//! have. Without it the process kept the change and answered success; a later snapshot from
//! another process replaced its namespace and the write was gone (review of 6852be1, §1).
//!
//! Each node is kept as it was before its first change of the operation, as little of it as the
//! change needs: the bytes a write covers and the length (a write into a big file costs what it
//! writes, as publishing does), the counts and times, or all of it. Once a node is kept whole
//! nothing more of it is kept; taking back sets the whole node, then reverses the byte changes
//! newest first, then sets the counts: each was kept before the next.
#![cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
use super::*;
use kome::num::us;

/// A node's counts, times and mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Meta {
    mode: u32,
    mtime: i64,
    nlink: u32,
    open_refs: u32,
}

impl Meta {
    fn of(n: &Node) -> Meta {
        Meta { mode: n.mode, mtime: n.mtime, nlink: n.nlink, open_refs: n.open_refs }
    }
    fn put(self, n: &mut Node) {
        (n.mode, n.mtime, n.nlink, n.open_refs) = (self.mode, self.mtime, self.nlink, self.open_refs);
    }
}

/// A file in memory before one change of its bytes: its length, and what was at `off`.
struct Edit {
    len: u64,
    off: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct Before {
    /// all of it (None: the slot was free)
    whole: Option<Option<Node>>,
    meta: Option<Meta>,
    /// oldest first
    edits: Vec<Edit>,
}

/// What an operation changed, from where it began.
pub(super) struct Undo {
    /// the count of node slots (the ones past it are new)
    len: usize,
    next_handle: u32,
    free: Option<Vec<Ino>>,
    nodes: BTreeMap<Ino, Before>,
    /// each handle changed, as it was (None: not open)
    handles: BTreeMap<u32, Option<Handle>>,
}

impl Undo {
    pub(super) fn begin(fs: &Fs) -> Undo {
        Undo { len: fs.nodes.len(), next_handle: fs.next_handle, free: None, nodes: BTreeMap::new(), handles: BTreeMap::new() }
    }

    /// What is kept of `ino` (None: a slot made by this operation; it goes whole).
    fn before(&mut self, ino: Ino) -> Option<&mut Before> {
        if us(ino) >= self.len {
            return None;
        }
        Some(self.nodes.entry(ino).or_default())
    }

    fn whole(&mut self, ino: Ino, n: impl FnOnce() -> Option<Node>) {
        if let Some(b) = self.before(ino) && b.whole.is_none() {
            b.whole = Some(n());
        }
    }

    fn meta(&mut self, ino: Ino, n: Option<&Node>) {
        if let Some(b) = self.before(ino) && b.whole.is_none() && b.meta.is_none() {
            b.meta = n.map(Meta::of);
        }
    }

    /// Before bytes `off..end` of `v` change, or its length.
    fn edit(&mut self, ino: Ino, v: &crate::pages::Pages, off: u64, end: u64) {
        let Some(b) = self.before(ino) else { return };
        if b.whole.is_some() {
            return;
        }
        let len = v.len();
        let (lo, hi) = (off.min(len), end.min(len));
        // (what a file in memory holds fits usize: the span of it too)
        let mut bytes = vec![0; usize::try_from(hi.saturating_sub(lo)).unwrap_or(0)];
        v.read(lo, &mut bytes);
        b.edits.push(Edit { len, off: lo, bytes });
    }

    fn free(&mut self, free: &[Ino]) {
        if self.free.is_none() {
            self.free = Some(free.to_vec());
        }
    }

    fn handle(&mut self, id: u32, h: Option<&Handle>) {
        self.handles.entry(id).or_insert_with(|| h.cloned());
    }
}

impl Fs {
    /// Before node `ino` changes in any way.
    pub(super) fn keep_whole(&mut self, ino: Ino) {
        if let Some(u) = &mut self.undo {
            let nodes = &self.nodes;
            u.whole(ino, || nodes.get(us(ino)).cloned().flatten());
        }
    }
    /// Node `ino` as it was, taken out of its slot (freed): kept without a copy.
    pub(super) fn keep_taken(&mut self, ino: Ino, n: Option<Node>) {
        if let Some(u) = &mut self.undo {
            u.whole(ino, || n);
        }
    }
    /// Before node `ino`'s counts, times or mode change.
    pub(super) fn keep_meta(&mut self, ino: Ino) {
        if let Some(u) = &mut self.undo {
            u.meta(ino, self.nodes.get(us(ino)).and_then(Option::as_ref));
        }
    }
    /// Before bytes `off..end` of file in memory `ino` change, or its length.
    pub(super) fn keep_bytes(&mut self, ino: Ino, off: u64, end: u64) {
        if let Some(u) = &mut self.undo && let Some(Some(Node { data: Data::File(Content::Upper(v)), .. })) = self.nodes.get(us(ino)) {
            u.edit(ino, v, off, end);
        }
    }
    /// Before the free slots change.
    pub(super) fn keep_free(&mut self) {
        if let Some(u) = &mut self.undo {
            u.free(&self.free);
        }
    }
    /// Before handle `id` opens or closes.
    pub(super) fn keep_handle(&mut self, id: u32) {
        if let Some(u) = &mut self.undo {
            u.handle(id, self.handles.get(&id));
        }
    }

    /// Takes back everything changed since `Undo::begin`, and forgets what was to be published.
    pub(super) fn undo(&mut self) {
        let Some(u) = self.undo.take() else { return };
        CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.nodes.truncate(u.len);
        for (ino, b) in u.nodes {
            let Some(slot) = self.nodes.get_mut(us(ino)) else { continue };
            if let Some(n) = b.whole {
                *slot = n;
            }
            if let Some(Node { data: Data::File(Content::Upper(v)), .. }) = slot {
                for e in b.edits.into_iter().rev() {
                    // (the file held these bytes and this length before: there is room again)
                    if v.write(e.off, &e.bytes).is_err() || v.set_len(e.len).is_err() {
                        eprintln!("kura: a file in memory could not be put back as it was (inode {ino})");
                    }
                }
            }
            if let (Some(m), Some(n)) = (b.meta, slot.as_mut()) {
                m.put(n);
            }
        }
        if let Some(free) = u.free {
            self.free = free;
        }
        // (the slots past the old count that were freed again)
        self.free.retain(|&i| us(i) < u.len);
        for (id, h) in u.handles {
            match h {
                Some(h) => self.handles.insert(id, h),
                None => self.handles.remove(&id),
            };
        }
        self.next_handle = u.next_handle;
        self.dirt.clear();
    }
}

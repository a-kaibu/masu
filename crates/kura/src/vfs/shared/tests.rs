use super::*;
use super::sim::{self, rooted, Mem, Op, Sim};
use proptest::prelude::*;

/// One process's operation as vfs::with runs it: take in the others' changes, do it, publish.
fn op<R>(fs: &mut Fs, m: &mut Mem, f: impl FnOnce(&mut Fs) -> R) -> R {
    fs.dirt.on = true;
    fs.sync_in(m).unwrap();
    let r = f(fs);
    assert!(fs.sync_out(m));
    r
}

#[test]
fn snapshots_preserve_remote_open_inodes_and_local_offsets() {
    let (mut parent, root) = rooted();
    parent.put_file("/file", b"first".to_vec(), 1);
    let ino = parent.walk(root, "file").unwrap(); let h = parent.new_handle(ino, 0); parent.handles.get_mut(&h).unwrap().pos = 3;
    let mut child = Fs::default(); child.apply_shared_snapshot(&parent.shared_snapshot()).unwrap(); child.unlink_name(root, "file");
    assert!(child.node(ino).is_some());
    child.put_file("/next", b"second".to_vec(), 2);
    assert_ne!(child.walk(root, "next"), Some(ino));
    parent.apply_shared_snapshot(&child.shared_snapshot()).unwrap(); assert_eq!(parent.handles[&h].pos, 3);
    let handle = parent.handles.remove(&h).unwrap(); parent.node_mut(handle.ino).unwrap().open_refs.release(handle.owner); parent.gc(handle.ino); assert!(parent.node(ino).is_none());
}

#[test]
fn opening_records_the_owner_without_sending_the_file() {
    let (mut fs, root) = rooted();
    fs.put_file("/save.dat", vec![5; 1000], 1);
    let ino = fs.walk(root, "save.dat").unwrap();
    let mut m = Mem::new(1 << 20);
    op(&mut fs, &mut m, |_| ());
    let before = m.written;
    let h = op(&mut fs, &mut m, |fs| fs.new_handle(ino, 0));
    assert_eq!(fs.node(ino).unwrap().open_refs.count(), 1);
    // (length, item count, kind, inode, owner count, owner PID, open handles)
    assert_eq!(m.written - before, 4 + 4 + 1 + 4 + 4 + 4 + 4);
    let mut other = Fs::default();
    other.sync_in(&m).unwrap();
    assert_eq!(other.node(ino).unwrap().open_refs.count(), 1);
    op(&mut fs, &mut m, |fs| fs.close_handle(h));
    other.sync_in(&m).unwrap();
    assert_eq!(other.node(ino).unwrap().open_refs.count(), 0);
}

// A child and its parent both have a handle to the original file. Dropping the child
// without its CloseHandle calls publishes only its references, even after a path is reused.
#[test]
fn an_exited_process_cannot_keep_unlinked_inodes_alive() {
    let (mut parent, root) = rooted();
    let mut child = Fs::default();
    let mut reader = Fs::default();
    let mut shared = Mem::new(1 << 20);
    let parent_pid = 0x10;
    let child_pid = 0x1_0010;
    op(&mut parent, &mut shared, |fs| {
        fs.put_file("/same", b"old".to_vec(), 0);
        fs.put_file("/child-only", b"delete".to_vec(), 0);
    });
    let held = parent.walk(root, "same").unwrap();
    let left = parent.walk(root, "child-only").unwrap();
    let parent_handle = op(&mut parent, &mut shared, |fs| fs.new_handle_for(held, 0, parent_pid));
    op(&mut child, &mut shared, |fs| {
        fs.new_handle_for(held, 0, child_pid);
        fs.new_handle_for(held, 0, child_pid);
        fs.new_handle_for(left, 0, child_pid);
    });
    reader.sync_in(&shared).unwrap();
    assert_eq!(reader.node(held).unwrap().open_refs.count(), 3);
    assert_eq!(reader.node(left).unwrap().open_refs.count(), 1);
    op(&mut parent, &mut shared, |fs| {
        fs.unlink_name(root, "same");
        fs.unlink_name(root, "child-only");
        fs.put_file("/same", b"replacement".to_vec(), 0);
    });
    let replacement = parent.walk(root, "same").unwrap();
    assert_ne!(held, replacement);
    drop(child); // No CloseHandle calls.

    op(&mut parent, &mut shared, |fs| fs.reap_open_refs(|pid| pid == child_pid));
    reader.sync_in(&shared).unwrap();
    assert_eq!(reader.node(held).unwrap().open_refs.count(), 1);
    assert_eq!(parent.handles[&parent_handle].ino, held);
    assert!(reader.node(left).is_none());
    assert_eq!(reader.walk(root, "same"), Some(replacement));
    op(&mut parent, &mut shared, |fs| fs.close_handle(parent_handle));
    reader.sync_in(&shared).unwrap();
    assert!(reader.node(held).is_none());
    assert_eq!(reader.file_size(replacement), 11);
}

#[test]
fn truncated_snapshots_do_not_mutate_the_receiver() {
    let (mut source, _) = rooted(); source.put_file("/data", vec![1,2,3], 4);
    let bytes = source.shared_snapshot(); let mut receiver = Fs::default();
    for n in 0..bytes.len() { assert!(receiver.apply_shared_snapshot(&bytes[..n]).is_none()); assert!(receiver.nodes.is_empty()); }
    receiver.apply_shared_snapshot(&bytes).unwrap(); assert_eq!(receiver.shared_snapshot(), bytes);
}

#[test]
fn cut_or_broken_entries_change_nothing() {
    let (mut a, root) = rooted();
    let mut m = Mem::new(1 << 20);
    op(&mut a, &mut m, |fs| fs.put_file("/f", vec![1; 10], 1));
    let ino = a.walk(root, "f").unwrap();
    let mut b = Fs::default();
    b.sync_in(&m).unwrap();
    a.dirt.on = true;
    a.write_upper(ino, b"xyz", 20).unwrap();
    a.mkdir_p(root, "d/e");
    let items = a.take_items();
    let before = b.shared_snapshot();
    for n in 0..items.len() {
        assert!(b.apply_items(&items[..n]).is_none(), "cut at {n}");
        assert_eq!(b.shared_snapshot(), before);
    }
    // (a directory naming a node that is not there)
    let mut bad = Vec::new();
    u32put(&mut bad, 1); bad.push(I_NODE); u32put(&mut bad, root);
    let mut dir = BTreeMap::new(); dir.insert("ghost".to_string(), 999);
    put_node(&mut bad, Some(&Node { data: Data::Dir(dir), mode: 0o755, mtime: 0, nlink: 2, open_refs: OpenRefs::default() }));
    assert!(b.apply_items(&bad).is_none());
    assert_eq!(b.shared_snapshot(), before);
    b.apply_items(&items).unwrap();
    assert_eq!(b.shared_snapshot(), a.shared_snapshot());
}

/// The case the review measured (P1-3): a 20k-file tree and a 64 MiB file written 64 KiB at a
/// time while another process shares the namespace. What goes through the buffer is about what
/// was written (once more for the snapshots that replace the journal), not the namespace per
/// write; an open and a close are a few bytes.
#[test]
fn writes_cost_what_they_write() {
    let (mut a, root) = rooted();
    for i in 0..20_000 { a.put_file(&format!("/data/{}/{i}.ks", i % 100), vec![], 0); }
    let mut m = Mem::new(1 << 30);
    op(&mut a, &mut m, |fs| fs.put_file("/save/big.bin", vec![], 1));
    let ino = a.walk(root, "save/big.bin").unwrap();
    let mut b = Fs::default();
    b.sync_in(&m).unwrap();
    let start = (m.written, m.read.get());
    let piece = vec![0xa5u8; 64 << 10];
    for k in 0..1024u64 {
        op(&mut a, &mut m, |fs| fs.write_upper(ino, &piece, k * piece.len() as u64).unwrap());
        // (the other process opens and closes something now and then, reading what is new)
        if k % 64 == 0 {
            let f = b.walk(root, "data/7/7.ks").unwrap();
            let before = m.written;
            let h = op(&mut b, &mut m, |fs| fs.new_handle(f, 0));
            op(&mut b, &mut m, |fs| fs.close_handle(h));
            assert!(m.written - before < 100, "an open and a close wrote {}", m.written - before);
        }
    }
    b.sync_in(&m).unwrap();
    assert_eq!(b.file_size(ino), 64 << 20);
    assert_eq!(b.shared_snapshot(), a.shared_snapshot());
    let (w, r) = (m.written - start.0, m.read.get() - start.1);
    // 64 MiB written: the journal, plus a snapshot each time it outgrows the last one
    assert!(w < 3 * (64 << 20) + (8 << 20), "{} MiB went into the buffer", w >> 20);
    assert!(r < 3 * (64 << 20) + (8 << 20), "{} MiB were read back", r >> 20);
}

/// One operation as vfs::change runs it, its publish allowed to fail: then taken back.
fn try_op<R>(fs: &mut Fs, m: &mut Mem, f: impl FnOnce(&mut Fs) -> R) -> (R, bool) {
    fs.dirt.on = true;
    fs.sync_in(m).unwrap();
    fs.undo = Some(super::super::undo::Undo::begin(fs));
    let r = f(fs);
    let ok = publish_or_undo(fs, m);
    (r, ok)
}

/// A change the buffer cannot take is not made: the process that made it has the namespace the
/// others have, its file as before (review of 6852be1 §1: the write answered success and stayed
/// here only).
#[test]
fn a_buffer_too_small_keeps_the_last_namespace() {
    let (mut a, root) = rooted();
    let mut m = Mem::new(400);
    op(&mut a, &mut m, |fs| fs.put_file("/x", vec![1; 10], 1));
    let generation = m.generation;
    let ino = a.walk(root, "x").unwrap();
    let before = a.shared_snapshot();
    let (_, ok) = try_op(&mut a, &mut m, |fs| fs.write_upper(ino, &[2; 1000], 0).unwrap());
    assert!(!ok);
    assert_eq!(m.generation, generation);
    assert_eq!(a.shared_snapshot(), before);
    assert_eq!(a.file_size(ino), 10);
    let mut b = Fs::default();
    b.sync_in(&m).unwrap();
    assert_eq!(b.shared_snapshot(), before);
}

/// The review's case: A's write of 1000 bytes does not fit; B then writes a new snapshot; A
/// takes it in. A was told ENOSPC, and no write it was told succeeded is lost: what it has is
/// what B published, A's own earlier writes in it.
#[test]
fn a_refused_write_is_not_lost_later() {
    let (mut a, root) = rooted();
    let mut m = Mem::new(400);
    op(&mut a, &mut m, |fs| fs.put_file("/x", vec![1; 10], 1));
    let ino = a.walk(root, "x").unwrap();
    op(&mut a, &mut m, |fs| fs.write_upper(ino, b"ok", 0).unwrap());
    let mut b = Fs::default();
    b.sync_in(&m).unwrap();
    let (_, ok) = try_op(&mut a, &mut m, |fs| fs.write_upper(ino, &[2; 1000], 0).unwrap());
    assert!(!ok);
    // (B's changes until one makes a new snapshot: the journal past the snapshot's size)
    let base = b.shared.base;
    for i in 0..20 {
        op(&mut b, &mut m, |fs| fs.write_upper(ino, &[i], 9).unwrap());
        if b.shared.base != base { break; }
    }
    assert_ne!(b.shared.base, base);
    a.sync_in(&m).unwrap();
    assert_eq!(a.shared_snapshot(), b.shared_snapshot());
    let mut got = [0u8; 10];
    let Some(Node { data: Data::File(Content::Upper(v)), .. }) = a.node(ino) else { panic!("not a file in memory") };
    v.read(0, &mut got);
    assert_eq!(&got[..9], b"ok\x01\x01\x01\x01\x01\x01\x01");
}

/// Every kind of change taken back: new nodes, freed ones, renamed names, bytes written over,
/// past the end and cut, counts, handles opened and closed; the process is as it was, bit for
/// bit, and its free slots are the empty ones.
#[test]
fn everything_an_operation_changed_is_taken_back() {
    let (mut a, root) = rooted();
    let mut m = Mem::new(1 << 20);
    op(&mut a, &mut m, |fs| {
        fs.put_file("/d/f", (0..=255u8).collect(), 1);
        fs.put_file("/d/g", vec![7; 30], 1);
        fs.put_file("/e/h", vec![9; 5], 1);
    });
    let (f, g) = (a.walk(root, "d/f").unwrap(), a.walk(root, "d/g").unwrap());
    let h0 = op(&mut a, &mut m, |fs| fs.new_handle(g, 0));
    let generation = m.generation;
    let snap = a.shared_snapshot();
    let (next, free, handles) = (a.next_handle, a.free.clone(), a.handles.keys().copied().collect::<Vec<_>>());
    // (a buffer that takes nothing more)
    m.full();
    let (_, ok) = try_op(&mut a, &mut m, |fs| {
        fs.write_upper(f, &[0xee; 40], 200).unwrap();
        fs.write_upper(f, &[0xdd; 3], 10).unwrap();
        fs.set_upper_size(f, 50).unwrap();
        fs.write_upper(f, &[0xcc; 4], 100).unwrap();
        let h = fs.new_handle(f, 0);
        fs.close_handle(h0);
        let e = fs.walk(root, "e").unwrap();
        fs.unlink_name(e, "h");
        fs.unlink_name(fs.walk(root, "d").unwrap(), "g");
        fs.put_file("/n/new", vec![1; 3], 5);
        fs.mkdir_p(root, "n/m/o");
        fs.close_handle(h);
    });
    assert!(!ok);
    assert_eq!(m.generation, generation);
    assert_eq!(a.shared_snapshot(), snap);
    assert_eq!((a.next_handle, a.free.clone()), (next, free));
    assert_eq!(a.handles.keys().copied().collect::<Vec<_>>(), handles);
    let empty: Vec<Ino> = (0..a.nodes.len() as Ino).filter(|&i| a.nodes[i as usize].is_none()).collect();
    let mut fr = a.free.clone(); fr.sort();
    assert_eq!(fr, empty);
    assert!(a.dirt.is_empty());
}

fn ops() -> impl Strategy<Value = Vec<(u8, Vec<Op>)>> {
    let data = || prop::collection::vec(any::<u8>(), 0..200);
    let op = prop_oneof![
        (0u8..12, data()).prop_map(|(n, d)| Op::Put(n, d)),
        (0u8..12, 0u16..600, data()).prop_map(|(n, o, d)| Op::Write(n, o, d)),
        (0u8..12, 0u16..900).prop_map(|(n, s)| Op::Size(n, s)),
        (0u8..12).prop_map(Op::Open),
        (0u8..8).prop_map(Op::Close),
        (0u8..12).prop_map(Op::Unlink),
        (0u8..4).prop_map(Op::Mkdir),
    ];
    // (one to three of them by one process before it publishes: what one publish carries)
    prop::collection::vec((0u8..3, prop::collection::vec(op, 1..4)), 1..60)
}
proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]
    /// Three processes doing anything to one namespace through the journal: after every
    /// operation each of them, having taken in what is new, has exactly the namespace the
    /// one that did it has (the same as a whole snapshot of it); handles stay each one's own.
    #[test]
    fn processes_agree(ops in ops(), limit in prop_oneof![Just(1usize << 24), 2000usize..20000]) {
        let mut sim = Sim::new(limit);
        for (who, batch) in ops {
            // (past the buffer: the batch is taken back, and the processes go on together)
            if let Err(e) = sim.step(who as usize, &batch) { prop_assert!(false, "{}", e); }
        }
    }

    /// The fuzz target's own decoding (fuzz/fuzz_targets/vfs_journal.rs) on random bytes.
    #[test]
    fn fuzz_target_on_any_bytes(b in prop::collection::vec(any::<u8>(), 0..600)) {
        sim::fuzz(&b);
    }

    /// Entries of any bytes are refused or applied whole, never half; they never panic.
    #[test]
    fn any_entry_bytes(b in prop::collection::vec(any::<u8>(), 0..300)) {
        let (mut fs, _) = rooted();
        fs.put_file("/a/b", vec![1; 20], 0);
        let before = fs.shared_snapshot();
        if fs.apply_items(&b).is_none() { prop_assert_eq!(fs.shared_snapshot(), before); }
    }

    /// A shared buffer of any bytes after a good head (another process's, or memory gone bad)
    /// is taken in or refused, never a panic (review: an assert and expects aborted the VM).
    #[test]
    fn any_buffer_bytes(snap in 0u32..400, b in prop::collection::vec(any::<u8>(), 0..400)) {
        let mut m = Mem::new(1 << 16);
        let mut head = b"VFS3".to_vec();
        head.extend_from_slice(&7u32.to_le_bytes());
        head.extend_from_slice(&snap.to_le_bytes());
        head.extend_from_slice(&b);
        prop_assert!(m.write(0, &head));
        m.commit(head.len());
        let mut fs = Fs::default();
        let _ = fs.sync_in(&m);
    }
}

/// Each way a shared buffer is bad is told apart, and none panics.
#[test]
fn bad_buffers_are_refused() {
    let buf = |b: &[u8]| { let mut m = Mem::new(1 << 16); assert!(m.write(0, b)); m.commit(b.len()); m };
    let head = |snap: u32| { let mut h = b"VFS3".to_vec(); h.extend_from_slice(&1u32.to_le_bytes()); h.extend_from_slice(&snap.to_le_bytes()); h };
    assert_eq!(Fs::default().sync_in(&buf(b"VFS")), Err(super::Corrupt::Head));
    assert_eq!(Fs::default().sync_in(&buf(b"VFS1\0\0\0\0\0\0\0\0")), Err(super::Corrupt::Head));
    assert_eq!(Fs::default().sync_in(&buf(&head(1000))), Err(super::Corrupt::Snapshot));
    // (a good snapshot, then an entry longer than what follows)
    let (fs, _) = rooted();
    let snap = fs.shared_snapshot();
    let mut b = head(snap.len() as u32);
    b.extend_from_slice(&snap);
    assert_eq!(Fs::default().sync_in(&buf(&b)), Ok(()));
    b.extend_from_slice(&50u32.to_le_bytes());
    b.extend_from_slice(&[0; 3]);
    assert_eq!(Fs::default().sync_in(&buf(&b)), Err(super::Corrupt::Entry));
}

/// Byte ranges merge and stay few.
#[test]
fn dirt_ranges_merge() {
    let mut d = Dirt { on: true, ..Default::default() };
    d.bytes(1, 10, 20);
    d.bytes(1, 30, 40);
    d.bytes(1, 20, 30);
    assert_eq!(d.nodes[&1].bytes, [(10, 40)]);
    d.bytes(1, 0, 5);
    d.bytes(1, 50, 60);
    assert_eq!(d.nodes[&1].bytes, [(0, 5), (10, 40), (50, 60)]);
    for i in 0..100 { d.bytes(2, i * 10, i * 10 + 1); }
    assert!(d.nodes[&2].bytes.len() <= MAX_RANGES);
    let covered: usize = d.nodes[&2].bytes.iter().map(|r| r.1 - r.0).sum();
    assert!(covered >= 100);
    for i in 0..100 { assert!(d.nodes[&2].bytes.iter().any(|r| r.0 <= i * 10 && i * 10 + 1 <= r.1)); }
    let mut off = Dirt::default();
    off.bytes(1, 0, 5);
    assert!(off.is_empty());
}

proptest! {
    /// Byte ranges against a bitmap of what was written (Kani proves the same for every input
    /// up to 8 bytes, vfs/shared.rs `proofs`): sorted and apart, at most MAX_RANGES, every
    /// written byte covered, and nothing else while they fit.
    #[test]
    fn ranges_against_a_bitmap(writes in prop::collection::vec((0usize..400, 0usize..60), 0..40)) {
        let mut r = Vec::new();
        let mut written = vec![false; 460];
        let mut max_seen = 0;
        for (lo, n) in writes {
            add_range(&mut r, lo, lo + n, MAX_RANGES);
            for b in lo..lo + n { written[b] = true; }
            max_seen = max_seen.max(r.len());
        }
        prop_assert!(r.len() <= MAX_RANGES);
        for w in r.windows(2) { prop_assert!(w[0].1 < w[1].0); }
        for x in &r { prop_assert!(x.0 < x.1); }
        let exact = max_seen < MAX_RANGES;
        for b in 0..460 {
            let covered = r.iter().any(|x| x.0 <= b && b < x.1);
            if written[b] { prop_assert!(covered, "byte {} written, not sent", b); }
            if exact && covered { prop_assert!(written[b], "byte {} sent, never written", b); }
        }
    }
}

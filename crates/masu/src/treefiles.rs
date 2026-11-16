//! The files of packed trees (`trees/<name>.tsv`, see pack.rs) as the server reads them for
//! itself (foresee.rs: a game's scripts and archives' indexes). Paths match without regard to
//! case (as on Windows). Web games' files are not served from here: the app's page reads them
//! from the device or the bucket (web/src/treeread.ts) for the frame's service worker.

use kome::locks::Lock as _;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::tree_list::{ChunkId, Chunks};

/// A file of a tree list: its chunks (cut by content: pack.rs, cdc.rs), checked when the list is
/// read (tree_list.rs: a damaged list's sums, hashes and lengths are not a file of it).
struct Entry(Chunks);

impl Entry {
    fn size(&self) -> u64 {
        self.0.size()
    }

    /// The chunk that holds byte `at` (one of the file's: `at` < its size).
    fn index(&self, at: u64) -> usize {
        self.0.starts().partition_point(|&x| x <= at).saturating_sub(1)
    }

    /// Where chunk `i` starts, and its length.
    fn span(&self, i: usize) -> Option<(u64, u64)> {
        let s = self.0.starts();
        Some((*s.get(i)?, s.get(i + 1)? - s.get(i)?))
    }
}

/// Decoded chunks kept for the next reads (a media element asking for small ranges one after
/// another decoded the same chunk each time: review b01), this many bytes in all.
const DECODED_MAX: usize = 64 << 20;

/// Decoded chunks by hash, newest last.
#[derive(Default)]
struct Decoded {
    map: HashMap<ChunkId, Arc<[u8]>>,
    order: std::collections::VecDeque<ChunkId>,
    bytes: usize,
}

impl Decoded {
    fn get(&mut self, k: ChunkId) -> Option<Arc<[u8]>> {
        let d = self.map.get(&k)?.clone();
        self.order.retain(|x| *x != k);
        self.order.push_back(k);
        Some(d)
    }

    fn put(&mut self, k: ChunkId, d: Arc<[u8]>) {
        if d.len() > DECODED_MAX / 4 {
            return;
        }
        if let Some(old) = self.map.insert(k, d.clone()) {
            self.bytes -= old.len();
            self.order.retain(|x| *x != k);
        }
        self.bytes += d.len();
        self.order.push_back(k);
        while self.bytes > DECODED_MAX {
            let Some(k) = self.order.pop_front() else { break };
            self.bytes -= self.map.remove(&k).map_or(0, |d| d.len());
        }
    }
}

/// Trees' lists kept parsed at most (the least lately used go).
const TREES_KEPT: usize = 64;

struct Tree {
    mtime: SystemTime,
    files: HashMap<String, Arc<Entry>>,
    /// when last used ([`tick`])
    used: std::sync::atomic::AtomicU64,
}

/// A count that goes up with each use of a tree list.
fn tick() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone)]
pub struct Trees {
    store: PathBuf,
    remote: Arc<Option<crate::storage::Storage>>,
    cache: Arc<Mutex<HashMap<String, Arc<Tree>>>>,
    decoded: Arc<Mutex<Decoded>>,
}

impl Trees {
    pub fn new(store: PathBuf) -> anyhow::Result<Self> {
        let remote = Arc::new(crate::storage::Storage::configured(crate::library::Library::new(&store)?)?);
        Ok(Trees { store, remote, cache: Default::default(), decoded: Default::default() })
    }

    fn tree(&self, name: &str) -> Option<Arc<Tree>> {
        let path = self.store.join("trees").join(format!("{name}.tsv"));
        let mtime = std::fs::metadata(&path).ok()?.modified().ok()?;
        if let Some(t) = self.cache.locked().get(name).filter(|t| t.mtime == mtime) {
            t.used.store(tick(), std::sync::atomic::Ordering::Relaxed);
            return Some(t.clone());
        }
        let text = std::fs::read_to_string(&path).ok()?;
        text.lines().next()?;
        let mut files = HashMap::new();
        for l in crate::tree_list::good_lines(&text) {
            match l.kind {
                crate::tree_list::Kind::File(c) => {
                    files.insert(l.path.to_lowercase(), Arc::new(Entry(c)));
                }
                crate::tree_list::Kind::Dir | crate::tree_list::Kind::Link(_) => {}
            }
        }
        let t = Arc::new(Tree { mtime, files, used: Default::default() });
        let mut c = self.cache.locked();
        // (bounded: the least lately used list goes; review b01)
        while c.len() >= TREES_KEPT {
            let Some(old) = c.iter().min_by_key(|(_, t)| t.used.load(std::sync::atomic::Ordering::Relaxed)).map(|(k, _)| k.clone()) else { break };
            c.remove(&old);
        }
        c.insert(name.to_string(), t.clone());
        Some(t)
    }
}

/// A stored chunk decoded, if it is `len` bytes: decoding stops past that (a broken or hostile
/// chunk does not get to fill memory).
fn decode(raw: &[u8], len: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(len.min(16 << 20));
    zstd::stream::Decoder::new(raw).ok()?.take(len as u64 + 1).read_to_end(&mut out).ok()?;
    (out.len() == len).then_some(out)
}

impl Trees {
    /// Chunk `i` of `e`, decoded; None when it cannot be had (or is not what the list says).
    async fn chunk(&self, e: &Entry, i: usize) -> Option<Arc<[u8]>> {
        let c = e.0.list().get(i)?;
        let len = usize::try_from(c.len.get()).ok()?;
        let Some(id) = c.id else { return Some(vec![0; len].into()) };
        if let Some(d) = self.decoded.locked().get(id).filter(|d| d.len() == len) {
            return Some(d);
        }
        let logical = id.store_path();
        let remote = match self.remote.as_ref() {
            Some(remote) => remote.get(&logical).await.ok()?,
            None => None,
        };
        let raw: Arc<[u8]> = match remote {
            Some(raw) => raw,
            None => tokio::fs::read(self.store.join(&logical)).await.ok()?.into(),
        };
        let d: Arc<[u8]> = tokio::task::spawn_blocking(move || decode(&raw, len)).await.ok()??.into();
        self.decoded.locked().put(id, d.clone());
        Some(d)
    }
}

/// The chunks of one file [`Trees::read`] keeps for the next reads of it (by index), and which
/// file that is: a read of another file starts it afresh (one cache handed to reads of two files
/// gave the second the first's chunks).
#[derive(Default)]
pub struct ReadCache {
    of: Option<(String, String)>,
    chunks: HashMap<usize, Arc<[u8]>>,
}

impl Trees {
    /// Tree `name`'s files: (path, lower case; size), in no order. None: no such tree.
    pub fn list(&self, name: &str) -> Option<Vec<(String, u64)>> {
        Some(self.tree(name)?.files.iter().map(|(p, e)| (p.clone(), e.size())).collect())
    }

    /// Tree `name`'s file at `path` (lower case): bytes `a..a + len` (fewer at its end), the
    /// chunks it spans kept in `cache` (index -> bytes) for the next reads of the same file.
    pub async fn read(&self, name: &str, path: &str, a: u64, len: u64, cache: &mut ReadCache) -> Option<Vec<u8>> {
        let e = self.tree(name)?.files.get(path)?.clone();
        if a >= e.size() || len == 0 {
            return Some(Vec::new());
        }
        let b = (a + len).min(e.size()) - 1;
        if cache.of.as_ref().is_none_or(|(n, p)| n != name || p != path) {
            *cache = ReadCache { of: Some((name.to_string(), path.to_string())), chunks: HashMap::new() };
        }
        let mut out = Vec::with_capacity((b + 1 - a) as usize);
        for i in e.index(a)..=e.index(b) {
            if !cache.chunks.contains_key(&i) {
                // (a few kept: the members of one archive lie side by side)
                if let Some(k) = cache.chunks.keys().next().copied().filter(|_| cache.chunks.len() >= 8) {
                    cache.chunks.remove(&k);
                }
                cache.chunks.insert(i, self.chunk(&e, i).await?);
            }
            let d = cache.chunks.get(&i)?;
            let start = e.span(i)?.0;
            let from = a.max(start) - start;
            let to = (b + 1).min(start + d.len() as u64) - start;
            out.extend_from_slice(&d[from as usize..to as usize]);
        }
        Some(out)
    }

    /// When tree `name`'s list last changed.
    pub fn changed(&self, name: &str) -> Option<SystemTime> {
        std::fs::metadata(self.store.join("trees").join(format!("{name}.tsv"))).ok()?.modified().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store with tree `t`: a.bin (10 bytes in chunks of 4, the middle one all zero) and
    /// empty.txt.
    fn store() -> (PathBuf, Trees) {
        let dir = std::env::temp_dir().join(format!("masu-treefiles-{}", uuid::Uuid::new_v4()));
        let put = |data: &[u8]| {
            let h = blake3::hash(data).to_hex()[..32].to_string();
            let d = dir.join("chunks").join(&h[..2]);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(&h), zstd::encode_all(data, 3).unwrap()).unwrap();
            h
        };
        let (c0, c2) = (put(b"0123"), put(b"89"));
        std::fs::create_dir_all(dir.join("trees")).unwrap();
        std::fs::write(
            dir.join("trees/t.tsv"),
            format!("{}\nf\t644\t0\tA.bin\t10\t{c0}:4,:4,{c2}:2\nf\t644\t0\tempty.txt\t0\t\n", crate::tree_list::Header { avg: 4 }),
        )
        .unwrap();
        (dir.clone(), Trees { store: dir, remote: Arc::new(None), cache: Default::default(), decoded: Default::default() })
    }

    async fn read(t: &Trees, name: &str, path: &str, a: u64, len: u64) -> Option<Vec<u8>> {
        t.read(name, path, a, len, &mut ReadCache::default()).await
    }

    #[tokio::test]
    async fn whole_files_and_parts() {
        let (dir, t) = store();
        assert_eq!(read(&t, "t", "a.bin", 0, 10).await.as_deref(), Some(&b"0123\0\0\0\089"[..]));
        for (a, len, want) in [(2, 7, &b"23\0\0\0\08"[..]), (8, 100, b"89"), (3, 100, b"3\0\0\0\089"), (4, 1, b"\0"), (10, 5, b""), (0, 0, b"")] {
            assert_eq!(read(&t, "t", "a.bin", a, len).await.as_deref(), Some(want), "{a}+{len}");
        }
        assert_eq!(read(&t, "t", "empty.txt", 0, 10).await.as_deref(), Some(&b""[..]));
        assert_eq!(read(&t, "t", "none", 0, 10).await, None);
        assert_eq!(read(&t, "none", "a.bin", 0, 10).await, None);
        assert_eq!(t.list("t").map(|mut l| { l.sort(); l }), Some(vec![("a.bin".into(), 10), ("empty.txt".into(), 0)]));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review b01: a chunk decoded once serves the next reads of it without being read again
    /// (here its stored file is gone after the first).
    #[tokio::test]
    async fn a_chunk_is_decoded_once() {
        let (dir, t) = store();
        assert_eq!(read(&t, "t", "a.bin", 0, 2).await.as_deref(), Some(&b"01"[..]));
        std::fs::remove_dir_all(dir.join("chunks")).unwrap();
        for (a, len, want) in [(1, 2, &b"12"[..]), (3, 1, b"3"), (0, 4, b"0123")] {
            assert_eq!(read(&t, "t", "a.bin", a, len).await.as_deref(), Some(want), "{a}+{len}");
        }
        // (the chunk not kept: gone)
        assert_eq!(read(&t, "t", "a.bin", 8, 2).await, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn id(i: u32) -> ChunkId {
        let mut b = [0; 16];
        b[..4].copy_from_slice(&i.to_le_bytes());
        ChunkId(b)
    }

    #[test]
    fn decoded_bounded() {
        let mut d = Decoded::default();
        for i in 0..100 {
            d.put(id(i), vec![i as u8; 4 << 20].into());
            assert!(d.bytes <= DECODED_MAX);
            assert_eq!(d.bytes, d.map.values().map(|v| v.len()).sum::<usize>());
            assert_eq!(d.order.len(), d.map.len());
        }
        assert!(d.get(id(99)).is_some() && d.get(id(0)).is_none());
        d.put(id(1000), vec![0u8; DECODED_MAX].into());
        assert!(d.get(id(1000)).is_none(), "too big to keep");
    }

    /// Random files of many chunks of any length (some all zero), random parts, one ReadCache
    /// across them all: what comes out is the file's bytes, in order. A chunk that decodes to more than its length is an error,
    /// found without decoding it all.
    #[tokio::test]
    async fn many_chunks_in_order() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("masu-treefiles-{}", uuid::Uuid::new_v4()));
        let mut seed = 0x853c49e6748fea9bu64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut lines = String::from("vnvm-tree\t2\t64\n");
        let mut files = Vec::new();
        for f in 0..6 {
            let size = 1 + rnd(40 * 64);
            let data: Vec<u8> = (0..size).map(|_| rnd(256) as u8).collect();
            let (mut hashes, mut want, mut at, mut i) = (Vec::new(), Vec::new(), 0usize, 0);
            while at < data.len() {
                let c = &data[at..(at + 1 + rnd(130) as usize).min(data.len())];
                at += c.len();
                i += 1;
                if i % 5 == 3 {
                    hashes.push(format!(":{}", c.len()));
                    want.extend(std::iter::repeat_n(0u8, c.len()));
                    continue;
                }
                let h = blake3::hash(c).to_hex()[..32].to_string();
                std::fs::create_dir_all(dir.join("chunks").join(&h[..2])).unwrap();
                std::fs::write(dir.join("chunks").join(&h[..2]).join(&h), zstd::encode_all(c, 3).unwrap()).unwrap();
                hashes.push(format!("{h}:{}", c.len()));
                want.extend_from_slice(c);
            }
            lines += &format!("f\t644\t0\tf{f}\t{size}\t{}\n", hashes.join(","));
            files.push(want);
        }
        // a chunk that decodes to 64 MiB, listed as 64 bytes (level 1: level 19 exhausted
        // mutants' 3 GB address-space cap when other tests compressed concurrently)
        let bomb = zstd::encode_all(std::io::repeat(0).take(64 << 20), 1).unwrap();
        std::fs::create_dir_all(dir.join("chunks/bb")).unwrap();
        std::fs::write(dir.join("chunks/bb/bb000000000000000000000000000000"), bomb).unwrap();
        lines += "f\t644\t0\tbomb\t128\tbb000000000000000000000000000000:64,bb000000000000000000000000000000:64\n";
        std::fs::create_dir_all(dir.join("trees")).unwrap();
        std::fs::write(dir.join("trees/m.tsv"), lines).unwrap();
        let t = Trees { store: dir.clone(), remote: Arc::new(None), cache: Default::default(), decoded: Default::default() };
        let mut cache = ReadCache::default();
        for (f, want) in files.iter().enumerate() {
            assert_eq!(read(&t, "m", &format!("f{f}"), 0, want.len() as u64).await.as_ref(), Some(want));
            for _ in 0..20 {
                let a = rnd(want.len() as u64);
                let z = a + rnd(want.len() as u64 - a);
                let got = t.read("m", &format!("f{f}"), a, z + 1 - a, &mut cache).await;
                assert_eq!(got.as_deref(), Some(&want[a as usize..=z as usize]), "f{f} {a}-{z}");
            }
        }
        assert_eq!(read(&t, "m", "bomb", 0, 128).await, None);
        assert_eq!(decode(&zstd::encode_all(&b"abcd"[..], 3).unwrap(), 4).as_deref(), Some(&b"abcd"[..]));
        assert_eq!(decode(&zstd::encode_all(&b"abcde"[..], 3).unwrap(), 4), None);
        assert_eq!(decode(&zstd::encode_all(&b"abc"[..], 3).unwrap(), 4), None);
        assert_eq!(decode(b"not zstd", 4), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn tree_listing_rules() {
        let (dir, t) = store();
        let h = {
            let h = "abcd0000000000000000000000000000";
            let d = dir.join("chunks/ab");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(h), zstd::encode_all(&b"zz"[..], 3).unwrap()).unwrap();
            h
        };
        // other kinds of lines, lines with other field counts and lists whose lengths do not
        // add up are not files; a short chunk name is not one either
        std::fs::write(
            dir.join("trees/a_b-c.d.tsv"),
            format!("vnvm-tree\t2\t4\nd\t755\t0\tdir\t2\t{h}:2\nf\t644\t0\tx\t2\t{h}:2\textra\nf\t644\t0\tok\t2\t{h}:2\nf\t644\t0\tshort\t2\ta:2\nf\t644\t0\tnochunks\t2\t:2\nf\t644\t0\twrong\t3\t{h}:2\nf\t644\t0\tnolength\t2\t{h}\n"),
        )
        .unwrap();
        assert_eq!(read(&t, "a_b-c.d", "ok", 0, 9).await.as_deref(), Some(&b"zz"[..]));
        for none in ["dir", "x", "wrong", "nolength", "short"] {
            assert_eq!(read(&t, "a_b-c.d", none, 0, 9).await, None, "{none}");
        }
        // a chunk with no name is all zero, also when it is the only one
        assert_eq!(read(&t, "a_b-c.d", "nochunks", 0, 9).await.as_deref(), Some(&b"\0\0"[..]));
        // a new listing (a new mtime) is read again
        let path = dir.join("trees/a_b-c.d.tsv");
        std::fs::write(&path, format!("vnvm-tree\t2\t4\nf\t644\t0\tnew\t2\t{h}:2\n")).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        assert_eq!(read(&t, "a_b-c.d", "new", 0, 9).await.as_deref(), Some(&b"zz"[..]));
        assert_eq!(read(&t, "a_b-c.d", "ok", 0, 9).await, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}

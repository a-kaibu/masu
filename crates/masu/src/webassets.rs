//! The web app (web/dist) built into the binary (rust-embed; a debug build reads web/dist from
//! the checkout instead). The server serves files from a directory (and writes `.zst` beside
//! them), so the first start of a build unpacks them into `<cache>/web/<hash>`. The benchmark
//! pages (bench.html, game.html, benchwasm/) are left out.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
// the benchmark pages are for development (serve them with --web web/dist)
#[exclude = "bench.html"]
#[exclude = "game.html"]
#[exclude = "benchwasm/*"]
#[exclude = "*.zst"]
struct Web;

pub fn unpack(cache: &Path) -> Result<(PathBuf, crate::lockfile::LockFile)> {
    let mut names: Vec<_> = Web::iter().collect();
    if names.is_empty() {
        bail!("this masu was built without the web app (build web/dist before masu, or give --web)");
    }
    names.sort();
    let mut h = blake3::Hasher::new();
    for n in &names {
        h.update(n.as_bytes());
        h.update(&Web::get(n).with_context(|| format!("missing web asset {n}"))?.metadata.sha256_hash());
    }
    let key = h.finalize().to_hex()[..16].to_string();
    let root = cache.join("web");
    std::fs::create_dir_all(&root)?;
    // Serialize publication, acquisition of the lifetime lock, and pruning across processes.
    let cache_lock = crate::lockfile::LockFile::open(&root.join(".lock"))?;
    cache_lock.lock()?;
    let dir = root.join(&key);
    if !dir.join(".done").exists() {
        let tmp = root.join(format!(".{key}.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for n in &names {
            let p = tmp.join(n.as_ref());
            std::fs::create_dir_all(p.parent().context("an asset with no folder")?)?;
            std::fs::write(&p, Web::get(n).with_context(|| format!("missing web asset {n}"))?.data).with_context(|| p.display().to_string())?;
        }
        std::fs::write(tmp.join(".done"), "")?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir).with_context(|| dir.display().to_string())?;
    }
    // this server uses it while it runs: older builds' servers still running keep theirs
    let inuse = crate::lockfile::LockFile::open(&dir.join(".done"))?;
    inuse.lock_shared()?;
    prune(&root, &key)?;
    Ok((dir, inuse))
}

/// Removes the copies of other builds that no running server uses (one that does holds a shared
/// lock on its `.done`).
fn prune(root: &Path, keep: &str) -> Result<()> {
    for e in std::fs::read_dir(root)?.flatten() {
        let n = e.file_name();
        if n == keep || n.to_string_lossy().starts_with('.') {
            continue;
        }
        match crate::lockfile::LockFile::existing(&e.path().join(".done")) {
            Ok(f) => {
                if f.try_lock().is_ok() {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
            // (one never finished)
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let _ = std::fs::remove_dir_all(e.path());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_copies_running_servers_use() {
        let root = std::env::temp_dir().join(format!("masu-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["new", "used", "unused", "unfinished"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            if d != "unfinished" {
                std::fs::write(root.join(d).join(".done"), "").unwrap();
            }
        }
        // (another server of an older build)
        let other = crate::lockfile::LockFile::open(&root.join("used/.done")).unwrap();
        other.lock_shared().unwrap();
        super::prune(&root, "new").unwrap();
        assert!(root.join("new").exists());
        assert!(root.join("used").exists(), "removed under a running server");
        assert!(!root.join("unused").exists());
        assert!(!root.join("unfinished").exists());
        drop(other);
        super::prune(&root, "new").unwrap();
        assert!(!root.join("used").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

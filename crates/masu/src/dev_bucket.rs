//! `masu serve --dev-bucket <dir>`: a stand-in for the S3 bucket on this machine, for the tests
//! and for work without Hugging Face. The server reads games only from a bucket and the page
//! fetches their chunks only from it (docs/s3-reads.md), so a run without the real one needs
//! this one: objects are files under `<dir>/objects`, path-style (`/<bucket>/<key>`), any
//! signature is taken, and every origin may read them (the page fetches presigned URLs itself).
//! `<dir>/storage.toml` is written for it (its key kept in `<dir>/key`, so a second run opens
//! what the first put there); the server then uses it as it would MASU_STORAGE_CONFIG.

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{Path as UrlPath, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use std::path::{Path, PathBuf};

/// The bucket's name in its URLs (any: it serves one).
pub const BUCKET: &str = "dev";

/// An object's key: one or more plain parts (no `..`, no empty part), as the server names them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key(String);

impl Key {
    fn parse(s: &str) -> Option<Key> {
        let ok = !s.is_empty() && s.split('/').all(|p| !p.is_empty() && p != "." && p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
        ok.then(|| Key(s.to_string()))
    }
    fn file(&self, root: &Path) -> PathBuf {
        self.0.split('/').fold(root.to_path_buf(), |p, part| p.join(part))
    }
}

#[derive(Clone)]
struct Objects(PathBuf);

/// `bytes=a-b` or `bytes=a-` of `len` bytes: (a, b inclusive); None: not one this takes (the
/// whole object goes); Some(None): past the end.
fn range(h: &str, len: u64) -> Option<Option<(u64, u64)>> {
    let (a, b) = h.trim().strip_prefix("bytes=")?.split_once('-')?;
    let a: u64 = a.parse().ok()?;
    let b: u64 = if b.is_empty() { u64::MAX } else { b.parse().ok()? };
    if b < a {
        return None;
    }
    Some((a < len).then(|| (a, b.min(len.saturating_sub(1)))))
}

fn key_of(path: &str) -> Option<Key> {
    let rest = path.strip_prefix(BUCKET)?.strip_prefix('/')?;
    Key::parse(rest)
}

async fn object(State(o): State<Objects>, method: Method, UrlPath(path): UrlPath<String>, headers: HeaderMap, body: Bytes) -> Response {
    if method == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    let Some(key) = key_of(&path) else { return StatusCode::NOT_FOUND.into_response() };
    let file = key.file(&o.0);
    match method {
        Method::PUT => {
            let write = async {
                tokio::fs::create_dir_all(file.parent().context("an object with no folder")?).await?;
                let temp = file.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
                tokio::fs::write(&temp, &body).await?;
                tokio::fs::rename(&temp, &file).await?;
                anyhow::Ok(())
            };
            match write.await {
                Ok(()) => StatusCode::OK.into_response(),
                Err(e) => {
                    eprintln!("dev bucket: {} not written: {e:#}", key.0);
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                }
            }
        }
        Method::DELETE => {
            let _ = tokio::fs::remove_file(&file).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Method::GET | Method::HEAD => {
            let Ok(data) = tokio::fs::read(&file).await else { return StatusCode::NOT_FOUND.into_response() };
            let len = data.len() as u64;
            let want = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|h| range(h, len));
            let (status, body, content_range) = match want {
                None => (StatusCode::OK, Bytes::from(data), None),
                Some(None) => (StatusCode::RANGE_NOT_SATISFIABLE, Bytes::new(), Some(format!("bytes */{len}"))),
                Some(Some((a, b))) => (StatusCode::PARTIAL_CONTENT, Bytes::from(data).slice(a as usize..b as usize + 1), Some(format!("bytes {a}-{b}/{len}"))),
            };
            let mut r = if method == Method::HEAD { status.into_response() } else { (status, body).into_response() };
            if let Some(c) = content_range.and_then(|c| HeaderValue::from_str(&c).ok()) {
                r.headers_mut().insert(header::CONTENT_RANGE, c);
            }
            r
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

/// Every origin may read (and ask for ranges), as the real bucket's CDN lets it.
async fn cors(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, HEAD"));
    h.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("range"));
    h.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, HeaderValue::from_static("content-range, content-length"));
    r
}

fn router(objects: PathBuf) -> Router {
    Router::new()
        .route("/{*path}", axum::routing::any(object))
        .layer(axum::extract::DefaultBodyLimit::max(1 << 30))
        .layer(axum::middleware::from_fn(cors))
        .with_state(Objects(objects))
}

/// The bucket's key in `dir` (made the first time): 64 hex digits.
fn key_hex(dir: &Path) -> Result<String> {
    let at = dir.join("key");
    if let Ok(k) = std::fs::read_to_string(&at) {
        let k = k.trim().to_string();
        anyhow::ensure!(k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()), "{}: not a key", at.display());
        return Ok(k);
    }
    let k: String = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()].iter().flat_map(|u| *u.as_bytes()).map(|b| format!("{b:02x}")).collect();
    std::fs::write(&at, &k)?;
    Ok(k)
}

/// The storage configuration for a bucket at `endpoint` with the key in `dir`.
fn config(dir: &Path, endpoint: &str) -> Result<String> {
    let cache = dir.join("cache");
    let cache = cache.to_str().context("the dev bucket's folder is not UTF-8")?;
    Ok(format!(
        "bucket = {BUCKET:?}\nregion = \"us-east-1\"\nendpoint = {endpoint:?}\nprefix = \"\"\nactive_key = \"dev\"\ncache = {cache:?}\ncache_gib = 1\n[keys]\ndev = {:?}\n",
        key_hex(dir)?
    ))
}

/// Starts the bucket for `dir` on a port of its own, on a thread of its own (the server's
/// runtime starts after), and points [`crate::storage::Storage::configured`] at it.
pub fn start(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("objects"))?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let path = dir.join("storage.toml");
    std::fs::write(&path, config(dir, &endpoint)?)?;
    let app = router(dir.join("objects"));
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let listener = rt.block_on(async { tokio::net::TcpListener::from_std(listener) })?;
    std::thread::spawn(move || {
        if let Err(e) = rt.block_on(async { axum::serve(listener, app).await }) {
            eprintln!("dev bucket: {e}");
        }
    });
    crate::storage::use_dev_config(path);
    eprintln!("masu: a stand-in bucket at {endpoint} (objects in {})", dir.join("objects").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str, range: Option<&str>, body: &[u8]) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut r = Request::builder().method(method).uri(uri);
        if let Some(v) = range {
            r = r.header(header::RANGE, v);
        }
        let res = app.clone().oneshot(r.body(Body::from(body.to_vec())).unwrap()).await.unwrap();
        let (p, b) = res.into_parts();
        (p.status, p.headers, to_bytes(b, usize::MAX).await.unwrap().to_vec())
    }

    /// What the server and the page ask of a bucket: put, get whole and in ranges (a delivery
    /// pack's runs), delete; open to every origin; no way out of its folder.
    #[tokio::test]
    async fn serves_objects_as_a_bucket_does() {
        let dir = std::env::temp_dir().join(format!("masu-dev-bucket-{}", uuid::Uuid::new_v4()));
        let app = router(dir.clone());
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7) as u8).collect();
        assert_eq!(call(&app, "PUT", "/dev/p/obj-1", None, &data).await.0, StatusCode::OK);
        let (st, h, got) = call(&app, "GET", "/dev/p/obj-1?X-Amz-Signature=any", None, b"").await;
        assert_eq!((st, got), (StatusCode::OK, data.clone()));
        assert_eq!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(), "*");
        assert!(h.get(header::ACCESS_CONTROL_EXPOSE_HEADERS).unwrap().to_str().unwrap().contains("content-range"));
        for (want, a, b) in [("bytes=0-9", 0, 9), ("bytes=990-", 990, 999), ("bytes=995-5000", 995, 999)] {
            let (st, h, got) = call(&app, "GET", "/dev/p/obj-1", Some(want), b"").await;
            assert_eq!(st, StatusCode::PARTIAL_CONTENT);
            assert_eq!(h.get(header::CONTENT_RANGE).unwrap().to_str().unwrap(), format!("bytes {a}-{b}/1000"));
            assert_eq!(got, data[a..=b]);
        }
        assert_eq!(call(&app, "GET", "/dev/p/obj-1", Some("bytes=1000-"), b"").await.0, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(call(&app, "OPTIONS", "/dev/p/obj-1", None, b"").await.0, StatusCode::NO_CONTENT);
        assert_eq!(call(&app, "DELETE", "/dev/p/obj-1", None, b"").await.0, StatusCode::NO_CONTENT);
        assert_eq!(call(&app, "GET", "/dev/p/obj-1", None, b"").await.0, StatusCode::NOT_FOUND);
        for bad in ["/dev/../x", "/dev/a/../../x", "/other/x", "/dev/", "/dev/a//b", "/dev/a%2F..%2Fb"] {
            assert_eq!(call(&app, "PUT", bad, None, b"x").await.0, StatusCode::NOT_FOUND, "{bad}");
        }
        assert!(!dir.parent().unwrap().join("x").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Any key part this does not take is refused whatever it holds (property: only plain
    /// parts pass, and what passes stays under the root).
    #[test]
    fn keys_stay_in_the_folder() {
        let root = Path::new("/r");
        let parts = ["a", "..", ".", "", "b.c", "x-y_z", "a b", "\\", "%2e", "ok"];
        for a in parts {
            for b in parts {
                for c in [None, Some("q")] {
                    let s = match c { Some(c) => format!("{a}/{b}/{c}"), None => format!("{a}/{b}") };
                    if let Some(k) = Key::parse(&s) {
                        let f = k.file(root);
                        assert!(f.starts_with(root) && f.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_))), "{s}");
                    } else {
                        assert!(s.split('/').any(|p| p.is_empty() || p == "." || p == ".." || p.contains([' ', '\\', '%'])), "{s} refused");
                    }
                }
            }
        }
    }

    /// The configuration opens a real Storage against the running bucket: a put comes back
    /// through a presigned URL as the page would fetch it.
    #[tokio::test]
    async fn the_server_reads_and_presigns_through_it() {
        let dir = std::env::temp_dir().join(format!("masu-dev-bucket-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("objects")).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = router(dir.join("objects"));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        std::fs::write(dir.join("storage.toml"), config(&dir, &endpoint).unwrap()).unwrap();
        let key = key_hex(&dir).unwrap();
        assert_eq!(key_hex(&dir).unwrap(), key, "the key is kept");
        let db = crate::library::Library::new(&dir.join("store")).unwrap();
        let s = crate::storage::Storage::open(db, &dir.join("storage.toml"), s3::creds::Credentials::new(Some("dev"), Some("dev"), None, None, None).unwrap()).unwrap();
        s.put("chunks/ab/00000000000000000000000000000001", b"hello").await.unwrap();
        assert_eq!(&*s.get("chunks/ab/00000000000000000000000000000001").await.unwrap().unwrap(), b"hello");
        let d = s.direct("chunks/ab/00000000000000000000000000000001", 60).await.unwrap().unwrap();
        assert!(d.url.starts_with(&endpoint), "{}", d.url);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

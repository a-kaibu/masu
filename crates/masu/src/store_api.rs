//! The store's routes: the catalog (`/store/trees/`, `/store/games/`, from the bucket) and how
//! the page fetches chunks from the bucket itself (`/store-direct`, `/sake-direct`). Chunks
//! never come through this server (docs/s3-reads.md).

use kome::locks::Rw as _;
use crate::storage::Storage;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
struct Store {
    remote: Arc<Storage>,
    /// sake's wasm files put in the bucket ([`publish_wasm`]): file name -> chunk name
    wasm: Arc<std::sync::RwLock<std::collections::HashMap<String, String>>>,
}

/// What `/store/<path>` serves: the catalog (a tree's list, a game's description or cover).
/// A chunk is not one: the page fetches it from the bucket (`/store-direct`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Catalog {
    Tree,
    Game,
}

impl Catalog {
    fn of(path: &str) -> Option<Catalog> {
        let p = std::path::Path::new(path);
        if !p.components().all(|c| matches!(c, std::path::Component::Normal(_))) {
            return None;
        }
        match path.split('/').next()? {
            "trees" => Some(Catalog::Tree),
            "games" => Some(Catalog::Game),
            _ => None,
        }
    }
}

/// `/store/<path>`: a catalog file from the bucket. The DB and keys are never reachable here.
async fn file(State(s): State<Store>, Path(path): Path<String>) -> Response {
    if Catalog::of(&path).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match s.remote.get(&path).await {
        Ok(Some(bytes)) => axum::body::Bytes::from_owner(bytes).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("store: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// How long a presigned URL from `/store-direct` works (a game's chunk asked for again after
/// that asks for a new one).
const DIRECT_EXPIRY: u32 = 6 * 3600;

/// `GET /store-direct/chunks/<ab>/<hash>`: how the page fetches a chunk from the bucket itself
/// (host/fetch-bridge.js): `{"url": presigned GET, "key": hex AES-256-GCM key, "object": UUID}`.
/// 404 when the bucket does not have it (or for what is not a chunk path).
async fn direct(State(s): State<Store>, Path(path): Path<String>) -> Response {
    let remote = &*s.remote;
    if !chunk_path(&path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match remote.direct(&path, DIRECT_EXPIRY).await {
        Ok(Some(d)) => {
            let mut r = axum::Json(direct_json(&d)).into_response();
            // (a key: kept by nobody else; the page keeps it in memory for DIRECT_EXPIRY at most)
            r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
            r
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("store-direct {path}: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// A /store-direct answer: `{url, key, object}`, and `pack: {id, url, offset, len}` when a
/// delivery pack holds the object too (storage.rs pack_objects; the page reads its neighbours
/// in the pack with the same GET).
fn direct_json(d: &crate::storage::Direct) -> serde_json::Value {
    let key: String = d.key.iter().map(|b| format!("{b:02x}")).collect();
    let mut v = serde_json::json!({ "url": d.url, "key": key, "object": d.object.to_string() });
    if let Some(p) = &d.pack {
        v["pack"] = serde_json::json!({ "id": p.id.to_string(), "url": p.url, "offset": p.offset, "len": p.len });
    }
    v
}

/// Whether `path` is a chunk's path (`chunks/<ab>/<hash>`), all /store-direct answers for.
fn chunk_path(path: &str) -> bool {
    path.split('/').count() == 3 && path.starts_with("chunks/") && path.bytes().all(|b| b.is_ascii_hexdigit() || b == b'/' || b.is_ascii_lowercase())
}

/// Chunk paths one `POST /store-direct` may ask for.
const DIRECT_MANY: usize = 4096;

/// `POST /store-direct` with a JSON list of chunk paths: `{"<path>": {url, key, object}}` for
/// those the page may fetch from the bucket itself, as `GET /store-direct/<path>` answers each
/// (host/fetch-bridge.js masuDirectMany: a game's chunks asked for at once, so its first read
/// of each goes to the bucket without a round trip here first). Those it leaves out the bucket
/// does not have; 413 past DIRECT_MANY paths.
async fn direct_many(State(s): State<Store>, axum::Json(paths): axum::Json<Vec<String>>) -> Response {
    let remote = &*s.remote;
    if paths.len() > DIRECT_MANY {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let mut out = serde_json::Map::new();
    let paths: Vec<String> = paths.into_iter().filter(|p| chunk_path(p)).collect();
    match remote.direct_many(paths, DIRECT_EXPIRY).await {
        Ok(all) => {
            for (path, d) in all {
                out.insert(path, direct_json(&d));
            }
        }
        Err(e) => {
            eprintln!("store-direct: {e:#}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    }
    let mut r = axum::Json(serde_json::Value::Object(out)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    r
}

/// Whether `name` is one of sake's wasm files (`sake.wasm`, `sake-grow.wasm`, ...).
fn wasm_name(name: &str) -> bool {
    name.len() <= 64 && name.ends_with(".wasm") && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.') && !name.starts_with('.')
}

/// Puts sake's wasm files (`<web>/sake/*.wasm`, ~23 MB each, ~5 MB as zstd) in the bucket as
/// chunks (named by their BLAKE3 like a game's, zstd-coded, encrypted), those not there yet,
/// and notes them in `wasm` for `/sake-direct`: the page then fetches sake from the bucket at
/// the bucket's speed, not through this server's own line (worker2's gave ~100 KB/s at times,
/// minutes before a game could start). Each build once: a release's files are the same at
/// every start.
async fn publish_wasm(remote: &Storage, web: &std::path::Path, wasm: &std::sync::RwLock<std::collections::HashMap<String, String>>) -> anyhow::Result<usize> {
    let mut names: Vec<String> = std::fs::read_dir(web.join("sake"))?.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| wasm_name(n)).collect();
    names.sort();
    let mut sent = 0;
    for name in names {
        let bytes = tokio::fs::read(web.join("sake").join(&name)).await?;
        let h = blake3::hash(&bytes).to_hex()[..32].to_string();
        let logical = format!("chunks/{}/{h}", &h[..2]);
        if !remote.has(&logical)? {
            let z = tokio::task::spawn_blocking(move || zstd::bulk::compress(&bytes, 19)).await??;
            remote.put(&logical, &z).await?;
            sent += 1;
        }
        wasm.w().insert(name, h);
    }
    Ok(sent)
}

/// `GET /sake-direct/<file>`: how the page fetches sake's wasm `file` from the bucket, as
/// `/store-direct` answers for a chunk, and `chunk`: its name (the decoded bytes' BLAKE3,
/// checked by the page). 404 until [`publish_wasm`] put it there (or for another name).
async fn sake_direct(State(s): State<Store>, Path(name): Path<String>) -> Response {
    let remote = &*s.remote;
    let Some(h) = s.wasm.r().get(&name).cloned() else { return StatusCode::NOT_FOUND.into_response() };
    match remote.direct(&format!("chunks/{}/{h}", &h[..2]), DIRECT_EXPIRY).await {
        Ok(Some(d)) => {
            let mut v = direct_json(&d);
            v["chunk"] = h.into();
            let mut r = axum::Json(v).into_response();
            r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
            r
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("sake-direct {name}: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

/// The store's routes; `web`: the web app's folder, whose sake wasm files go to the bucket.
pub fn router(local: PathBuf, web: Option<PathBuf>) -> anyhow::Result<Router> {
    use anyhow::Context as _;
    let db = crate::library::Library::new(&local)?;
    let remote = Storage::configured(db)?.context("the store's routes need a bucket (MASU_STORAGE_CONFIG or --dev-bucket)")?;
    Ok(routes(Store { remote: Arc::new(remote), wasm: Default::default() }, web))
}

fn routes(state: Store, web: Option<PathBuf>) -> Router {
    if let Some(web) = web {
        let s = state.clone();
        tokio::spawn(async move {
            match publish_wasm(&s.remote, &web, &s.wasm).await {
                Ok(0) => {}
                Ok(n) => eprintln!("sake: {n} wasm files put in the bucket (pages fetch them from there)"),
                Err(e) => eprintln!("sake: wasm not put in the bucket (pages fetch it from here): {e:#}"),
            }
        });
    }
    Router::new()
        .route("/store/{*path}", get(file))
        .route("/sake-direct/{name}", get(sake_direct))
        .route("/store-direct/{*path}", get(direct))
        .route("/store-direct", axum::routing::post(direct_many))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderMap, Request};
    use tower::ServiceExt;

    async fn get_(app: &Router, uri: &str, range: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut r = Request::builder().uri(uri);
        if let Some(v) = range {
            r = r.header(header::RANGE, v);
        }
        let res = app.clone().oneshot(r.body(Body::empty()).unwrap()).await.unwrap();
        let (p, b) = res.into_parts();
        (p.status, p.headers, to_bytes(b, usize::MAX).await.unwrap().to_vec())
    }

    /// /store serves the catalog from the bucket, and never a chunk: not one the bucket has, not
    /// one on this machine's disk (the page fetches chunks from the bucket itself), not the DB.
    #[tokio::test]
    async fn store_serves_the_catalog_and_no_chunk() {
        let (fake, remote, dir) = crate::storage::gc_tests::serve(0).await;
        let h = format!("ab{}", "1".repeat(30));
        remote.put(&format!("chunks/ab/{h}"), b"a chunk").await.unwrap();
        remote.put("trees/game.tsv", b"vnvm-tree\t2\t64\n").await.unwrap();
        remote.put("games/game.json", b"{}").await.unwrap();
        std::fs::create_dir_all(dir.join("chunks/ab")).unwrap();
        std::fs::write(dir.join(format!("chunks/ab/{h}")), b"a chunk on the disk").unwrap();
        std::fs::write(dir.join("trees-local.tsv"), b"x").unwrap();
        let app = routes(Store { remote: Arc::new(remote), wasm: Default::default() }, None);
        assert_eq!(get_(&app, "/store/trees/game.tsv", None).await.2, b"vnvm-tree\t2\t64\n");
        assert_eq!(get_(&app, "/store/games/game.json", None).await.2, b"{}");
        let before = fake.gets();
        for p in [format!("/store/chunks/ab/{h}"), "/store/library.sqlite3".into(), "/store/trees/../chunks/x".into(), "/store/trees-local.tsv".into(), "/store/keys".into()] {
            for range in [None, Some("bytes=0-3")] {
                assert_eq!(get_(&app, &p, range).await.0, StatusCode::NOT_FOUND, "{p} {range:?}");
            }
        }
        assert_eq!(fake.gets(), before, "nothing asked of the bucket for them");
        assert_eq!(get_(&app, "/store/trees/none.tsv", None).await.0, StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(dir);
    }

    proptest::proptest! {
        /// Catalog::of takes plain paths under trees/ and games/ only: never chunks/, a way up,
        /// or anything else of the store.
        #[test]
        fn catalog_model(top in "(trees|games|chunks|keys|\\.\\.|)", rest in "(/[a-z.]{0,4}){0,3}") {
            let path = format!("{top}{rest}");
            let got = Catalog::of(&path);
            let plain = std::path::Path::new(&path).components().all(|c| matches!(c, std::path::Component::Normal(_)));
            let want = match top.as_str() {
                "trees" if plain => Some(Catalog::Tree),
                "games" if plain => Some(Catalog::Game),
                _ => None,
            };
            proptest::prop_assert_eq!(got, want, "{}", path);
        }
    }

    /// /store-direct: a presigned URL the bucket answers, and the key that opens what it
    /// gives as the browser opens it (AES-256-GCM, nonce first, the UUID as associated data):
    /// the chunk as stored. 404 for what the bucket does not have.
    #[tokio::test]
    async fn direct_gives_what_opens_the_chunk() {
        let (_fake, remote, dir) = crate::storage::gc_tests::serve(0).await;
        let raw: Vec<u8> = (0..200_000u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
        let stored = zstd::encode_all(&raw[..], 3).unwrap();
        let h = "ab".to_string() + &"5".repeat(30);
        remote.put(&format!("chunks/ab/{h}"), &stored).await.unwrap();
        let state = Store { remote: Arc::new(remote), wasm: Default::default() };
        let app = Router::new().route("/store-direct/{*path}", get(direct)).with_state(state);
        let (st, hd, body) = get_(&app, &format!("/store-direct/chunks/ab/{h}"), None).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(hd[header::CACHE_CONTROL], "no-store");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let (url, key, object) = (v["url"].as_str().unwrap(), v["key"].as_str().unwrap(), v["object"].as_str().unwrap());
        assert!(url.contains("X-Amz-Signature=") && url.contains("X-Amz-Expires=21600"), "{url}");
        let cipher = reqwest::get(url).await.unwrap().bytes().await.unwrap().to_vec();
        let key: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&key[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let uuid = uuid::Uuid::parse_str(object).unwrap();
        let open = |key: &[u8], cipher: &[u8]| -> Option<Vec<u8>> {
            use ring::aead;
            let k = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, key).ok()?);
            let mut d = cipher.get(12..)?.to_vec();
            let p = k.open_in_place(aead::Nonce::assume_unique_for_key(cipher[..12].try_into().ok()?), aead::Aad::from(uuid.as_bytes()), &mut d).ok()?;
            Some(p.to_vec())
        };
        assert_eq!(open(&key, &cipher).unwrap(), stored);
        assert_eq!(zstd::decode_all(&stored[..]).unwrap(), raw);
        // another key, a changed byte: refused
        let mut bad = key.clone();
        bad[0] ^= 1;
        assert!(open(&bad, &cipher).is_none());
        let mut c2 = cipher.clone();
        c2[40] ^= 1;
        assert!(open(&key, &c2).is_none());
        // what the bucket does not have, what is not a chunk path
        for p in [format!("/store-direct/chunks/ab/{}", "6".repeat(32)), "/store-direct/trees/x.tsv".into(), "/store-direct/chunks/../x".into()] {
            assert_eq!(get_(&app, &p, None).await.0, StatusCode::NOT_FOUND, "{p}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// sake's wasm in the bucket: publish_wasm puts each `<web>/sake/*.wasm` there once (a second
    /// start sends nothing), and /sake-direct answers what opens to the file's zstd, named by the
    /// file's BLAKE3; other names 404.
    #[tokio::test]
    async fn sake_wasm_from_the_bucket() {
        let (_fake, remote, dir) = crate::storage::gc_tests::serve(0).await;
        let web = dir.join("web");
        std::fs::create_dir_all(web.join("sake")).unwrap();
        let wasm: Vec<u8> = b"\0asm\x01\0\0\0".iter().copied().chain((0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)).collect();
        std::fs::write(web.join("sake/sake.wasm"), &wasm).unwrap();
        std::fs::write(web.join("sake/sake-grow.wasm"), &wasm[..1000]).unwrap();
        std::fs::write(web.join("sake/sake.js"), b"js").unwrap();
        std::fs::write(web.join("sake/.x.wasm"), b"hidden").unwrap();
        let state = Store { remote: Arc::new(remote), wasm: Default::default() };
        let remote = &*state.remote;
        assert_eq!(publish_wasm(remote, &web, &state.wasm).await.unwrap(), 2);
        assert_eq!(publish_wasm(remote, &web, &state.wasm).await.unwrap(), 0, "the same build again: nothing sent");
        assert_eq!(state.wasm.r().len(), 2);
        let h = blake3::hash(&wasm).to_hex()[..32].to_string();
        assert!(remote.has(&format!("chunks/{}/{h}", &h[..2])).unwrap());
        // (the server reads it as any chunk too)
        assert!(remote.get(&format!("chunks/{}/{h}", &h[..2])).await.unwrap().is_some());
        let app = Router::new().route("/sake-direct/{name}", get(sake_direct)).with_state(state.clone());
        let (st, _, body) = get_(&app, "/sake-direct/sake.wasm", None).await;
        assert_eq!(st, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["chunk"], h);
        let cipher = reqwest::get(v["url"].as_str().unwrap()).await.unwrap().bytes().await.unwrap().to_vec();
        let k = v["key"].as_str().unwrap();
        let key: Vec<u8> = (0..32).map(|i| u8::from_str_radix(&k[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let uuid = uuid::Uuid::parse_str(v["object"].as_str().unwrap()).unwrap();
        use ring::aead;
        let lk = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap());
        let mut d = cipher[12..].to_vec();
        let plain = lk.open_in_place(aead::Nonce::assume_unique_for_key(cipher[..12].try_into().unwrap()), aead::Aad::from(uuid.as_bytes()), &mut d).unwrap();
        assert_eq!(zstd::decode_all(&plain[..]).unwrap(), wasm);
        for p in ["/sake-direct/sake.js", "/sake-direct/.x.wasm", "/sake-direct/other.wasm", "/sake-direct/..%2Fsake.wasm"] {
            assert_eq!(get_(&app, p, None).await.0, StatusCode::NOT_FOUND, "{p}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    proptest::proptest! {
        /// wasm_name takes only plain names ending in .wasm (no folders, no hidden files)
        #[test]
        fn wasm_name_model(n in "[a-z./_%-]{0,12}(\\.wasm)?") {
            let ok = wasm_name(&n);
            proptest::prop_assert_eq!(ok, n.ends_with(".wasm") && !n.starts_with('.') && !n.contains('/') && !n.contains('%'));
        }
    }


    /// POST /store-direct: many chunks at once, each answered as GET /store-direct/<path>
    /// answers it alone (the same key and object; the URL opens the same ciphertext); what the
    /// bucket lacks or is not a chunk path left out; too many refused.
    #[tokio::test]
    async fn direct_many_answers_each_as_alone() {
        let (_fake, remote, dir) = crate::storage::gc_tests::serve(0).await;
        let mut names = Vec::new();
        for i in 0..5u8 {
            let raw = vec![i; 1000 + i as usize];
            let h = format!("{:02x}{}", 0xa0 + i, "7".repeat(30));
            remote.put(&format!("chunks/{}/{h}", &h[..2]), &zstd::encode_all(&raw[..], 3).unwrap()).await.unwrap();
            names.push(format!("chunks/{}/{h}", &h[..2]));
        }
        // (the first three in a delivery pack: the answers say where, the rest do not)
        assert_eq!(remote.pack_objects("g", &names[..3], 1 << 20).await.unwrap().packs, 1);
        let state = Store { remote: Arc::new(remote), wasm: Default::default() };
        let app = Router::new().route("/store-direct/{*path}", get(direct)).route("/store-direct", axum::routing::post(direct_many)).with_state(state);
        let post = |body: String| {
            let app = app.clone();
            async move {
                let r = app.oneshot(Request::post("/store-direct").header("content-type", "application/json").body(Body::from(body)).unwrap()).await.unwrap();
                let st = r.status();
                (st, to_bytes(r.into_body(), usize::MAX).await.unwrap())
            }
        };
        let mut ask = names.clone();
        ask.push(format!("chunks/ab/{}", "6".repeat(32)));
        ask.push("trees/x.tsv".into());
        let (st, body) = post(serde_json::to_string(&ask).unwrap()).await;
        assert_eq!(st, StatusCode::OK);
        let all: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(all.as_object().unwrap().len(), names.len(), "{all}");
        for n in &names {
            let (st, _, one) = get_(&app, &format!("/store-direct/{n}"), None).await;
            assert_eq!(st, StatusCode::OK);
            let one: serde_json::Value = serde_json::from_slice(&one).unwrap();
            assert_eq!(all[n]["key"], one["key"]);
            assert_eq!(all[n]["object"], one["object"]);
            let a = reqwest::get(all[n]["url"].as_str().unwrap()).await.unwrap().bytes().await.unwrap();
            let b = reqwest::get(one["url"].as_str().unwrap()).await.unwrap().bytes().await.unwrap();
            assert_eq!(a, b);
            assert_eq!(all[n]["pack"], one["pack"]);
            if names[..3].contains(n) {
                let p = &all[n]["pack"];
                let body = reqwest::get(p["url"].as_str().unwrap()).await.unwrap().bytes().await.unwrap();
                let (o, l) = (p["offset"].as_u64().unwrap() as usize, p["len"].as_u64().unwrap() as usize);
                assert_eq!(&body[o..o + l], &a[..], "{n}: the pack's span is the object");
                assert!(p["id"].as_str().is_some_and(|id| p["url"].as_str().unwrap().contains(id)));
            } else {
                assert!(all[n].get("pack").is_none(), "{n}");
            }
        }
        let many = vec![names[0].clone(); DIRECT_MANY + 1];
        assert_eq!(post(serde_json::to_string(&many).unwrap()).await.0, StatusCode::PAYLOAD_TOO_LARGE);
        let _ = std::fs::remove_dir_all(dir);
    }
}

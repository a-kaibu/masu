//! Encrypted immutable S3 objects. Names and key envelopes live in SQLite.
use kome::locks::Lock as _;
use crate::{envelope::Envelope, library::Library};
use anyhow::{anyhow, ensure, Context, Result};
use rusqlite::OptionalExtension;
use s3::{creds::Credentials, Bucket, Region};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

#[cfg(test)]
mod rustfs_tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    bucket: String,
    region: String,
    endpoint: Option<String>,
    #[serde(default)]
    prefix: String,
    active_key: String,
    /// Hex encoded 256-bit keys. This file must be backed up separately from S3.
    keys: BTreeMap<String, String>,
    /// Uploads at once in `push` [default: PUTS]. Each object is a chunk (4 MiB on average,
    /// pack.rs DEFAULT_CHUNK_KIB): with a far gateway a round trip, not the line, bounds one
    /// upload, so more at once go faster.
    #[serde(default)]
    uploads: Option<usize>,
    /// Where objects read from the bucket are kept on this machine [default: <cache>/s3, e.g.
    /// ~/.cache/masu/s3], as the bucket has them (encrypted, checked again on every read).
    #[serde(default)]
    cache: Option<std::path::PathBuf>,
    /// The most that may be kept there, in GiB [default: 64]; 0 keeps nothing.
    #[serde(default)]
    cache_gib: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes,
        extract::{Path as UrlPath, State},
        http::StatusCode,
        response::IntoResponse,
        routing::get,
        Router,
    };
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn s3_roundtrip_rotation_and_disaster_restore() {
        type Objects = Arc<Mutex<BTreeMap<String, Vec<u8>>>>;
        async fn put(
            State(s): State<Objects>,
            UrlPath(p): UrlPath<String>,
            b: Bytes,
        ) -> StatusCode {
            s.locked().insert(p, b.to_vec());
            StatusCode::OK
        }
        async fn read(
            State(s): State<Objects>,
            UrlPath(p): UrlPath<String>,
        ) -> axum::response::Response {
            s.locked()
                .get(&p)
                .cloned()
                .map(IntoResponse::into_response)
                .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
        }
        let objects: Objects = Default::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/{*path}", get(read).put(put))
            .with_state(objects.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = std::env::temp_dir().join(format!("masu-s3-{}", Uuid::new_v4()));
        let db = Library::new(&dir).unwrap();
        db.set_profile(
            "game",
            &crate::library::Profile {
                favorite: true,
                note: "restore me".into(),
            },
        )
        .unwrap();
        let config = Config {
            bucket: "test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint.clone()),
            prefix: "".into(),
            active_key: "old".into(),
            keys: BTreeMap::from([
                ("old".into(), "01".repeat(32)),
                ("new".into(), "02".repeat(32)),
            ]),
            uploads: None,
            cache: None,
            cache_gib: None,
        };
        let bucket = Bucket::new(
            "test",
            Region::Custom {
                region: "us-east-1".into(),
                endpoint,
            },
            Credentials::new(Some("test"), Some("test"), None, None, None).unwrap(),
        )
        .unwrap()
        .with_path_style();
        let mut s = Storage {
            db,
            bucket,
            config,
            shared: Shared::new(8, None),
        };
        s.put("chunks/test", b"game bytes").await.unwrap();
        let snapshot = s.backup().await.unwrap();
        let before = objects.locked().clone();
        s.config.active_key = "new".into();
        s.rotate_objects().unwrap();
        s.rotate_backup(snapshot).await.unwrap();
        // Only the small backup envelope changed; encrypted object payloads are immutable.
        for (name, bytes) in before {
            if !name.ends_with(&snapshot.to_string()) {
                assert_eq!(objects.locked()[&name], bytes);
            }
        }
        s.config.keys.remove("old");
        assert_eq!(&*s.get("chunks/test").await.unwrap().unwrap(), b"game bytes");
        let restored = dir.join("restored.db");
        s.restore(snapshot, &restored).await.unwrap();
        s.db = Library(restored);
        *s.shared.cache.locked() = Default::default();
        assert!(s.db.profile("game").unwrap().favorite);
        assert_eq!(&*s.get("chunks/test").await.unwrap().unwrap(), b"game bytes");
        server.abort();
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
pub(crate) mod gc_tests {
    use super::*;
    mod saves_trace;
    use axum::{body::Bytes, extract::{Path as UrlPath, State}, http::StatusCode, response::IntoResponse, routing::get, Router};
    use std::sync::{atomic::{AtomicUsize, Ordering::SeqCst}, Arc, Mutex};

    /// Keys for the test's own fake bucket: never the machine's (Storage::open takes them)
    pub(crate) fn test_credentials() -> Credentials {
        Credentials::new(Some("t"), Some("t"), None, None, None).unwrap()
    }

    /// An S3 bucket in memory answering after `delay`; counts requests and the most PUTs or
    /// GETs on their way at once.
    #[derive(Clone, Default)]
    pub(crate) struct Fake {
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        delay_ms: u64,
        now: Arc<AtomicUsize>,
        most: Arc<AtomicUsize>,
        gets: Arc<AtomicUsize>,
        /// requests of any kind, answered or refused
        requests: Arc<AtomicUsize>,
        /// refuse the next this many requests with `fail_status`
        fail_next: Arc<AtomicUsize>,
        /// refuse every n-th request with `fail_status` (0: none)
        fail_every: Arc<AtomicUsize>,
        /// what a refused request gets (0: 503)
        fail_status: Arc<AtomicUsize>,
        /// PUTs of more bytes than this (0: none) wait for `gate` (a test holds it to stall them)
        hold_over: Arc<AtomicUsize>,
        gate: Arc<tokio::sync::RwLock<()>>,
        /// PUTs waiting for `gate` now
        holding: Arc<AtomicUsize>,
    }

    impl Fake {
        pub(crate) fn gets(&self) -> usize {
            self.gets.load(SeqCst)
        }
        /// Some(status) when this request is refused.
        fn refuse(&self) -> Option<StatusCode> {
            let n = self.requests.fetch_add(1, SeqCst) + 1;
            let mut left = self.fail_next.load(SeqCst);
            let burst = loop {
                if left == 0 {
                    break false;
                }
                match self.fail_next.compare_exchange(left, left - 1, SeqCst, SeqCst) {
                    Ok(_) => break true,
                    Err(now) => left = now,
                }
            };
            let every = self.fail_every.load(SeqCst);
            (burst || (every > 0 && n % every == 0)).then(|| match self.fail_status.load(SeqCst) {
                0 => StatusCode::SERVICE_UNAVAILABLE,
                s => StatusCode::from_u16(s as u16).unwrap(),
            })
        }
    }

    pub(crate) async fn serve(delay_ms: u64) -> (Fake, Storage, std::path::PathBuf) {
        async fn busy(f: &Fake) {
            let n = f.now.fetch_add(1, SeqCst) + 1;
            f.most.fetch_max(n, SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(f.delay_ms)).await;
            f.now.fetch_sub(1, SeqCst);
        }
        async fn put(State(f): State<Fake>, UrlPath(p): UrlPath<String>, b: Bytes) -> StatusCode {
            if let Some(s) = f.refuse() {
                return s;
            }
            let over = f.hold_over.load(SeqCst);
            if over > 0 && b.len() > over {
                f.holding.fetch_add(1, SeqCst);
                let _open = f.gate.read().await;
                f.holding.fetch_sub(1, SeqCst);
            }
            busy(&f).await;
            f.objects.locked().insert(p, b.to_vec());
            StatusCode::OK
        }
        async fn read(State(f): State<Fake>, UrlPath(p): UrlPath<String>) -> axum::response::Response {
            if let Some(s) = f.refuse() {
                return s.into_response();
            }
            f.gets.fetch_add(1, SeqCst);
            busy(&f).await;
            f.objects.locked().get(&p).cloned().map(IntoResponse::into_response).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
        }
        async fn delete(State(f): State<Fake>, UrlPath(p): UrlPath<String>) -> StatusCode {
            if let Some(s) = f.refuse() {
                return s;
            }
            f.objects.locked().remove(&p);
            StatusCode::NO_CONTENT
        }
        let f = Fake { delay_ms, ..Default::default() };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route("/{*path}", get(read).put(put).delete(delete)).layer(axum::extract::DefaultBodyLimit::max(64 << 20)).with_state(f.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = std::env::temp_dir().join(format!("masu-gc-{}", Uuid::new_v4()));
        let config = Config {
            bucket: "test".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint.clone()),
            prefix: "".into(),
            active_key: "k".into(),
            keys: BTreeMap::from([("k".into(), "03".repeat(32))]),
            uploads: None,
            cache: None,
            cache_gib: None,
        };
        let bucket = Bucket::new("test", Region::Custom { region: "us-east-1".into(), endpoint }, Credentials::new(Some("t"), Some("t"), None, None, None).unwrap()).unwrap().with_path_style();
        let s = Storage { db: Library::new(&dir).unwrap(), bucket, config, shared: Shared::new(8, None) };
        (f, s, dir)
    }

    fn names(f: &Fake) -> std::collections::BTreeSet<String> {
        f.objects.locked().keys().map(|k| k.trim_start_matches("test/").to_string()).collect()
    }

    fn named(s: &Storage) -> std::collections::BTreeSet<String> {
        let c = s.db.connect().unwrap();
        let mut q = c.prepare("SELECT envelope FROM objects").unwrap();
        let v: Vec<String> = q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        v.iter().map(|e| serde_json::from_str::<Envelope>(e).unwrap().object.to_string()).collect()
    }

    /// Moves everything recorded `ms` into the past.
    fn age_retired(s: &Storage, ms: i64) {
        let c = s.db.connect().unwrap();
        c.execute("UPDATE retired SET retired_ms=retired_ms-?", [ms]).unwrap();
        c.execute("UPDATE backups SET created_ms=created_ms-?", [ms]).unwrap();
    }

    /// Random puts over a few names, saves, forgets and backups, then gc: what is named
    /// stays and reads back; what is gone is only what nothing (no row, no kept backup's
    /// database) names; a backup restores whole after gc.
    #[tokio::test]
    async fn gc_deletes_only_what_nothing_names() {
        let (f, s, dir) = serve(0).await;
        let mut seed = 0x6a09e667f3bcc909u64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let mut want: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut backups = Vec::new();
        for step in 0..120 {
            match rnd(10) {
                0 => {
                    let l = format!("trees/t{}", rnd(4));
                    s.forget(&l).unwrap();
                    want.remove(&l);
                }
                1 if step > 60 && backups.len() < 2 => {
                    backups.push((s.backup().await.unwrap(), want.clone()));
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
                _ => {
                    let l = format!("trees/t{}", rnd(4));
                    let v = format!("{step}").into_bytes();
                    s.put(&l, &v).await.unwrap();
                    want.insert(l, v);
                }
            }
        }
        // nothing is old enough yet
        assert_eq!(s.gc(std::time::Duration::from_secs(3600), None).await.unwrap().deleted, 0);
        age_retired(&s, 7_200_000);
        let before = names(&f);
        let r = s.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
        let after = names(&f);
        assert!(r.deleted > 0, "{r:?}");
        assert_eq!(before.len() - after.len(), r.deleted);
        assert!(named(&s).is_subset(&after));
        *s.shared.cache.locked() = Default::default();
        for (l, v) in &want {
            assert_eq!(&*s.get(l).await.unwrap().unwrap(), &v[..]);
        }
        // each backup restores whole: its rows' objects are all still there
        for (i, (b, then)) in backups.iter().enumerate() {
            let out = dir.join(format!("restored{i}.db"));
            s.restore(*b, &out).await.unwrap();
            let r = Storage { db: Library(out), bucket: s.bucket.clone(), config: Config { keys: s.config.keys.clone(), active_key: "k".into(), bucket: "test".into(), region: "us-east-1".into(), endpoint: None, prefix: "".into(), uploads: None, cache: None, cache_gib: None }, shared: Shared::new(8, None) };
            for (l, v) in then {
                assert_eq!(&*r.get(l).await.unwrap().unwrap(), &v[..], "backup {i} {l}");
            }
        }
        // with the backups dropped, what only they named goes too; what is named stays
        let r = s.gc(std::time::Duration::from_secs(3600), Some(0)).await.unwrap();
        assert_eq!(r.backups_dropped, backups.len());
        assert_eq!(names(&f), named(&s));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The store as restored from a backup at `db` (the same bucket and keys as `s`).
    fn restored(s: &Storage, db: std::path::PathBuf) -> Storage {
        Storage { db: Library(db), bucket: s.bucket.clone(), config: Config { keys: s.config.keys.clone(), active_key: "k".into(), bucket: "test".into(), region: "us-east-1".into(), endpoint: None, prefix: "".into(), uploads: None, cache: None, cache_gib: None }, shared: Shared::new(8, None) }
    }

    /// A store restored from a backup goes on (puts, forgets, its own backups) and gcs, over
    /// seeded random histories: the backup it came from restores whole again afterwards, and
    /// so does each one the restored store made (review e18bf44: restore did not pin the backup
    /// it came from, and a gc after an update deleted its objects).
    #[tokio::test]
    async fn a_restored_store_keeps_the_backup_it_came_from() {
        for seed in [0x9e3779b97f4a7c15u64, 0xbf58476d1ce4e5b9, 0x94d049bb133111eb] {
            let (f, s, dir) = serve(0).await;
            let mut seed = seed;
            let mut rnd = move |n: usize| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                usize::try_from(seed % u64::try_from(n).unwrap_or(u64::MAX)).unwrap_or(0)
            };
            let mut want: BTreeMap<String, Vec<u8>> = BTreeMap::new();
            for k in 0..4 {
                let v = format!("first {k}").into_bytes();
                s.put(&format!("trees/t{k}"), &v).await.unwrap();
                want.insert(format!("trees/t{k}"), v);
            }
            let mut backups = vec![(s.backup().await.unwrap(), want.clone())];
            let mut store = s;
            for round in 0..3 {
                let (b, _) = backups[rnd(backups.len())].clone();
                let db = dir.join(format!("restored-{round}.db"));
                store.restore(b, &db).await.unwrap();
                let r = restored(&store, db);
                want = backups.iter().find(|(id, _)| *id == b).unwrap().1.clone();
                for step in 0..30 {
                    let l = format!("trees/t{}", rnd(4));
                    match rnd(8) {
                        0 => {
                            r.forget(&l).unwrap();
                            want.remove(&l);
                        }
                        1 if backups.len() < 6 => backups.push((r.backup().await.unwrap(), want.clone())),
                        _ => {
                            let v = format!("round {round} step {step}").into_bytes();
                            r.put(&l, &v).await.unwrap();
                            want.insert(l, v);
                        }
                    }
                }
                age_retired(&r, 7_200_000);
                let gc = r.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
                assert!(gc.deleted > 0 || gc.kept > 0, "{gc:?}");
                for (l, v) in &want {
                    assert_eq!(&*r.get(l).await.unwrap().unwrap(), &v[..], "round {round}: {l}");
                }
                // (every backup still restores whole, the one this store came from too)
                for (i, (id, then)) in backups.iter().enumerate() {
                    let out = dir.join(format!("check-{round}-{i}.db"));
                    r.restore(*id, &out).await.unwrap_or_else(|e| panic!("round {round}: backup {i}: {e:#}"));
                    let c = restored(&r, out);
                    for (l, v) in then {
                        assert_eq!(&*c.get(l).await.unwrap().unwrap(), &v[..], "round {round}: backup {i}: {l}");
                    }
                }
                store = r;
            }
            drop(f);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn gc_protects_the_snapshot_during_publication() {
        let (f, mut s, dir) = serve(0).await;
        s.put("saves/game/1", b"before").await.unwrap();
        f.hold_over.store(1, SeqCst);
        let gate = f.gate.write().await;
        let (backup, ()) = tokio::join!(s.backup(), async {
            until(|| f.holding.load(SeqCst) == 1).await;
            s.forget("saves/game/1").unwrap();
            age_retired(&s, 60_000);
            assert_eq!(s.gc(std::time::Duration::ZERO, None).await.unwrap().deleted, 0);
            drop(gate);
        });
        let restored = dir.join("empty.db");
        s.restore(backup.unwrap(), &restored).await.unwrap();
        s.db = Library(restored);
        s.shared = Shared::new(8, None);
        assert_eq!(&*s.get("saves/game/1").await.unwrap().unwrap(), b"before");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Every packs row's pack, as the bucket has it: its span is the object's own ciphertext,
    /// and a pack's spans are side by side in the order they were packed.
    fn check_packs(f: &Fake, s: &Storage) {
        let c = s.db.connect().unwrap();
        let mut q = c.prepare("SELECT p.logical,p.object,p.pack,p.offset,p.len,o.envelope FROM packs p JOIN objects o USING(logical) ORDER BY p.pack,p.offset").unwrap();
        let rows: Vec<(String, String, String, i64, i64, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let objects = f.objects.locked();
        let get = |id: &str| objects.iter().find(|(k, _)| k.ends_with(id)).map(|(_, v)| v.clone());
        let mut end: BTreeMap<String, i64> = BTreeMap::new();
        for (l, object, pack, offset, len, env) in rows {
            let body = get(&pack).unwrap_or_else(|| panic!("pack {pack} of {l} not in the bucket"));
            let own = get(&object).unwrap_or_else(|| panic!("object {object} of {l} not in the bucket"));
            assert_eq!(&body[offset as usize..(offset + len) as usize], &own[..], "{l}");
            // (still the object the row names: a replaced one's span stays, but direct leaves it out)
            let e: Envelope = serde_json::from_str(&env).unwrap();
            if e.object.to_string() == object {
                assert_eq!(e.decrypt(&s.key("k").unwrap(), &body[offset as usize..(offset + len) as usize]).is_ok(), true);
            }
            let at = end.entry(pack).or_insert(0);
            // (side by side when packed; a replaced object's row moved on to a newer pack)
            assert!(*at <= offset, "{l}: spans overlap");
            *at = offset + len;
        }
    }

    /// Delivery packs over random puts, replacements, packs and gcs: each pack's spans are the
    /// objects' own ciphertexts side by side in read order, each pack reaches the target but
    /// the last; packing again packs only what is new or replaced; direct answers name a span
    /// only for the object it holds; gc never deletes a pack a row names, and deletes one no
    /// row names once retired long enough.
    #[tokio::test]
    async fn packs_hold_objects_in_order_and_outlive_nothing() {
        let (f, s, dir) = serve(0).await;
        let mut seed = 0x510e527fade682d1u64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let target = 4000;
        for round in 0..12 {
            for _ in 0..rnd(12) + 1 {
                let l = format!("chunks/ab/{:032x}", rnd(30));
                let v: Vec<u8> = (0..rnd(1500) + 1).map(|i| (i as u64 + round) as u8).collect();
                s.put(&l, &v).await.unwrap();
            }
            let mut order: Vec<String> = (0..30).map(|i| format!("chunks/ab/{i:032x}")).collect();
            for i in (1..order.len()).rev() {
                order.swap(i, rnd(i as u64 + 1) as usize);
            }
            let packs_now = |s: &Storage| -> std::collections::BTreeSet<String> {
                let c = s.db.connect().unwrap();
                let mut q = c.prepare("SELECT DISTINCT pack FROM packs").unwrap();
                q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
            };
            let before = packs_now(&s);
            let p = s.pack_objects("g", &order, target).await.unwrap();
            let made: Vec<String> = packs_now(&s).difference(&before).cloned().collect();
            assert_eq!(made.len(), p.packs);
            // (each of those not in a pack yet went into one, in this order)
            let c = s.db.connect().unwrap();
            let objects: i64 = c.query_row("SELECT COUNT(*) FROM objects", [], |r| r.get(0)).unwrap();
            let current: i64 = c.query_row("SELECT COUNT(*) FROM packs p JOIN objects o USING(logical) WHERE instr(o.envelope,p.object) > 0", [], |r| r.get(0)).unwrap();
            assert_eq!(current, objects, "round {round}: every object in a pack");
            assert_eq!(p.objects + p.skipped, order.len());
            let mut short = 0;
            let mut total = 0;
            for pack in &made {
                let n = f.objects.locked().iter().find(|(k, _)| k.ends_with(pack.as_str())).unwrap().1.len();
                total += n as u64;
                short += (n < target) as usize;
                let mut q = c.prepare("SELECT logical,offset,len FROM packs WHERE pack=? ORDER BY offset").unwrap();
                let rows: Vec<(String, i64, i64)> = q.query_map([pack], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                // (side by side from 0 to the pack's end)
                let mut at = 0;
                for (_, o, len) in &rows {
                    assert_eq!(*o, at);
                    at += len;
                }
                assert_eq!(at as usize, n);
                let ls: Vec<String> = rows.into_iter().map(|r| r.0).collect();
                let pos: Vec<usize> = ls.iter().map(|l| order.iter().position(|o| o == l).unwrap()).collect();
                assert!(pos.windows(2).all(|w| w[0] < w[1]), "round {round}: in read order {pos:?}");
            }
            assert!(short <= 1, "round {round}: {short} packs short of the target");
            assert_eq!(total, p.bytes);
            // again: nothing new
            assert_eq!(s.pack_objects("g", &order, target).await.unwrap().packs, 0);
            check_packs(&f, &s);
            // a direct answer's span is the object's, for each named object
            let l = &order[rnd(30) as usize];
            if let Some(d) = s.direct(l, 60).await.unwrap() {
                let span = d.pack.expect("packed");
                let (pack, off, len): (String, i64, i64) = c.query_row("SELECT pack,offset,len FROM packs WHERE logical=?", [l], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
                assert_eq!((span.id.to_string(), span.offset, span.len), (pack, off as u64, len as u64));
                assert!(span.url.contains(&span.id.to_string()));
            }
            if rnd(3) == 0 {
                age_retired(&s, 7_200_000);
                s.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
                check_packs(&f, &s);
                // (no pack in the bucket that no row names: the retired ones went)
                let packs: std::collections::BTreeSet<String> = {
                    let mut q = c.prepare("SELECT DISTINCT pack FROM packs").unwrap();
                    q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
                };
                let mut want = named(&s);
                want.extend(packs);
                assert_eq!(names(&f), want, "round {round}");
            }
        }
        // (a pack that reaches the target exactly is closed: 3 objects of 128 bytes in the
        // bucket, a target of 2 of them)
        let three: Vec<String> = (0..3).map(|i| format!("chunks/cd/{i:032x}")).collect();
        for l in &three {
            s.put(l, &[7u8; 100]).await.unwrap();
        }
        let p = s.pack_objects("g", &three, 2 * (100 + 28)).await.unwrap();
        assert_eq!((p.packs, p.objects, p.bytes), (2, 3, 3 * 128));
        // a replaced object: no span until packed again
        let l = "chunks/ab/00000000000000000000000000000001".to_string();
        s.put(&l, b"new").await.unwrap();
        assert!(s.direct(&l, 60).await.unwrap().unwrap().pack.is_none());
        assert_eq!(s.pack_objects("g", &[l.clone()], target).await.unwrap().objects, 1);
        assert!(s.direct(&l, 60).await.unwrap().unwrap().pack.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn big_uploads_get_time_for_their_size() {
        let s = |len, n| upload_timeout(len, AtOnce::new(n)).as_secs();
        assert_eq!(s(0, 1), 60);
        assert_eq!(s(4 << 20, 1), 60 + 32);
        // (a 64 MiB pack: 9 minutes, enough at 1 Mbit/s)
        assert_eq!(s(PACK_BYTES, 1), 60 + 512);
        assert!((PACK_BYTES as u64 * 8) / s(PACK_BYTES, 1) < 1_000_000);
        // (worker1, 2026-11-17: 16 chunks of 4 MiB at once timed out at 60 s, every try)
        assert_eq!(s(4 << 20, PUTS), 60 + 512);
        assert_eq!(AtOnce::new(0), AtOnce::ALONE);
        assert_eq!(s(usize::MAX, usize::MAX), 60 + u64::MAX.div_ceil(128 << 10), "no overflow");
    }

    /// A request's failure is told with what is under it (reqwest says only "error sending
    /// request"; worker1's were timeouts, which the log never said).
    #[test]
    fn failures_are_told_with_their_causes() {
        #[derive(Debug)]
        struct Top(std::io::Error);
        impl std::fmt::Display for Top {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("error sending request")
            }
        }
        impl std::error::Error for Top {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let top = Top(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"));
        assert_eq!(causes(&top), "error sending request: timed out");
        assert_eq!(causes(&top.0), "timed out");
    }

    proptest::proptest! {
        /// Uploads sharing a 1 Mbit/s uplink each get there in time: the timeout covers the
        /// bytes of all of them at 128 KiB/s, plus the 60 s of a request; more bytes or more at
        /// once never get less time.
        #[test]
        fn upload_time_covers_the_shared_line(len in 0usize..(256 << 20), n in 1usize..64) {
            let t = upload_timeout(len, AtOnce::new(n)).as_secs_f64();
            let line = f64::from(u32::try_from(len).unwrap()) * f64::from(u32::try_from(n).unwrap()) / f64::from(128u32 << 10);
            proptest::prop_assert!(t >= 60.0 + line && t < 61.0 + line, "{t} for {len} x {n}");
            proptest::prop_assert!(upload_timeout(len + 1, AtOnce::new(n)) >= upload_timeout(len, AtOnce::new(n)));
            proptest::prop_assert!(upload_timeout(len, AtOnce::new(n + 1)) >= upload_timeout(len, AtOnce::new(n)));
        }
    }

    /// Paced packing takes at least the bytes' time at the rate, and packs the same.
    #[tokio::test]
    async fn paced_packs() {
        assert_eq!(pace(10 << 20, 1 << 20), std::time::Duration::from_secs(10));
        assert_eq!(pace(0, 0), std::time::Duration::ZERO);
        let (f, s, dir) = serve(0).await;
        let ls: Vec<String> = (0..6).map(|i| format!("chunks/ef/{i:032x}")).collect();
        for (i, l) in ls.iter().enumerate() {
            s.put(l, &vec![i as u8 + 1; 1000]).await.unwrap();
        }
        let t = std::time::Instant::now();
        let p = s.pack_objects_paced("g", &ls, 2000, Some(10_000)).await.unwrap();
        // (6 objects of 1028 bytes at 10 kB/s: 0.6 s at least)
        assert_eq!((p.packs, p.objects), (3, 6));
        assert!(t.elapsed() >= std::time::Duration::from_millis(600), "{:?}", t.elapsed());
        check_packs(&f, &s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pack past 8 MiB goes up through the bucket with the longer timeout, whole.
    #[tokio::test]
    async fn big_pack_uploads() {
        let (f, s, dir) = serve(0).await;
        let ls: Vec<String> = (0..3).map(|i| format!("chunks/ef/{i:032x}")).collect();
        for (i, l) in ls.iter().enumerate() {
            s.put(l, &vec![i as u8 + 1; 3 << 20]).await.unwrap();
        }
        let p = s.pack_objects("g", &ls, 16 << 20).await.unwrap();
        assert_eq!(p.packs, 1);
        assert!(p.bytes > 8 << 20);
        check_packs(&f, &s);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Usage by kind and by game against a model (random games sharing random chunks, packed
    /// game by game), and dropping a game's packs: its objects are read alone again, its packs
    /// leave the bucket at gc, the other games' packs stay.
    #[tokio::test]
    async fn usage_and_unpack_by_game() {
        let (f, s, dir) = serve(0).await;
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let chunk = |i: u64| format!("chunks/ab/{i:032x}");
        let mut size = BTreeMap::new();
        for i in 0..40 {
            let n = rnd(900) + 1;
            s.put(&chunk(i), &vec![i as u8; n as usize]).await.unwrap();
            size.insert(chunk(i), n);
        }
        s.put("saves/x/1", b"save").await.unwrap();
        let games: Vec<(String, Vec<String>)> = (0..5).map(|g| (format!("g{g}"), (0..rnd(15) + 1).map(|_| chunk(rnd(40))).collect())).collect();
        for (id, ls) in &games {
            s.pack_objects(id, ls, 2000).await.unwrap();
        }
        // (a pack from before packs were marked)
        s.db.connect().unwrap().execute("DELETE FROM pack_games WHERE pack=(SELECT pack FROM packs WHERE logical=?)", [&games[0].1[0]]).unwrap();
        let pack_bytes = |game: Option<&str>| -> u64 {
            let c = s.db.connect().unwrap();
            let q = match game {
                Some(_) => "SELECT COALESCE(SUM(p.len),0) FROM packs p JOIN pack_games g USING(pack) WHERE g.game=?1",
                None => "SELECT COALESCE(SUM(len),0) FROM packs WHERE pack NOT IN (SELECT pack FROM pack_games) AND ?1 IS NULL",
            };
            c.query_row(q, [game], |r| r.get::<_, i64>(0)).unwrap() as u64
        };
        let u = s.usage(&games).unwrap();
        let kind = |k: &str| u.kinds.iter().find(|x| x.0 == k).map(|x| (x.1, x.2));
        assert_eq!(kind("chunks"), Some((40, size.values().sum())));
        assert_eq!(kind("saves"), Some((1, 4)));
        let all_packs: u64 = s.db.connect().unwrap().query_row("SELECT SUM(len) FROM packs", [], |r| r.get::<_, i64>(0)).unwrap() as u64;
        assert_eq!(kind("packs").unwrap().1, all_packs);
        assert!(u.kinds.windows(2).all(|w| w[0].2 >= w[1].2), "largest first");
        for (id, ls) in &games {
            let mine: std::collections::BTreeSet<&String> = ls.iter().collect();
            let (mut own, mut shared) = (0, 0);
            for l in mine {
                if games.iter().filter(|g| g.1.contains(l)).count() == 1 { own += size[l] } else { shared += size[l] }
            }
            let g = u.games.iter().find(|g| g.game == *id).unwrap();
            assert_eq!((g.own, g.shared, g.packs), (own, shared, pack_bytes(Some(id))), "{id}");
        }
        assert_eq!(u.games.iter().find(|g| g.game == "?").map_or(0, |g| g.packs), pack_bytes(None));
        assert_eq!(u.games.iter().map(|g| g.packs).sum::<u64>(), all_packs);
        // (drop one game's packs)
        let gone = games[1].0.clone();
        let had = pack_bytes(Some(&gone));
        let (_, b) = s.unpack(Some(&[gone.clone()])).unwrap();
        assert_eq!(b, had);
        assert_eq!(pack_bytes(Some(&gone)), 0);
        for (id, _) in games.iter().filter(|g| g.0 != gone) {
            assert_eq!(s.usage(&games).unwrap().games.iter().find(|g| g.game == *id).unwrap().packs, pack_bytes(Some(id)));
        }
        age_retired(&s, 7_200_000);
        s.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
        check_packs(&f, &s);
        let packs: std::collections::BTreeSet<String> = {
            let c = s.db.connect().unwrap();
            let mut q = c.prepare("SELECT DISTINCT pack FROM packs").unwrap();
            q.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        let mut want = named(&s);
        want.extend(packs);
        assert_eq!(names(&f), want);
        // (every pack, the unmarked too; the chunks read as before)
        s.unpack(None).unwrap();
        assert_eq!(s.usage(&games).unwrap().games.iter().map(|g| g.packs).sum::<u64>(), 0);
        for l in size.keys() {
            assert!(s.direct(l, 60).await.unwrap().unwrap().pack.is_none());
            assert_eq!(s.get(l).await.unwrap().unwrap().len() as u64, size[l]);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Until `cond`, at most 10 s.
    async fn until(cond: impl Fn() -> bool) {
        let t0 = std::time::Instant::now();
        while !cond() {
            assert!(t0.elapsed() < std::time::Duration::from_secs(10), "never came");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn saves_on(s: Storage) -> Arc<crate::saves::Saves> {
        let db = s.db.clone();
        Arc::new(crate::saves::Saves::open(db, Some(s)).unwrap())
    }

    /// Review D-1: a big save state stalled on its way to the bucket holds up only its own
    /// name. Other games' saves (and the same game's other files) go through meanwhile; a
    /// second writer of the same name waits for the first and then finds the revision moved
    /// on (412, as before); no name's lock is left behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_stalled_save_holds_up_only_its_own_name() {
        let (f, s, dir) = serve(0).await;
        let saves = saves_on(s);
        f.hold_over.store(1 << 20, SeqCst);
        let gate = f.gate.write().await;
        let big = tokio::spawn({
            let saves = saves.clone();
            async move { saves.write("state.g.big", 0, &vec![7u8; 2 << 20]).await.unwrap() }
        });
        until(|| f.holding.load(SeqCst) == 1).await;
        let same = tokio::spawn({
            let saves = saves.clone();
            async move { saves.write("state.g.big", 0, b"other device").await.unwrap() }
        });
        for (i, name) in ["game-a", "game-b", "state.g.index", "state.h.big"].iter().enumerate() {
            let w = tokio::time::timeout(std::time::Duration::from_secs(5), saves.write(name, 0, format!("save {i}").as_bytes())).await;
            assert!(w.expect("waited for another name's upload").unwrap(), "{name}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!big.is_finished() && !same.is_finished(), "the same name waits for its writer");
        drop(gate);
        assert!(big.await.unwrap());
        assert!(!same.await.unwrap(), "revision 1 now: the second writer's 0 is stale");
        assert_eq!(saves.read("state.g.big").await.unwrap().unwrap(), (1, vec![7u8; 2 << 20]));
        assert_eq!(saves.read("game-b").await.unwrap().unwrap(), (1, b"save 1".to_vec()));
        assert_eq!(saves.lock.held(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A writer given up while it waits (the page's request dropped) takes nothing with it: the
    /// name is free once its holder goes, and no lock is left.
    #[tokio::test]
    async fn a_dropped_waiter_leaves_no_lock() {
        let locks = crate::saves::NameLocks::default();
        let held = locks.lock("a").await;
        let waiter = locks.lock("a");
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), waiter).await.is_err());
        assert_eq!(locks.held(), 1);
        drop(held);
        assert_eq!(locks.held(), 0);
        let again = tokio::time::timeout(std::time::Duration::from_secs(1), locks.lock("a")).await.unwrap();
        drop(again);
        assert_eq!(locks.held(), 0);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]
        /// Writers of a few names at once, each with the revision it last saw, as devices do:
        /// per name, the writes that succeeded are exactly one per revision (no lost update,
        /// no two writers of one revision), the save read back is one of them (the last), and
        /// no lock is left.
        #[test]
        fn concurrent_writers_keep_each_name_linear(plan in proptest::collection::vec((0usize..3, 1usize..4), 1..10)) {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap();
            rt.block_on(async {
                let (_f, s, dir) = serve(1).await;
                let saves = saves_on(s);
                let mut jobs = tokio::task::JoinSet::new();
                for (k, (name, tries)) in plan.iter().copied().enumerate() {
                    let saves = saves.clone();
                    jobs.spawn(async move {
                        let name = format!("game-{name}");
                        let mut won = Vec::new();
                        for t in 0..tries {
                            let seen = saves.read(&name).await.unwrap().map_or(0, |r| r.0);
                            let data = format!("{k}.{t}");
                            if saves.write(&name, seen, data.as_bytes()).await.unwrap() {
                                won.push((name.clone(), seen + 1, data));
                            }
                        }
                        won
                    });
                }
                let mut won: std::collections::BTreeMap<String, Vec<(i64, String)>> = Default::default();
                while let Some(r) = jobs.join_next().await {
                    for (n, rev, d) in r.unwrap() {
                        won.entry(n).or_default().push((rev, d));
                    }
                }
                for (name, mut ws) in won {
                    ws.sort();
                    let revs: Vec<i64> = ws.iter().map(|w| w.0).collect();
                    assert_eq!(revs, (1..=ws.len() as i64).collect::<Vec<_>>(), "{name}");
                    let last = ws.last().unwrap();
                    assert_eq!(saves.read(&name).await.unwrap().unwrap(), (last.0, last.1.clone().into_bytes()));
                }
                assert_eq!(saves.lock.held(), 0);
                let _ = std::fs::remove_dir_all(dir);
            });
        }
    }

    /// Pruning state files reads the indexes with no lock held; an index written meanwhile
    /// (naming a file the old one did not) keeps its game's files, until the next prune.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_index_written_while_pruning_keeps_its_files() {
        let (_f, s, dir) = serve(30).await;
        let saves = saves_on(s);
        let hash = |i: i64| format!("{i:032x}");
        for i in 0..3 {
            assert!(saves.write(&format!("state.g.{}", hash(i)), 0, b"state").await.unwrap());
        }
        let index = |i: i64| format!(r#"{{"states":[{{"slot":0,"hash":"{}","created":{i}}}]}}"#, hash(i));
        assert!(saves.write("state.g.index", 0, index(0).as_bytes()).await.unwrap());
        saves.db.connect().unwrap().execute("UPDATE saves SET written_ms=written_ms-?", [8 * 86_400_000i64]).unwrap();
        // (the prune reads the index from the bucket, 30 ms a request; the device writes a new
        // index naming file 1 at once)
        let prune = tokio::spawn({
            let saves = saves.clone();
            async move { saves.prune_states(crate::saves::STATE_GRACE).await.unwrap() }
        });
        assert!(saves.write("state.g.index", 1, index(1).as_bytes()).await.unwrap());
        assert_eq!(prune.await.unwrap(), 0, "the index moved on: nothing of g goes");
        for i in 0..3 {
            assert!(saves.read(&format!("state.g.{}", hash(i))).await.unwrap().is_some(), "file {i}");
        }
        // next time: what neither kept revision names (file 2) goes
        assert_eq!(saves.prune_states(crate::saves::STATE_GRACE).await.unwrap(), 1);
        assert!(saves.read(&format!("state.g.{}", hash(2))).await.unwrap().is_none());
        assert_eq!(saves.lock.held(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review D-1: a bucket that never answers (each request outlasts the budget) gives up
    /// within GET_BUDGET, not after every try again; the request's permit is free again and a
    /// later read of something cached answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_read_from_a_hung_bucket_gives_up_within_its_budget() {
        let (f, s, dir) = serve(0).await;
        s.put("chunks/a", b"first").await.unwrap();
        s.put("chunks/b", b"second").await.unwrap();
        assert_eq!(&*s.get("chunks/a").await.unwrap().unwrap(), b"first");
        // (the same bucket, now 4x the budget per answer)
        let slow = Fake { delay_ms: 4 * GET_BUDGET.as_millis() as u64, ..f.clone() };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        async fn read(State(f): State<Fake>, UrlPath(p): UrlPath<String>) -> axum::response::Response {
            tokio::time::sleep(std::time::Duration::from_millis(f.delay_ms)).await;
            f.objects.locked().get(&p).cloned().map(IntoResponse::into_response).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
        }
        let app = Router::new().route("/{*path}", get(read)).with_state(slow);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let bucket = Bucket::new("test", Region::Custom { region: "us-east-1".into(), endpoint }, Credentials::new(Some("t"), Some("t"), None, None, None).unwrap()).unwrap().with_path_style();
        // (one request at a time, the cache of the reads so far kept)
        let one = Shared::new(1, s.shared.disk.clone());
        *one.cache.locked() = std::mem::take(&mut *s.shared.cache.locked());
        let s = Storage { bucket, shared: one, ..s };
        let t0 = std::time::Instant::now();
        let e = s.get("chunks/b").await.unwrap_err();
        let took = t0.elapsed();
        assert!(e.to_string().contains("no answer within"), "{e:#}");
        assert!(took >= GET_BUDGET && took < GET_BUDGET + std::time::Duration::from_secs(2), "{took:?}");
        assert_eq!(s.shared.requests.available_permits(), 1, "the permit came back");
        assert_eq!(&*s.get("chunks/a").await.unwrap().unwrap(), b"first", "cached: no request");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Saves keep their last SAVE_REVISIONS revisions in S3.
    #[tokio::test]
    async fn old_save_revisions_are_retired() {
        let (f, s, dir) = serve(0).await;
        let db = s.db.clone();
        let saves = crate::saves::Saves::open(db, Some(s)).unwrap();
        for rev in 0..20 {
            assert!(saves.write("game", rev, format!("save {rev}").as_bytes()).await.unwrap());
        }
        assert_eq!(saves.read("game").await.unwrap().unwrap(), (20, b"save 19".to_vec()));
        let s = saves.remote.as_ref().unwrap();
        let rows: i64 = s.db.connect().unwrap().query_row("SELECT COUNT(*) FROM objects WHERE logical LIKE 'saves/game/%'", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, SAVE_REVISIONS);
        age_retired(s, 7_200_000);
        s.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
        assert_eq!(names(&f).len() as i64, SAVE_REVISIONS);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review b01: the save is stored once its row is; old revisions that cannot be forgotten
    /// (here a trigger refuses every delete of objects) are logged, the write still answers
    /// yes and the next one with the new revision goes through (a 502 here made the client's
    /// next PUT, with the old revision, a 412).
    #[tokio::test]
    async fn a_failed_prune_is_no_failed_save() {
        let (_f, s, dir) = serve(0).await;
        let saves = saves_on(s);
        saves.remote.as_ref().unwrap().db.connect().unwrap().execute_batch("CREATE TRIGGER no_forget BEFORE DELETE ON objects BEGIN SELECT RAISE(FAIL, 'refused'); END;").unwrap();
        for rev in 0..SAVE_REVISIONS + 4 {
            assert!(saves.write("game", rev, format!("save {rev}").as_bytes()).await.unwrap(), "rev {rev}");
        }
        let last = SAVE_REVISIONS + 4;
        assert_eq!(saves.read("game").await.unwrap().unwrap(), (last, format!("save {}", last - 1).into_bytes()));
        assert!(!saves.write("game", last - 1, b"stale").await.unwrap(), "the revision check still holds");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Review b01: every Storage the server opens on one configuration (saves, store API,
    /// tree files) shares one disk cache (walked once, one byte count), one plaintext cache,
    /// one download gate and one request cap; another database or configuration has its own.
    #[tokio::test]
    async fn storages_on_one_configuration_share_their_caches() {
        let dir = std::env::temp_dir().join(format!("masu-shared-storage-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = |name: &str, cache: &str| {
            let p = dir.join(name);
            std::fs::write(&p, format!("bucket = \"b\"\nregion = \"us-east-1\"\nendpoint = \"http://127.0.0.1:9\"\nprefix = \"\"\nactive_key = \"k\"\ncache = {:?}\ncache_gib = 1\n[keys]\nk = \"{}\"\n", dir.join(cache), "03".repeat(32))).unwrap();
            p
        };
        let (c1, c2) = (cfg("a.toml", "s3a"), cfg("b.toml", "s3b"));
        let lib = |n: &str| Library::new(&dir.join(n)).unwrap();
        let a = Storage::open(lib("store"), &c1, test_credentials()).unwrap();
        let b = Storage::open(lib("store"), &c1, test_credentials()).unwrap();
        let other_db = Storage::open(lib("store2"), &c1, test_credentials()).unwrap();
        let other_cfg = Storage::open(lib("store"), &c2, test_credentials()).unwrap();
        assert!(Arc::ptr_eq(&a.shared, &b.shared));
        assert!(Arc::ptr_eq(a.shared.disk.as_ref().unwrap(), b.shared.disk.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(&a.shared, &other_db.shared) && !Arc::ptr_eq(&a.shared, &other_cfg.shared));
        assert!(!Arc::ptr_eq(a.shared.disk.as_ref().unwrap(), other_cfg.shared.disk.as_ref().unwrap()));
        // (the cap is one: what one holds, the other cannot take)
        let held: Vec<_> = (0..16).map(|_| a.shared.requests.try_acquire().unwrap()).collect();
        assert!(b.shared.requests.try_acquire().is_err());
        assert!(other_db.shared.requests.try_acquire().is_ok());
        drop(held);
        assert!(b.shared.requests.try_acquire().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(32))]
        /// Any order of opens on two databases and two configurations: two Storages share their
        /// caches and request cap exactly when both database and configuration are the same
        /// (with the test's keys: no lookup of the machine's, #242)
        #[test]
        fn sharing_is_by_database_and_configuration(opens in proptest::collection::vec((0usize..2, 0usize..2), 1..12)) {
            let dir = std::env::temp_dir().join(format!("masu-share-pbt-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let cfgs: Vec<_> = (0..2).map(|i| {
                let p = dir.join(format!("{i}.toml"));
                std::fs::write(&p, format!("bucket = \"b\"\nregion = \"us-east-1\"\nendpoint = \"http://127.0.0.1:9\"\nactive_key = \"k\"\ncache = {:?}\ncache_gib = 1\n[keys]\nk = \"{}\"\n", dir.join(format!("s3-{i}")), "04".repeat(32))).unwrap();
                p
            }).collect();
            let opened: Vec<((usize, usize), Storage)> = opens.iter().map(|&(d, c)| ((d, c), Storage::open(Library::new(&dir.join(format!("db{d}"))).unwrap(), &cfgs[c], test_credentials()).unwrap())).collect();
            for (ka, a) in &opened {
                for (kb, b) in &opened {
                    proptest::prop_assert_eq!(Arc::ptr_eq(&a.shared, &b.shared), ka == kb);
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Review b01: a read dropped at any point (its client went away: axum drops the handler)
    /// leaves no download gate behind; reads that finish leave none either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_read_lets_go_of_its_gate() {
        let (_f, s, dir) = serve(40).await;
        for i in 0..6 {
            s.put(&format!("chunks/ab/{i}"), format!("object {i}").as_bytes()).await.unwrap();
        }
        let s = Arc::new(s);
        for (i, after) in [0u64, 2, 5, 15, 30, 60].into_iter().enumerate() {
            let task = tokio::spawn({
                let s = s.clone();
                async move { s.get(&format!("chunks/ab/{i}")).await }
            });
            tokio::time::sleep(std::time::Duration::from_millis(after)).await;
            task.abort();
            let _ = task.await;
            assert_eq!(s.gates(), 0, "dropped after {after} ms");
        }
        assert_eq!(&*s.get("chunks/ab/0").await.unwrap().unwrap(), b"object 0");
        assert_eq!(s.gates(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { cases: 32, ..Default::default() })]
        /// direct_many answers, in the order asked, for the names the store has (duplicates
        /// too), exactly what direct answers for each alone.
        #[test]
        fn direct_many_is_direct_of_each(ask in proptest::collection::vec(0usize..8, 0..20)) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let (_f, s, dir) = serve(0).await;
                let names: Vec<String> = (0..8).map(|i| format!("chunks/ab/{i:032x}")).collect();
                for n in &names[..5] {
                    s.put(n, n.as_bytes()).await.unwrap();
                }
                s.pack_objects("g", &names[..2], 1 << 20).await.unwrap();
                let asked: Vec<String> = ask.iter().map(|&i| names[i].clone()).collect();
                let many = s.direct_many(asked.clone(), 60).await.unwrap();
                let want: Vec<&String> = asked.iter().filter(|n| names[..5].contains(n)).collect();
                assert_eq!(many.iter().map(|m| &m.0).collect::<Vec<_>>(), want);
                for (n, d) in &many {
                    let one = s.direct(n, 60).await.unwrap().unwrap();
                    assert_eq!((d.object, d.key, d.pack.as_ref().map(|p| (p.id, p.offset, p.len))), (one.object, one.key, one.pack.as_ref().map(|p| (p.id, p.offset, p.len))));
                }
                let _ = std::fs::remove_dir_all(dir);
            });
        }
    }

    /// A store with one game ("sample", one tree) and its save written, in the Storage's dir.
    async fn package_store(saves: &crate::saves::Saves, dir: &std::path::Path) -> Vec<u8> {
        std::fs::create_dir_all(dir.join("games")).unwrap();
        std::fs::create_dir_all(dir.join("trees")).unwrap();
        let g = serde_json::json!({"id":"sample","run":"Z:\\game\\game.exe","mounts":["/game=sample"]});
        std::fs::write(dir.join("games/sample.json"), serde_json::to_vec(&g).unwrap()).unwrap();
        std::fs::write(dir.join("trees/sample.tsv"), "vnvm-tree\t1\t262144\nf\t0\t0\tgame.exe\t1\tabcd\n").unwrap();
        let name = crate::portable::save_name_of(&g).unwrap();
        assert!(saves.write(&name, 0, b"progress").await.unwrap());
        crate::portable::export(dir, "sample", saves).await.unwrap()
    }

    /// `package` with its manifest changed by `edit`.
    fn repackage(package: &[u8], edit: impl Fn(&mut serde_json::Value)) -> Vec<u8> {
        use std::io::{Read, Write};
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(package)).unwrap();
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for i in 0..z.len() {
            let mut e = z.by_index(i).unwrap();
            let name = e.name().to_string();
            let mut data = vec![];
            e.read_to_end(&mut data).unwrap();
            if name == "manifest.json" {
                let mut m: serde_json::Value = serde_json::from_slice(&data).unwrap();
                edit(&mut m);
                data = serde_json::to_vec(&m).unwrap();
            }
            out.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            out.write_all(&data).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// Review b01: a package refused for anything in its manifest (here: settings that are not
    /// strings) is refused before a save goes to the bucket: no object is left behind.
    #[tokio::test]
    async fn a_refused_package_leaves_nothing_in_the_bucket() {
        let (f, s, dir) = serve(0).await;
        let saves = saves_on(s);
        let package = package_store(&saves, &dir).await;
        let before = names(&f);
        for bad in [serde_json::json!([1]), serde_json::json!("x"), serde_json::json!([null])] {
            for field in ["args", "env"] {
                let p = repackage(&package, |m| m["game"][field] = bad.clone());
                let e = crate::portable::import(&dir, "sample", p.into(), &saves).await.unwrap_err();
                assert!(e.to_string().contains("invalid game settings"), "{e}");
                assert_eq!(names(&f), before, "{field} = {bad}: objects written");
            }
        }
        // (and the package itself goes in: two saves' new revisions)
        crate::portable::import(&dir, "sample", package.into(), &saves).await.unwrap();
        assert_eq!(names(&f).len(), before.len() + 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig { cases: 24, ..Default::default() })]
        /// Any manifest edit: the import goes in (objects for both saves) or is refused with
        /// the bucket as it was.
        #[test]
        fn a_package_goes_in_whole_or_not_at_all(field in proptest::sample::select(vec!["args", "env", "mounts", "run", "version", "note", "sessions", "fingerprints"]), v in proptest::prop_oneof![
            proptest::prelude::Just(serde_json::json!([])),
            proptest::prelude::Just(serde_json::json!(["a"])),
            proptest::prelude::Just(serde_json::json!([1])),
            proptest::prelude::Just(serde_json::json!("x")),
            proptest::prelude::Just(serde_json::json!(null)),
            proptest::prelude::Just(serde_json::json!(["/game=sample"])),
            proptest::prelude::Just(serde_json::json!(["/../x=sample"])),
            proptest::prelude::Just(serde_json::json!(2)),
        ]) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let (f, s, dir) = serve(0).await;
                let saves = saves_on(s);
                let package = package_store(&saves, &dir).await;
                let before = names(&f);
                let p = repackage(&package, |m| match field {
                    "version" => m["version"] = v.clone(),
                    "note" => m["profile"]["note"] = v.clone(),
                    "sessions" => m["sessions"] = v.clone(),
                    "fingerprints" => m["fingerprints"] = v.clone(),
                    _ => m["game"][field] = v.clone(),
                });
                match crate::portable::import(&dir, "sample", p.into(), &saves).await {
                    Ok(()) => assert_eq!(names(&f).len(), before.len() + 2),
                    Err(_) => assert_eq!(names(&f), before),
                }
                let _ = std::fs::remove_dir_all(dir);
            });
        }
    }

    /// Save states with S3 (issue #90): the index of a slot saved again 12 times, each time a
    /// new file; the files the kept revisions of the index ([`SAVE_REVISIONS`]) name stay (a
    /// device may still list one of them), the older ones are forgotten and, after the grace
    /// period, deleted from the bucket; every file still named reads back.
    #[tokio::test]
    async fn unnamed_state_files_leave_the_bucket() {
        let (f, s, dir) = serve(0).await;
        let db = s.db.clone();
        let saves = crate::saves::Saves::open(db, Some(s)).unwrap();
        let hash = |i: i64| format!("{i:032x}");
        for i in 0..12 {
            assert!(saves.write(&format!("state.g.{}", hash(i)), 0, format!("state {i}").as_bytes()).await.unwrap());
            let index = format!(r#"{{"states":[{{"slot":0,"hash":"{}","created":{i}}}]}}"#, hash(i));
            assert!(saves.write("state.g.index", i, index.as_bytes()).await.unwrap());
        }
        saves.db.connect().unwrap().execute("UPDATE saves SET written_ms=written_ms-?", [8 * 86_400_000i64]).unwrap();
        let before = names(&f).len();
        assert_eq!(saves.prune_states(crate::saves::STATE_GRACE).await.unwrap(), 12 - SAVE_REVISIONS as usize);
        for i in 0..12 {
            let kept = i >= 12 - SAVE_REVISIONS;
            assert_eq!(saves.read(&format!("state.g.{}", hash(i))).await.unwrap().is_some(), kept, "file {i}");
        }
        let s = saves.remote.as_ref().unwrap();
        // (forgotten, not yet deleted: retired for the grace period)
        assert_eq!(names(&f).len(), before);
        age_retired(s, 7_200_000);
        let r = s.gc(std::time::Duration::from_secs(3600), None).await.unwrap();
        // (the files, and the index's revisions older than the kept ones)
        assert_eq!(r.deleted, 2 * (12 - SAVE_REVISIONS as usize));
        assert_eq!(names(&f), named(s));
        *s.shared.cache.locked() = Default::default();
        for i in 12 - SAVE_REVISIONS..12 {
            assert_eq!(saves.read(&format!("state.g.{}", hash(i))).await.unwrap().unwrap().1, format!("state {i}").into_bytes());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// push_store uploads several at once (A/B: 64 chunks against a bucket answering in 40 ms
    /// each); chunks before trees before games; one pushed is not pushed again.
    #[tokio::test]
    async fn push_store_in_parallel() {
        let (f, s, dir) = serve(40).await;
        let store = dir.join("store");
        for i in 0..64 {
            let d = store.join(format!("chunks/{:02x}", i % 4));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("c{i}")), [i as u8; 100]).unwrap();
        }
        std::fs::create_dir_all(store.join("trees")).unwrap();
        std::fs::write(store.join("trees/t.tsv"), "tree").unwrap();
        std::fs::create_dir_all(store.join("games")).unwrap();
        std::fs::write(store.join("games/g.json"), "game").unwrap();
        let t = std::time::Instant::now();
        s.push_store(&store).await.unwrap();
        let took = t.elapsed();
        eprintln!("push_store: 66 files at 40 ms each in {took:?} (one at a time: {} ms)", 66 * 40);
        assert!(f.most.load(SeqCst) >= 4, "{}", f.most.load(SeqCst));
        assert!(took < std::time::Duration::from_millis(66 * 40 / 2));
        assert_eq!(names(&f).len(), 66);
        let order: Vec<i64> = ["chunks/00/c0", "trees/t.tsv", "games/g.json"]
            .iter()
            .map(|l| s.db.connect().unwrap().query_row("SELECT rowid FROM objects WHERE logical=?", [l], |r| r.get(0)).unwrap())
            .collect();
        assert!(order[0] < order[1] && order[1] < order[2], "{order:?}");
        let max_chunk: i64 = s.db.connect().unwrap().query_row("SELECT MAX(rowid) FROM objects WHERE logical LIKE 'chunks/%'", [], |r| r.get(0)).unwrap();
        assert!(max_chunk < order[1]);
        s.push_store(&store).await.unwrap();
        assert_eq!(names(&f).len(), 66 + 2, "only trees and games go again");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The server's store has game a, a save and a favorite; another machine's store (the same
    /// bucket and key, its own database) adds a again (the same files) and b, a newer revision
    /// of the save and a play session, and backs up. The server merges that backup: b shows up
    /// (entry, tree, every chunk reads back), a's chunks stay the server's objects and the other
    /// copies are retired, a's catalog files are the other's, the save is at the newer revision,
    /// the favorite stays, the session is added. gc then leaves the bucket holding what the
    /// server names plus the other machine's backup; a second merge adds nothing.
    #[tokio::test]
    async fn merge_another_stores_backup() {
        let (f, s, dir) = serve(0).await;
        let pc_db = Library::new(&dir.join("pc")).unwrap();
        let pc = Storage {
            db: pc_db.clone(),
            bucket: s.bucket.clone(),
            config: Config { keys: s.config.keys.clone(), active_key: s.config.active_key.clone(), bucket: "test".into(), region: "us-east-1".into(), endpoint: None, prefix: "".into(), uploads: None, cache: None, cache_gib: None },
            shared: Shared::new(8, None),
        };
        let shared: Vec<u8> = (0..600_000u32).map(|i| (i * 13 % 251) as u8).collect();
        for (name, own) in [("a", 1u8), ("b", 2)] {
            let d = dir.join(format!("files-{name}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("game.exe"), [b'M', b'Z', own]).unwrap();
            std::fs::write(d.join("shared.dat"), &shared).unwrap();
        }
        let (server_store, pc_store) = (dir.join("server"), dir.join("pc"));
        crate::pack::add_for_test(&server_store, &dir.join("files-a"), "a", vec![]).unwrap();
        s.push_store(&server_store).await.unwrap();
        s.db.set_profile("a", &crate::library::Profile { favorite: true, note: "server".into() }).unwrap();
        let saves = |db: &Library, rev: i64| {
            let c = db.connect().unwrap();
            c.execute_batch("CREATE TABLE IF NOT EXISTS saves(name TEXT PRIMARY KEY, revision INTEGER NOT NULL, data BLOB);").unwrap();
            c.execute("INSERT INTO saves(name,revision,data) VALUES('a',?,NULL) ON CONFLICT(name) DO UPDATE SET revision=excluded.revision", [rev]).unwrap();
        };
        s.put("saves/a/1", b"save 1").await.unwrap();
        saves(&s.db, 1);
        crate::pack::add_for_test(&pc_store, &dir.join("files-a"), "a", vec![]).unwrap();
        crate::pack::add_for_test(&pc_store, &dir.join("files-b"), "b", vec![]).unwrap();
        pc.push_store(&pc_store).await.unwrap();
        pc.put("saves/a/2", b"save 2").await.unwrap();
        saves(&pc_db, 2);
        pc_db.connect().unwrap().execute("INSERT INTO sessions(id,game,started_ms,elapsed_ms) VALUES('s1','b',1,60000)", []).unwrap();
        pc.db.set_profile("a", &crate::library::Profile { favorite: false, note: "pc".into() }).unwrap();
        let server_a_chunk = |l: &str| s.db.connect().unwrap().query_row("SELECT envelope FROM objects WHERE logical=?", [l], |r| r.get::<_, String>(0)).unwrap();
        let a_chunks: Vec<String> = s.db.connect().unwrap().prepare("SELECT logical FROM objects WHERE logical LIKE 'chunks/%'").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let before: Vec<String> = a_chunks.iter().map(|l| server_a_chunk(l)).collect();
        // (the other machine packed both games: a's chunks are other objects here, b's the same)
        let pc_chunks: Vec<String> = pc_db.connect().unwrap().prepare("SELECT logical FROM objects WHERE logical LIKE 'chunks/%' ORDER BY logical").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert!(pc.pack_objects("ab", &pc_chunks, 1).await.unwrap().packs >= 2);
        let backup = pc.backup().await.unwrap();

        let m = s.merge(backup, &server_store).await.unwrap();
        // (spans taken for objects the server names as the other did, none for a's own copies)
        {
            let c = s.db.connect().unwrap();
            let taken: i64 = c.query_row("SELECT COUNT(*) FROM packs", [], |r| r.get(0)).unwrap();
            assert_eq!(taken as usize, m.packs);
            let same: i64 = c.query_row("SELECT COUNT(*) FROM packs p JOIN objects o USING(logical) WHERE instr(o.envelope,p.object) > 0", [], |r| r.get(0)).unwrap();
            assert_eq!(same, taken, "every span taken is of the object named here");
            let pc_c = pc_db.connect().unwrap();
            let same_objects = pc_chunks.iter().filter(|l| {
                let theirs: String = pc_c.query_row("SELECT envelope FROM objects WHERE logical=?", [l], |r| r.get(0)).unwrap();
                let ours: Option<String> = c.query_row("SELECT envelope FROM objects WHERE logical=?", [l], |r| r.get(0)).optional().unwrap();
                ours.is_some_and(|o| serde_json::from_str::<Envelope>(&o).unwrap().object == serde_json::from_str::<Envelope>(&theirs).unwrap().object)
            }).count();
            assert_eq!(m.packs, same_objects);
            assert!(m.packs > 0 && m.packs < pc_chunks.len(), "{m:?}");
            for l in a_chunks.iter() {
                assert!(s.direct(l, 60).await.unwrap().unwrap().pack.is_none(), "a's chunk {l}: the server's own object, in no pack");
            }
        }
        check_packs(&f, &s);
        assert!(m.added > 0 && m.replaced >= 2 && m.kept >= a_chunks.len(), "{m:?}");
        assert_eq!((m.saves, m.records), (1, 1), "{m:?}");
        assert!(server_store.join("games/b.json").exists() && server_store.join("trees/b.tsv").exists());
        assert_eq!(a_chunks.iter().map(|l| server_a_chunk(l)).collect::<Vec<_>>(), before, "a's chunks stay the server's objects");
        *s.shared.cache.locked() = Default::default();
        for line in std::fs::read_to_string(server_store.join("trees/b.tsv")).unwrap().lines().filter(|l| l.starts_with("f\t")) {
            for c in line.split('\t').nth(5).unwrap().split(',').map(|c| c.split(':').next().unwrap()).filter(|c| c.len() > 2) {
                assert!(s.get(&format!("chunks/{}/{c}", &c[..2])).await.unwrap().is_some(), "b's chunk {c}");
            }
        }
        let rev: i64 = s.db.connect().unwrap().query_row("SELECT revision FROM saves WHERE name='a'", [], |r| r.get(0)).unwrap();
        assert_eq!(rev, 2);
        assert_eq!(&*s.get("saves/a/2").await.unwrap().unwrap(), b"save 2");
        assert_eq!(s.db.profile("a").unwrap().note, "server");
        let sessions: i64 = s.db.connect().unwrap().query_row("SELECT COUNT(*) FROM sessions WHERE id='s1'", [], |r| r.get(0)).unwrap();
        assert_eq!(sessions, 1);

        s.gc(std::time::Duration::ZERO, None).await.unwrap();
        let pc_backup: std::collections::BTreeSet<String> = {
            let snapshot: String = pc_db.connect().unwrap().query_row("SELECT snapshot FROM backups", [], |r| r.get(0)).unwrap();
            [backup.to_string(), snapshot].into()
        };
        let packs: std::collections::BTreeSet<String> = s.db.connect().unwrap().prepare("SELECT DISTINCT pack FROM packs").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let mut want: std::collections::BTreeSet<String> = named(&s).union(&pc_backup).cloned().collect();
        want.extend(packs);
        want.extend(named(&pc));
        want.extend(pc_db.connect().unwrap().prepare("SELECT DISTINCT pack FROM packs").unwrap()
            .query_map([], |r| r.get::<_, String>(0)).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap());
        assert_eq!(names(&f), want, "the bucket: what the server names, its packs, and the other's backup");
        check_packs(&f, &s);
        let restored = dir.join("source-restored.db");
        pc.restore(backup, &restored).await.unwrap();
        let mut source = pc;
        let logicals: Vec<String> = source.db.connect().unwrap().prepare("SELECT logical FROM objects").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        source.db = Library(restored);
        source.shared = Shared::new(8, None);
        for logical in logicals {
            assert!(source.get(&logical).await.unwrap().is_some(), "{logical}");
        }
        let again = s.merge(backup, &server_store).await.unwrap();
        assert_eq!((again.added, again.packs), (0, 0), "{again:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Two games pushed, with chunks in common: removing one drops its entry, tree and own
    /// chunks from the store's rows (the other's all stay and read back); a backup taken before
    /// keeps the objects in the bucket until it goes; then gc deletes exactly them, and a fresh
    /// server's catalog has only the other game.
    #[tokio::test]
    async fn removed_game_leaves_the_bucket() {
        let (f, s, dir) = serve(0).await;
        let store = dir.join("store");
        let shared: Vec<u8> = (0..600_000u32).map(|i| (i * 13 % 251) as u8).collect();
        for (name, own) in [("a", 1u8), ("b", 2)] {
            let d = dir.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("game.exe"), [b'M', b'Z', own]).unwrap();
            std::fs::write(d.join("own.dat"), vec![own; 300_000]).unwrap();
            std::fs::write(d.join("shared.dat"), &shared).unwrap();
            crate::pack::add_for_test(&store, &d, name, vec![]).unwrap();
        }
        s.push_store(&store).await.unwrap();
        let mut b_files: Vec<String> = vec!["games/b.json".into(), "trees/b.tsv".into()];
        for line in std::fs::read_to_string(store.join("trees/b.tsv")).unwrap().lines().filter(|l| l.starts_with("f\t")) {
            b_files.extend(line.split('\t').nth(5).unwrap().split(',').map(|c| c.split(':').next().unwrap()).filter(|c| c.len() > 2).map(|c| format!("chunks/{}/{c}", &c[..2])));
        }
        let backup = s.backup().await.unwrap();
        let before = names(&f);

        let gone = crate::pack::remove_game(&store, "a", Some(&s)).await.unwrap();
        assert!(gone.iter().any(|l| l.starts_with("chunks/")) && gone.contains(&"trees/a.tsv".into()) && gone.contains(&"games/a.json".into()), "{gone:?}");
        for l in &gone {
            assert!(s.get(l).await.unwrap().is_none(), "{l} still named");
            assert!(!store.join(l).exists(), "{l} still here");
        }
        // what b uses: chunks a shared are not among them, and all read back
        for l in &b_files {
            assert!(!gone.contains(l), "{l} is b's");
            assert!(s.get(l).await.unwrap().is_some(), "{l}");
        }

        age_retired(&s, 30 * 86_400_000);
        let r = s.gc(std::time::Duration::from_secs(86_400), None).await.unwrap();
        assert_eq!((r.deleted, names(&f)), (0, before.clone()), "the backup from before still needs them");
        let r = s.gc(std::time::Duration::from_secs(86_400), Some(0)).await.unwrap();
        assert_eq!(r.backups_dropped, 1);
        assert_eq!(r.deleted, gone.len());
        assert_eq!(names(&f), named(&s));
        assert_eq!(before.len() - names(&f).len(), gone.len() + 2, "a's objects and the backup's two");
        assert!(!names(&f).contains(&backup.to_string()));

        let fresh = dir.join("fresh");
        s.hydrate_catalog(&fresh).await.unwrap();
        assert!(fresh.join("games/b.json").exists() && fresh.join("trees/b.tsv").exists());
        assert!(!fresh.join("games/a.json").exists() && !fresh.join("trees/a.tsv").exists(), "a does not come back");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A bucket that turns requests away with 503 (as Hugging Face Buckets do under load):
    /// push_store still uploads the whole store, every object reads back, and gc's deletes go
    /// through; the refused requests were tried again, not skipped.
    #[tokio::test]
    async fn push_store_rides_out_503() {
        let (f, s, dir) = serve(0).await;
        let store = dir.join("store");
        for i in 0..64 {
            let d = store.join(format!("chunks/{:02x}", i % 4));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("c{i}")), [i as u8; 100]).unwrap();
        }
        std::fs::create_dir_all(store.join("trees")).unwrap();
        std::fs::write(store.join("trees/t.tsv"), "tree").unwrap();
        std::fs::create_dir_all(store.join("games")).unwrap();
        std::fs::write(store.join("games/g.json"), "game").unwrap();
        // a burst at the start (more than PUTS at once, so every upload sees one), then every third
        f.fail_next.store(PUTS * 2, SeqCst);
        f.fail_every.store(3, SeqCst);
        s.push_store(&store).await.unwrap();
        assert_eq!(names(&f).len(), 66);
        assert!(f.requests.load(SeqCst) > 66 + PUTS * 2, "{}", f.requests.load(SeqCst));
        for i in 0..64 {
            let logical = format!("chunks/{:02x}/c{i}", i % 4);
            assert_eq!(&*s.get(&logical).await.unwrap().unwrap(), &[i as u8; 100][..], "{logical}");
        }
        assert_eq!(&*s.get("games/g.json").await.unwrap().unwrap(), b"game");
        // replaced objects are retired, and gc deletes them through the refusals too
        std::fs::write(store.join("games/g.json"), "game 2").unwrap();
        s.push_store(&store).await.unwrap();
        age_retired(&s, 30 * 86_400_000);
        let r = s.gc(std::time::Duration::from_secs(86_400), None).await.unwrap();
        assert!(r.deleted >= 1, "{r:?}");
        assert_eq!(names(&f), named(&s));
        f.fail_every.store(0, SeqCst);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A bucket that keeps refusing: the upload fails after RETRIES more tries (not forever);
    /// an answer that will not change (403) fails at the first.
    #[tokio::test]
    async fn retries_end() {
        let (f, s, dir) = serve(0).await;
        f.fail_next.store(1000, SeqCst);
        let e = s.put("chunks/x", b"x").await.unwrap_err();
        assert!(e.to_string().contains("503"), "{e}");
        assert_eq!(f.requests.load(SeqCst), RETRIES as usize + 1);
        f.fail_next.store(1000, SeqCst);
        f.fail_status.store(403, SeqCst);
        f.requests.store(0, SeqCst);
        let e = s.put("chunks/x", b"x").await.unwrap_err();
        assert!(e.to_string().contains("403"), "{e}");
        assert_eq!(f.requests.load(SeqCst), 1);
        assert!(names(&f).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// storage.toml's `uploads` sets how many go up at once (a far gateway: the round trip
    /// bounds one upload), and every chunk is named once all is through.
    #[tokio::test]
    async fn uploads_at_once_as_configured() {
        let (f, mut s, dir) = serve(40).await;
        s.config.uploads = Some(40);
        let store = dir.join("store");
        for i in 0..200 {
            let d = store.join(format!("chunks/{:02x}", i % 4));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("c{i}")), (i as u32).to_le_bytes()).unwrap();
        }
        s.push_store(&store).await.unwrap();
        assert!(f.most.load(SeqCst) > PUTS, "{} at once", f.most.load(SeqCst));
        assert_eq!(names(&f).len(), 200);
        assert_eq!(named(&s).len(), 200);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The same object asked for many times at once: one download; the cache holds what it
    /// says it holds.
    #[tokio::test]
    async fn one_download_per_object() {
        let (f, s, dir) = serve(30).await;
        s.put("chunks/x", b"payload").await.unwrap();
        let s = Arc::new(s);
        let tasks: Vec<_> = (0..16).map(|_| { let s = s.clone(); tokio::spawn(async move { s.get("chunks/x").await.unwrap().unwrap() }) }).collect();
        for t in tasks {
            assert_eq!(&*t.await.unwrap(), b"payload");
        }
        assert_eq!(f.gets.load(SeqCst), 1);
        assert!(s.shared.inflight.locked().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A chunk read once comes from the disk afterwards: a restarted server (a new Storage on
    /// the same disk) and a big chunk (past the 1 MiB memory cache) ask the bucket once.
    #[tokio::test]
    async fn disk_keeps_what_the_bucket_gave() {
        let (f, mut s, dir) = serve(0).await;
        let disk = dir.join("s3");
        s.shared = Shared::new(8, Some(Arc::new(Disk::open(disk.clone(), 64 << 20).unwrap())));
        let big: Vec<u8> = (0..3u32 << 19).map(|i| (i * 7 + i / 991) as u8).collect();
        s.put("chunks/big", &big).await.unwrap();
        s.put("chunks/small", b"small").await.unwrap();
        for _ in 0..3 {
            assert_eq!(&*s.get("chunks/big").await.unwrap().unwrap(), &big[..]);
            assert_eq!(&*s.get("chunks/small").await.unwrap().unwrap(), b"small");
        }
        // (the disk write does not hold the answer back: let it land)
        for _ in 0..200 {
            if s.shared.disk.as_ref().unwrap().kept() >= big.len() as u64 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(f.gets.load(SeqCst), 2, "the big chunk once, the small once");
        // a new server on the same disk: nothing from the bucket
        let mut again = Storage { db: s.db.clone(), bucket: s.bucket.clone(), config: Config { keys: s.config.keys.clone(), active_key: "k".into(), bucket: "test".into(), region: "us-east-1".into(), endpoint: None, prefix: "".into(), uploads: None, cache: None, cache_gib: None }, shared: Shared::new(8, None) };
        again.shared = Shared::new(8, Some(Arc::new(Disk::open(disk.clone(), 64 << 20).unwrap())));
        assert_eq!(&*again.get("chunks/big").await.unwrap().unwrap(), &big[..]);
        assert_eq!(&*again.get("chunks/small").await.unwrap().unwrap(), b"small");
        assert_eq!(f.gets.load(SeqCst), 2);
        // kept encrypted, as the bucket has it: no plain game bytes on the disk
        let files = std::fs::read_dir(&disk).unwrap().flatten().flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten()).map(|e| std::fs::read(e.path()).unwrap()).collect::<Vec<_>>();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|b| !b.windows(5).any(|w| w == b"small") && b.len() != big.len()));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Real fake-S3 HTTP + a deliberately stopped disk write: callers get the
    /// large object promptly, and warm/direct do not send it to S3 again.
    #[tokio::test]
    async fn large_reads_share_pending_ciphertext_without_waiting_for_disk() {
        let (f, mut s, dir) = serve(0).await;
        let disk = Arc::new(Disk::open(dir.join("s3"), 64 << 20).unwrap());
        s.shared = Shared::new(8, Some(disk.clone()));
        let big = vec![0x5a; CACHE_ITEM + 4096];
        s.put("chunks/big", &big).await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = Mutex::new(Some(started_tx));
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        *disk.before_write.locked() = Some(Arc::new(move || {
            if let Some(tx) = started_tx.locked().take() { let _ = tx.send(()); }
            // Bounded even when an assertion fails before the writer is released.
            let _ = release_rx.locked().recv_timeout(std::time::Duration::from_secs(5));
        }));
        let deadline = std::time::Duration::from_secs(2);
        let first = tokio::time::timeout(deadline, s.get("chunks/big")).await.expect("first answer waited for disk").unwrap().unwrap();
        assert_eq!(&*first, &big);
        tokio::time::timeout(deadline, started_rx).await.unwrap().unwrap();
        assert_eq!(disk.files().0.len(), 0, "writer should still be stopped before IO");
        assert!(s.shared.cache.locked().map.is_empty(), "large objects stay outside the plaintext cache");
        tokio::time::timeout(deadline, async {
            let reads = futures_util::future::join_all((0..8).map(|_| s.get("chunks/big"))).await;
            for read in reads {
                assert_eq!(&*read.unwrap().unwrap(), &big);
            }
            // (the page is sent to the bucket even so: storage.rs direct)
            assert!(s.direct("chunks/big", 60).await.unwrap().is_some());
        }).await.expect("readers waited for disk instead of sharing the pending bytes");
        assert_eq!(f.gets.load(SeqCst), 1);
        release_tx.send(()).unwrap();
        tokio::time::timeout(deadline, async {
            while !disk.pending.locked().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert_eq!(disk.files().0.len(), 1);
        assert_eq!(&*s.get("chunks/big").await.unwrap().unwrap(), &big);
        assert_eq!(f.gets.load(SeqCst), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// push_trees: what the server made itself reaches the bucket: the chunks the bucket lacks
    /// (each once, those it has not again), then the lists; the page reads them back through
    /// /store-direct as any game's. A chunk neither has fails the push and names nothing.
    #[tokio::test]
    async fn push_trees_puts_what_only_the_disk_has() {
        let (f, s, dir) = serve(0).await;
        let store = dir.join("store");
        let chunk = |k: u32| format!("{:02x}{k:030x}", 0x10 + k);
        let mut tsv = format!("{}\n", crate::tree_list::Header { avg: 64 });
        for k in 0..4u32 {
            let h = chunk(k);
            std::fs::create_dir_all(store.join("chunks").join(&h[..2])).unwrap();
            std::fs::write(store.join("chunks").join(&h[..2]).join(&h), format!("chunk {k}")).unwrap();
            // (the same chunk twice in a tree: sent once)
            tsv += &format!("f\t644\t0\tf{k}\t7\t{h}:7\nf\t644\t0\tcopy{k}\t7\t{h}:7\n");
        }
        tsv += "d\t755\t0\tdir\n";
        std::fs::create_dir_all(store.join("trees")).unwrap();
        std::fs::write(store.join("trees/prefix32.tsv"), &tsv).unwrap();
        // (one the bucket has already)
        s.put(&format!("chunks/10/{}", chunk(0)), b"chunk 0").await.unwrap();
        let before = f.objects.locked().len();
        s.push_trees(&store, &["prefix32"]).await.unwrap();
        assert_eq!(f.objects.locked().len() - before, 3 + 1, "three chunks and the list");
        for k in 0..4u32 {
            let h = chunk(k);
            assert!(s.direct(&format!("chunks/{}/{h}", &h[..2]), 60).await.unwrap().is_some());
            assert_eq!(&*s.get(&format!("chunks/{}/{h}", &h[..2])).await.unwrap().unwrap(), format!("chunk {k}").as_bytes());
        }
        assert_eq!(&*s.get("trees/prefix32.tsv").await.unwrap().unwrap(), tsv.as_bytes());
        // again: only the list
        let before = f.objects.locked().len();
        s.push_trees(&store, &["prefix32"]).await.unwrap();
        assert_eq!(f.objects.locked().len() - before, 1);
        // a chunk on neither side: an error, and the list not sent
        let gone = format!("ff{:030x}", 9);
        std::fs::write(store.join("trees/bench.tsv"), format!("{}\nf\t644\t0\tx\t1\t{gone}:1\n", crate::tree_list::Header { avg: 64 })).unwrap();
        let e = s.push_trees(&store, &["bench"]).await.unwrap_err();
        assert!(format!("{e:#}").contains(&gone), "{e:#}");
        assert!(!s.has("trees/bench.tsv").unwrap());
        assert!(s.push_trees(&store, &["none"]).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn failed_background_write_releases_pending_memory() {
        let dir = std::env::temp_dir().join(format!("masu-pending-failure-{}", Uuid::new_v4()));
        let disk = Arc::new(Disk::open(dir.clone(), 4096).unwrap());
        // The configured directory becomes a file, so the real write must fail.
        std::fs::remove_dir(&dir).unwrap();
        std::fs::write(&dir, b"not a directory").unwrap();
        let id = Uuid::new_v4();
        disk.write_later(id, vec![1; 100]);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while !disk.pending.locked().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }).await.unwrap();
        assert!(disk.pending(id).is_none() && !disk.path(id).exists());
        assert_eq!(disk.kept(), 0);
        // Oversized objects never enqueue a write or consume pending space.
        disk.write_later(id, vec![0; 4097]);
        assert!(disk.pending.locked().is_empty());
        std::fs::remove_file(dir).unwrap();
    }

    /// A kept object damaged on the disk (a flipped byte, cut short, emptied) is noticed by its
    /// envelope: read from the bucket again, and kept again whole.
    #[tokio::test]
    async fn damaged_disk_objects_are_read_again() {
        let (f, mut s, dir) = serve(0).await;
        let disk = dir.join("s3");
        s.shared = Shared::new(8, Some(Arc::new(Disk::open(disk.clone(), 64 << 20).unwrap())));
        s.put("chunks/x", b"payload bytes").await.unwrap();
        let object = |s: &Storage| {
            let e: String = s.db.connect().unwrap().query_row("SELECT envelope FROM objects WHERE logical='chunks/x'", [], |r| r.get(0)).unwrap();
            serde_json::from_str::<Envelope>(&e).unwrap().object
        };
        let id = object(&s);
        let path = s.shared.disk.as_ref().unwrap().path(id);
        let wait_kept = |p: std::path::PathBuf| async move {
            for _ in 0..200 {
                if p.exists() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("not kept");
        };
        assert_eq!(&*s.get("chunks/x").await.unwrap().unwrap(), b"payload bytes");
        wait_kept(path.clone()).await;
        let good = std::fs::read(&path).unwrap();
        let mut gets = 1;
        let damages: Vec<Box<dyn Fn(&mut Vec<u8>)>> = vec![Box::new(|b| b[20] ^= 1), Box::new(|b| b.truncate(b.len() - 1)), Box::new(|b| b.clear()), Box::new(|b| *b.last_mut().unwrap() ^= 0x80)];
        for damage in damages {
            let mut b = good.clone();
            damage(&mut b);
            std::fs::write(&path, &b).unwrap();
            *s.shared.cache.locked() = Default::default();
            assert_eq!(&*s.get("chunks/x").await.unwrap().unwrap(), b"payload bytes");
            gets += 1;
            assert_eq!(f.gets.load(SeqCst), gets);
            for _ in 0..200 {
                if std::fs::read(&path).is_ok_and(|b| b == good) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert_eq!(std::fs::read(&path).unwrap(), good, "kept whole again");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A ciphertext damaged in the bucket: a whole copy kept on the disk still answers (the
    /// bucket is not asked); with no copy on the disk (none kept, or no disk cache) the get is an
    /// error, and the damaged bytes are neither kept on the disk nor in memory.
    #[tokio::test]
    async fn damaged_bucket_objects_are_refused() {
        let (f, mut s, dir) = serve(0).await;
        let disk = dir.join("s3");
        s.shared = Shared::new(8, Some(Arc::new(Disk::open(disk.clone(), 64 << 20).unwrap())));
        s.put("chunks/x", b"payload bytes").await.unwrap();
        let e: String = s.db.connect().unwrap().query_row("SELECT envelope FROM objects WHERE logical='chunks/x'", [], |r| r.get(0)).unwrap();
        let id = serde_json::from_str::<Envelope>(&e).unwrap().object;
        let path = s.shared.disk.as_ref().unwrap().path(id);
        assert_eq!(&*s.get("chunks/x").await.unwrap().unwrap(), b"payload bytes");
        for _ in 0..200 {
            if path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let good = std::fs::read(&path).unwrap();
        for (i, flip) in [0usize, 12, good.len() / 2, good.len() - 1].into_iter().enumerate() {
            for v in f.objects.locked().values_mut() {
                *v = good.clone();
                v[flip] ^= 1 << (i % 8);
            }
            // the disk's whole copy
            *s.shared.cache.locked() = Default::default();
            let gets = f.gets.load(SeqCst);
            assert_eq!(&*s.get("chunks/x").await.unwrap().unwrap(), b"payload bytes");
            assert_eq!(f.gets.load(SeqCst), gets, "read from the disk");
            // no copy on the disk: the bucket's damaged one is refused, and not kept
            s.shared.disk.as_ref().unwrap().remove(id);
            *s.shared.cache.locked() = Default::default();
            assert!(s.get("chunks/x").await.is_err());
            assert_eq!(f.gets.load(SeqCst), gets + 1);
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            assert!(!path.exists(), "a damaged object kept on the disk");
            assert!(s.shared.cache.locked().get(&id).is_none());
            // no disk cache at all: the same
            let kept = std::mem::replace(&mut s.shared, Shared::new(8, None));
            assert!(s.get("chunks/x").await.is_err());
            s.shared = kept;
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &good).unwrap();
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Random writes and reads past the limit: the disk never holds more than the limit after a
    /// write, what it says it holds is what is there, the least lately read go first, and a
    /// write cut short (a .tmp left by a crash) is cleared when it opens.
    #[test]
    fn disk_bounds_and_order() {
        let dir = std::env::temp_dir().join(format!("masu-disk-{}", Uuid::new_v4()));
        let limit = 10_000u64;
        let d = Disk::open(dir.clone(), limit).unwrap();
        let mut seed = 0x6a09e667f3bcc908u64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let ids: Vec<Uuid> = (0..40).map(|_| Uuid::new_v4()).collect();
        let on_disk = |d: &Disk| d.files().0.iter().map(|f| f.1).sum::<u64>();
        for step in 0..600 {
            let id = ids[rnd(40) as usize];
            if rnd(3) == 0 {
                if let Some(b) = d.read(id) {
                    assert!(b.iter().all(|&x| x == id.as_bytes()[0]));
                }
            } else {
                let n = rnd(1500) as usize + 1;
                d.write(id, &vec![id.as_bytes()[0]; n]);
            }
            assert!(d.kept() <= limit, "step {step}");
            assert_eq!(d.kept(), on_disk(&d), "step {step}");
        }
        // bigger than the whole limit: not kept
        d.write(ids[0], &vec![0; limit as usize + 1]);
        assert!(d.kept() <= limit);
        // least lately read first: read one, write until a trim, the one read stays
        let d = Disk::open(dir.join("order"), 3000).unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        d.write(a, &[1; 1000]);
        d.write(b, &[2; 1000]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(d.read(a).is_some());
        std::thread::sleep(std::time::Duration::from_millis(20));
        d.write(Uuid::new_v4(), &[3; 1500]);
        assert!(d.read(a).is_some() && d.read(b).is_none());
        // a write cut short
        let tmp = d.path(a).with_extension("abc.tmp");
        std::fs::write(&tmp, b"half").unwrap();
        let again = Disk::open(dir.join("order"), 3000).unwrap();
        assert!(!tmp.exists());
        assert_eq!(again.kept(), on_disk(&again));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cache_counts_and_bounds() {
        let mut c = Cache::default();
        let mut seed = 0xbb67ae8584caa73bu64;
        let mut rnd = move |n: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % n
        };
        let ids: Vec<Uuid> = (0..30).map(|_| Uuid::new_v4()).collect();
        for _ in 0..5000 {
            let id = ids[rnd(30) as usize];
            c.put(id, vec![0u8; rnd(2000) as usize].into(), 10_000);
            assert_eq!(c.bytes, c.map.values().map(|d| d.len()).sum::<usize>());
            assert!(c.bytes <= 10_000);
            assert_eq!(c.order.len(), c.map.len());
        }
        c.put(Uuid::new_v4(), vec![0u8; CACHE_ITEM + 1].into(), usize::MAX);
        assert!(c.bytes <= 10_000);
        // the limits are inclusive: an item of exactly CACHE_ITEM, a cache of exactly the limit
        let mut c = Cache::default();
        c.put(Uuid::new_v4(), vec![0u8; CACHE_ITEM].into(), CACHE_ITEM);
        assert_eq!((c.bytes, c.map.len()), (CACHE_ITEM, 1));
        c.put(Uuid::new_v4(), vec![0u8; 1].into(), CACHE_ITEM);
        assert_eq!((c.bytes, c.map.len()), (1, 1), "the oldest goes first");
    }
}

pub struct Storage {
    pub db: Library,
    bucket: Box<Bucket>,
    config: Config,
    /// shared by every Storage the process opened on the same configuration and database
    /// (`Storage::open`): one Arc, so its parts cannot be mixed (Arc audit 2026-11-20)
    shared: std::sync::Arc<Shared>,
}

/// [`Storage::gate`]'s hold.
struct Gate<'a>(&'a Storage, std::sync::Arc<tokio::sync::Mutex<()>>, Uuid);

impl Drop for Gate<'_> {
    fn drop(&mut self) {
        self.0.ungate(self.2, &self.1);
    }
}

/// What the Storages opened on one configuration share (`Storage::open`).
struct Shared {
    cache: std::sync::Mutex<Cache>,
    /// one download per object at a time (the others wait for it and take it from the cache)
    inflight: std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<()>>>>,
    /// bucket requests at once
    requests: tokio::sync::Semaphore,
    /// objects downloaded, kept on the disk (None: storage.toml's cache_gib = 0)
    disk: Option<std::sync::Arc<Disk>>,
}

impl Shared {
    fn new(requests: usize, disk: Option<std::sync::Arc<Disk>>) -> std::sync::Arc<Shared> {
        std::sync::Arc::new(Shared { cache: Default::default(), inflight: Default::default(), requests: tokio::sync::Semaphore::new(requests), disk })
    }
}

/// The configuration of `serve --dev-bucket`'s stand-in bucket (dev_bucket.rs), used instead of
/// MASU_STORAGE_CONFIG once set.
static DEV_CONFIG: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Makes [`Storage::configured`] open the stand-in bucket configured at `path` (once: a second
/// call keeps the first).
pub fn use_dev_config(path: std::path::PathBuf) {
    DEV_CONFIG.get_or_init(|| path);
}

/// By (configuration file, database).
static SHARED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, std::path::PathBuf), std::sync::Arc<Shared>>>> = std::sync::LazyLock::new(Default::default);

/// Objects as the bucket has them (encrypted: [`Storage::get`] decrypts, so checks, them each
/// time), in `<dir>/<2 hex>/<uuid>`, the least lately read dropped once past `limit` bytes.
/// A game's chunks come from the bucket once, not on every start: from Hugging Face a chunk
/// took 1-4 s to arrive, and a start read hundreds of them.
pub(crate) struct Disk {
    dir: std::path::PathBuf,
    limit: u64,
    bytes: std::sync::Mutex<u64>,
    // The write task already owns these ciphertext buffers. Share them until the
    // rename finishes, so a > CACHE_ITEM object never needs a second download.
    pending: std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Arc<Vec<u8>>>>,
    #[cfg(test)]
    before_write: std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
}

/// Disk::trim keeps this share of the limit (a little room, so not every write trims).
const DISK_KEEP: f64 = 0.9;

impl Disk {
    pub(crate) fn open(dir: std::path::PathBuf, limit: u64) -> Result<Self> {
        std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
        let d = Disk { dir, limit, bytes: Default::default(), pending: Default::default(),
            #[cfg(test)] before_write: Default::default(),
        };
        let (files, _) = d.files();
        *d.bytes.locked() = files.iter().map(|f| f.1).sum();
        d.trim();
        Ok(d)
    }

    fn path(&self, id: Uuid) -> std::path::PathBuf {
        let s = id.simple().to_string();
        self.dir.join(&s[..2]).join(s)
    }

    /// Every object kept: (path, size, last read); writes cut short (`.tmp`) are removed.
    fn files(&self) -> (Vec<(std::path::PathBuf, u64, std::time::SystemTime)>, usize) {
        let (mut out, mut temps) = (Vec::new(), 0);
        for sub in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            for f in std::fs::read_dir(sub.path()).into_iter().flatten().flatten() {
                let p = f.path();
                if p.extension().is_some_and(|x| x == "tmp") {
                    let _ = std::fs::remove_file(&p);
                    temps += 1;
                    continue;
                }
                if let Ok(m) = f.metadata() && m.is_file() {
                    out.push((p, m.len(), m.modified().unwrap_or(std::time::UNIX_EPOCH)));
                }
            }
        }
        (out, temps)
    }

    fn pending(&self, id: Uuid) -> Option<std::sync::Arc<Vec<u8>>> {
        self.pending.locked().get(&id).cloned()
    }

    /// Keep the answer independent of disk latency. Pending data shares the
    /// existing buffer rather than copying it or adding it to the plaintext cache.
    fn write_later(self: &std::sync::Arc<Self>, id: Uuid, bytes: Vec<u8>) {
        if bytes.len() as u64 > self.limit { return; }
        let bytes = std::sync::Arc::new(bytes);
        {
            let mut pending = self.pending.locked();
            if pending.contains_key(&id) { return; }
            pending.insert(id, bytes.clone());
        }
        let disk = self.clone();
        tokio::task::spawn_blocking(move || {
            disk.write(id, &bytes);
            // A failed write is not a permanent, unbounded memory cache either.
            disk.pending.locked().remove(&id);
        });
    }

    /// Object `id` when kept (and marked as just read).
    pub(crate) fn read(&self, id: Uuid) -> Option<std::sync::Arc<Vec<u8>>> {
        if let Some(data) = self.pending(id) { return Some(data); }
        use std::io::Read;
        let p = self.path(id);
        // (one open to read it and mark it read: review b01, each read opened the file twice)
        let mut f = std::fs::File::options().read(true).write(true).open(&p).or_else(|_| std::fs::File::open(&p)).ok()?;
        let mut data = Vec::new();
        f.read_to_end(&mut data).ok()?;
        let _ = f.set_modified(std::time::SystemTime::now());
        Some(std::sync::Arc::new(data))
    }

    /// Keeps `bytes` as object `id` (written whole or not at all: a temporary file renamed).
    pub(crate) fn write(&self, id: Uuid, bytes: &[u8]) {
        #[cfg(test)]
        if let Some(hook) = self.before_write.locked().clone() { hook(); }
        if bytes.len() as u64 > self.limit {
            return;
        }
        let p = self.path(id);
        let tmp = p.with_extension(format!("{}.tmp", Uuid::new_v4().simple()));
        // (kept already: replaced, counted once)
        let old = std::fs::metadata(&p).map_or(0, |m| m.len());
        let ok = p.parent().ok_or_else(|| std::io::Error::other("a chunk path with no folder")).and_then(std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&tmp, bytes))
            .and_then(|_| std::fs::rename(&tmp, &p));
        if let Err(e) = ok {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("S3 cache: {}: {e}", p.display());
            return;
        }
        let over = {
            let mut b = self.bytes.locked();
            *b = (*b + bytes.len() as u64).saturating_sub(old);
            *b > self.limit
        };
        if over {
            self.trim();
        }
    }

    /// Drops object `id` (it did not decrypt: damaged on the disk).
    pub(crate) fn remove(&self, id: Uuid) {
        let p = self.path(id);
        if let Ok(m) = std::fs::metadata(&p) && std::fs::remove_file(&p).is_ok() {
            let mut b = self.bytes.locked();
            *b = b.saturating_sub(m.len());
        }
    }

    /// Down to DISK_KEEP of the limit, the least lately read first.
    fn trim(&self) {
        let (mut files, _) = self.files();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        let keep = (self.limit as f64 * DISK_KEEP) as u64;
        if total > self.limit {
            files.sort_by_key(|f| f.2);
            for (p, len, _) in files {
                if total <= keep {
                    break;
                }
                if std::fs::remove_file(&p).is_ok() {
                    total -= len;
                }
            }
        }
        *self.bytes.locked() = total;
    }

    #[cfg(test)]
    pub(crate) fn kept(&self) -> u64 {
        *self.bytes.locked()
    }
}

/// See [`Storage::direct`].
pub struct Direct {
    pub url: String,
    pub key: [u8; 32],
    pub object: Uuid,
    /// The same ciphertext inside a delivery pack ([`Storage::pack_objects`]), when one holds it.
    pub pack: Option<PackSpan>,
}

/// Where a delivery pack holds an object's ciphertext: a presigned GET of the pack, read with
/// `Range: bytes=offset-(offset+len-1)`.
pub struct PackSpan {
    pub url: String,
    pub id: Uuid,
    pub offset: u64,
    pub len: u64,
}

/// Bytes of objects per delivery pack (docs/s3-reads.md): big enough that a game's start is a
/// few packs, small enough that one is uploaded again soon when a run is cut short.
pub const PACK_BYTES: usize = 64 << 20;

/// What [`Storage::pack_objects`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Packed {
    pub packs: usize,
    pub objects: usize,
    pub bytes: u64,
    /// in a pack already (or not in the bucket)
    pub skipped: usize,
}

/// See [`Storage::usage`]: (kind, objects, bytes) and each game's.
#[derive(Default, Debug)]
pub struct Usage {
    pub kinds: Vec<(String, u64, u64)>,
    pub games: Vec<GameUsage>,
}

#[derive(Default, Debug, PartialEq)]
pub struct GameUsage {
    pub game: String,
    /// its chunks no other game has
    pub own: u64,
    pub shared: u64,
    pub packs: u64,
}

/// Decrypted objects of at most 1 MiB, 64 MiB of them, the oldest put dropped first.
#[derive(Default)]
struct Cache {
    map: std::collections::HashMap<Uuid, std::sync::Arc<[u8]>>,
    order: std::collections::VecDeque<Uuid>,
    bytes: usize,
}

/// The plaintext cache's largest item: small objects (the catalog, saves, small chunks). The
/// chunks of today's packs (4 MiB on average) stay out: their decoded data is kept by
/// store_api's `Decoded` and their ciphertext by the `Disk` (review b01 asked).
const CACHE_ITEM: usize = 1 << 20;
const CACHE_BYTES: usize = 64 << 20;

impl Cache {
    fn get(&self, id: &Uuid) -> Option<std::sync::Arc<[u8]>> {
        self.map.get(id).cloned()
    }
    fn put(&mut self, id: Uuid, data: std::sync::Arc<[u8]>, limit: usize) {
        if data.len() > CACHE_ITEM || self.map.contains_key(&id) {
            return;
        }
        self.bytes += data.len();
        self.map.insert(id, data);
        self.order.push_back(id);
        while self.bytes > limit {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(d) = self.map.remove(&old) {
                self.bytes -= d.len();
            }
        }
    }
}

/// Uploads at once in push_store unless storage.toml says `uploads`.
const PUTS: usize = 16;
/// Objects push_store names in one transaction.
const RECORD_BATCH: usize = 64;

/// An object uploaded, not yet named (Storage::send, Storage::record).
struct Sent {
    logical: String,
    envelope: Envelope,
    size: i64,
}
/// Revisions of a save kept in S3 (older ones are retired, see [`Storage::gc`]).
#[cfg(test)]
use masu_kernel::save::SAVE_REVISIONS;

/// Tries after the first for a request the bucket could not answer (see [`retrying`]).
pub(crate) const RETRIES: u32 = 8;
/// Wait before the first try again; doubled each time, at most [`RETRY_MAX`].
#[cfg(not(test))]
const RETRY_FIRST: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(test)]
const RETRY_FIRST: std::time::Duration = std::time::Duration::from_millis(1);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(60);

/// The longest one object's download may take, its tries again included (review D-1: with
/// [`RETRIES`] of up to 60 s each and the waits between, a slow or down bucket held each of the
/// 16 request permits for 10 minutes and more, and every chunk read of every player behind
/// them). A read that gives up answers 502 and the page asks again.
#[cfg(not(test))]
pub(crate) const GET_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);
#[cfg(test)]
pub(crate) const GET_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// Whether a request that got `status` may succeed when tried again: throttling (429) and the
/// server's own errors (5xx). Hugging Face Buckets answer 503 often while uploading many
/// objects at once; a store of 60k chunks never got through in one push without this.
fn retryable(status: u16) -> bool {
    status == 429 || status / 100 == 5
}

/// `request`, tried again with doubling waits ([`RETRIES`] times at most) when it fails to
/// connect or the bucket answers [`retryable`]; the last answer otherwise (a 404 or 403 at once).
async fn retrying<F, Fut>(mut request: F) -> Result<s3::request::ResponseData>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<s3::request::ResponseData, s3::error::S3Error>>,
{
    let mut wait = RETRY_FIRST;
    for attempt in 0..=RETRIES {
        let last = attempt == RETRIES;
        match request().await {
            Ok(r) if !retryable(r.status_code()) || last => return Ok(r),
            Ok(r) => eprintln!("S3: {}, trying again in {wait:?}", r.status_code()),
            Err(e) if last => return Err(e.into()),
            // (with its causes: reqwest's own words, "error sending request", say not which)
            Err(e) => eprintln!("S3: {}, trying again in {wait:?}", causes(&e)),
        }
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(RETRY_MAX);
    }
    unreachable!()
}

/// `e` and the errors under it, `a: b: c`.
fn causes(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut at = e.source();
    while let Some(c) = at {
        s += &format!(": {c}");
        at = c.source();
    }
    s
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// What [`Storage::merge`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    /// objects only the other store named
    pub added: usize,
    /// catalog files taken from the other store
    pub replaced: usize,
    /// named by both, kept as this store had them (the other's copy retired)
    pub kept: usize,
    /// the same object in both
    pub same: usize,
    /// saves added or moved to the other's higher revision
    pub saves: usize,
    /// profiles, play sessions and game settings added
    pub records: usize,
    /// delivery pack spans taken: the other's packs of objects this store names as well
    pub packs: usize,
}

/// What [`Storage::gc`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Gc {
    pub deleted: usize,
    /// retired but kept: within the grace period, or a backup still refers to them
    pub kept: usize,
    pub backups_dropped: usize,
}

#[derive(Serialize, Deserialize)]
struct Backup {
    envelope: Envelope,
    created_ms: u128,
    recovery: Envelope,
    recovery_ciphertext: Vec<u8>,
}

fn catalog_path(store: &Path, logical: &str) -> Result<std::path::PathBuf> {
    let (dir, name) = logical.split_once('/').context("invalid catalog path")?;
    ensure!(matches!(dir, "trees" | "games") && !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']), "invalid catalog path");
    Ok(store.join(dir).join(name))
}

impl Storage {
    /// Restore the small catalogs locally. Chunk payloads remain remote and stream on demand.
    pub async fn hydrate_catalog(&self, store: &Path) -> Result<()> {
        let lock = crate::pack::StoreWrite::acquire_async(store).await?;
        self.hydrate_catalog_locked(&lock).await
    }

    pub(crate) async fn hydrate_catalog_locked(&self, lock: &crate::pack::StoreWrite) -> Result<()> {
        let store = lock.path();
        let paths: Vec<String> = {
            let c = self.db.connect()?;
            let mut q = c.prepare("SELECT logical FROM objects WHERE logical LIKE 'trees/%' OR logical LIKE 'games/%'")?;
            let rows = q.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for logical in paths {
            let path = catalog_path(store, &logical)?;
            if path.exists() {
                continue;
            }
            let data = self
                .get(&logical)
                .await?
                .context("catalog object missing")?;
            tokio::fs::create_dir_all(path.parent().context("a catalog path with no folder")?).await?;
            let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
            tokio::fs::write(&temp, data).await?;
            tokio::fs::rename(temp, path).await?;
        }
        Ok(())
    }
    pub fn configured(db: Library) -> Result<Option<Self>> {
        // (the stand-in bucket of `serve --dev-bucket`: any signature does)
        if let Some(path) = DEV_CONFIG.get() {
            return Self::open(db, path, Credentials::new(Some("dev"), Some("dev"), None, None, None)?).map(Some);
        }
        let Some(path) = std::env::var_os("MASU_STORAGE_CONFIG") else {
            return Ok(None);
        };
        // (the bucket's keys from the environment: AWS_ACCESS_KEY_ID..., the profile or the
        // instance's metadata)
        Self::open(db, Path::new(&path), Credentials::default()?).map(Some)
    }

    /// The storage `path` configures, signing with `credentials`: given, not looked up here, so
    /// that a test cannot reach for the machine's (#242: one did, and failed on CI)
    pub fn open(db: Library, path: &Path, credentials: Credentials) -> Result<Self> {
        let config: Config = toml::from_str(&std::fs::read_to_string(path)?)?;
        ensure!(!config.active_key.is_empty(), "active key id is empty");
        let region = match &config.endpoint {
            Some(endpoint) => Region::Custom {
                region: config.region.clone(),
                endpoint: endpoint.clone(),
            },
            None => config.region.parse()?,
        };
        let mut bucket = Bucket::new(&config.bucket, region, credentials)?;
        if config.endpoint.is_some() {
            bucket = bucket.with_path_style();
        }
        let gib = config.cache_gib.unwrap_or(64);
        // (review b01: gib << 30 wrapped for a typo'd huge value, to a tiny or zero limit)
        anyhow::ensure!(gib <= 1 << 30, "storage.toml: cache_gib {gib} is more than any disk");
        // One disk cache, plaintext cache, download gate and request cap per configuration
        // (review b01: the server opened three Storages, each walking the whole disk cache at
        // start, each with its own byte count, caches and 16 requests).
        let key = (path.canonicalize().unwrap_or_else(|_| path.to_path_buf()), db.0.clone());
        let shared = {
            let mut all = SHARED.lock().unwrap_or_else(|e| e.into_inner());
            match all.get(&key) {
                Some(s) => s.clone(),
                None => {
                    let disk = if gib == 0 {
                        None
                    } else {
                        let dir = match &config.cache {
                            Some(d) => d.clone(),
                            None => directories::ProjectDirs::from("", "", "masu").context("no home directory")?.cache_dir().join("s3"),
                        };
                        Some(std::sync::Arc::new(Disk::open(dir, gib << 30)?))
                    };
                    let s = Shared::new(16, disk);
                    all.insert(key, s.clone());
                    s
                }
            }
        };
        let s = Self {
            db,
            bucket,
            config,
            shared,
        };
        s.key(&s.config.active_key)?;
        Ok(s)
    }

    fn key(&self, id: &str) -> Result<[u8; 32]> {
        let hex = self
            .config
            .keys
            .get(id)
            .context("key id missing from storage configuration")?;
        ensure!(
            hex.len() == 64 && hex.is_ascii(),
            "key must be 64 hexadecimal characters"
        );
        let mut out = [0; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).context("invalid key encoding")?;
        }
        Ok(out)
    }

    fn path(&self, id: Uuid) -> String {
        format!("{}{}", self.config.prefix, id)
    }

    /// Uploads `bytes` as object `id`, one of `at_once` uploads going on together.
    async fn upload(&self, id: Uuid, bytes: &[u8], at_once: AtOnce) -> Result<()> {
        let path = self.path(id);
        // (rust-s3 gives up on any request after 60 s: a 64 MiB pack on a slow uplink never got
        // through, nor 16 chunks of 4 MiB at once from worker1, 2026-11-17. Each object gets time
        // for its share of the line.)
        let bucket = self.bucket.with_request_timeout(upload_timeout(bytes.len(), at_once))?;
        let r = retrying(|| bucket.put_object(&path, bytes)).await?;
        ensure!(
            r.status_code() / 100 == 2,
            "S3 upload failed: {}",
            r.status_code()
        );
        Ok(())
    }

    /// One object's ciphertext, within [`GET_BUDGET`] for all its tries.
    async fn download(&self, id: Uuid) -> Result<Vec<u8>> {
        let path = self.path(id);
        let r = tokio::time::timeout(GET_BUDGET, retrying(|| self.bucket.get_object(&path)))
            .await
            .map_err(|_| anyhow::anyhow!("S3 download of {id}: no answer within {GET_BUDGET:?}"))??;
        ensure!(
            r.status_code() == 200,
            "S3 download failed: {}",
            r.status_code()
        );
        // (the body as it came, not a copy of it: 2x a chunk per download, 48 at once)
        Ok(Vec::from(r.into_bytes()))
    }

    pub async fn put(&self, logical: &str, bytes: &[u8]) -> Result<()> {
        let sent = self.send(logical, bytes, AtOnce::ALONE).await?;
        self.record(&[sent])
    }

    /// Encrypts `bytes` as a new object and uploads it (not yet named: see [`Storage::record`]).
    async fn send(&self, logical: &str, bytes: &[u8], at_once: AtOnce) -> Result<Sent> {
        let (envelope, ciphertext) = Envelope::encrypt(
            &self.key(&self.config.active_key)?,
            &self.config.active_key,
            bytes,
        )?;
        self.upload(envelope.object, &ciphertext, at_once).await?;
        Ok(Sent { logical: logical.to_string(), envelope, size: bytes.len() as i64 })
    }

    /// Names the objects `sent` in one transaction (one commit for many: a commit is a few
    /// fsyncs, and a push sends thousands of chunks); what they replace is retired.
    fn record(&self, sent: &[Sent]) -> Result<()> {
        let mut c = self.db.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for s in sent {
            let old: Option<String> = tx.query_row("SELECT envelope FROM objects WHERE logical=?", [&s.logical], |r| r.get(0)).optional()?;
            tx.execute("INSERT INTO objects(logical,envelope,size) VALUES(?,?,?) ON CONFLICT(logical) DO UPDATE SET envelope=excluded.envelope,size=excluded.size",
                rusqlite::params![s.logical, serde_json::to_string(&s.envelope)?, s.size])?;
            if let Some(old) = old {
                retire(&tx, serde_json::from_str::<Envelope>(&old)?.object)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Drops `logical`; its object is retired.
    pub fn forget(&self, logical: &str) -> Result<()> {
        self.forget_all(&[logical])
    }

    /// Drops every name in `logicals` in one transaction (a game's thousands of chunks: one
    /// commit, not one each); their objects are retired.
    pub fn forget_all<S: AsRef<str>>(&self, logicals: &[S]) -> Result<()> {
        let mut c = self.db.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for logical in logicals {
            let logical = logical.as_ref();
            let old: Option<String> = tx.query_row("SELECT envelope FROM objects WHERE logical=?", [logical], |r| r.get(0)).optional()?;
            if let Some(old) = old {
                tx.execute("DELETE FROM objects WHERE logical=?", [logical])?;
                retire(&tx, serde_json::from_str::<Envelope>(&old)?.object)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forgets save `name`'s revisions older than the last
    /// [`masu_kernel::save::SAVE_REVISIONS`] up to `current`.
    pub fn prune_save(&self, name: &str, current: i64) -> Result<()> {
        let prefix = format!("saves/{name}/");
        let old: Vec<String> = {
            let c = self.db.connect()?;
            // (a range of the primary key, not a scan of every object: once per save written)
            let mut q = c.prepare("SELECT logical FROM objects WHERE logical >= ?1 AND logical < ?2")?;
            let rows = q.query_map(rusqlite::params![prefix, format!("saves/{name}0")], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|l| l.strip_prefix(&prefix).and_then(|rev| rev.parse::<i64>().ok())
                    .is_some_and(|rev| masu_kernel::save::revision_expired(current, rev)))
                .collect()
        };
        for l in old {
            self.forget(&l)?;
        }
        Ok(())
    }

    /// Deletes locally owned, unreferenced objects retired for at least `grace`.
    /// Snapshots predating reference tracking pin all retired objects until pruned.
    pub async fn gc(&self, grace: std::time::Duration, keep_backups: Option<usize>) -> Result<Gc> {
        let _guard = crate::pack::StoreWrite::acquire_async(self.db.0.parent().context("missing database directory")?).await?;
        let mut out = Gc::default();
        // A live publisher holds this lock until the descriptor is registered.
        // A process that died left a pending set; delete its uploads before unpinning.
        let writer = self.backup_lock()?;
        if writer.try_lock().is_ok() {
            let pending = {
                let c = self.db.connect()?;
                let mut q = c.prepare("SELECT id,snapshot FROM backup_sets WHERE id NOT IN (SELECT id FROM backups)")?;
                let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (id, snapshot) in pending {
                self.delete(Uuid::parse_str(&id)?).await?;
                self.delete(Uuid::parse_str(&snapshot)?).await?;
                self.db.connect()?.execute("DELETE FROM backup_sets WHERE id=?", [id])?;
            }
        }
        drop(writer);
        if let Some(n) = keep_backups {
            let old: Vec<(String, String)> = {
                let c = self.db.connect()?;
                let mut q = c.prepare("SELECT id,snapshot FROM backups ORDER BY created_ms DESC LIMIT -1 OFFSET ?")?;
                let rows = q.query_map([n as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (id, snapshot) in old {
                self.delete(Uuid::parse_str(&id)?).await?;
                self.delete(Uuid::parse_str(&snapshot)?).await?;
                let mut c = self.db.connect()?;
                let tx = c.transaction()?;
                tx.execute("DELETE FROM backups WHERE id=?", [&id])?;
                tx.execute("DELETE FROM backup_sets WHERE id=?", [&id])?;
                tx.commit()?;
                out.backups_dropped += 1;
            }
        }
        let retired: Vec<(String, i64, bool)> = {
            let c = self.db.connect()?;
            // Legacy records timestamped publication, after the snapshot was taken.
            // A deletion based on that timestamp could invalidate a retained backup.
            if c.query_row("SELECT EXISTS(SELECT 1 FROM backups WHERE id NOT IN (SELECT id FROM backup_sets))", [], |r| r.get::<_, bool>(0))? {
                out.kept = usize::try_from(c.query_row("SELECT COUNT(*) FROM retired", [], |r| r.get::<_, i64>(0))?)?;
                return Ok(out);
            }
            let mut q = c.prepare("SELECT object,retired_ms,
                object IN (SELECT object FROM imported_objects) OR object IN (SELECT object FROM backup_refs)
                FROM retired")?;
            let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let now = now_ms();
        for (object, at, pinned) in retired {
            if pinned || !masu_kernel::save::older_than_grace(at, now, grace.as_millis()) {
                out.kept = out.kept.saturating_add(1);
                continue;
            }
            // (an object still named by a row: never; new objects get new UUIDs)
            let named: bool = self.db.connect()?.query_row("SELECT EXISTS(SELECT 1 FROM objects WHERE instr(envelope,?) > 0)", [&object], |r| r.get(0))?;
            if named {
                out.kept += 1;
                continue;
            }
            self.delete(Uuid::parse_str(&object)?).await?;
            self.db.connect()?.execute("DELETE FROM retired WHERE object=?", [&object])?;
            out.deleted += 1;
        }
        Ok(out)
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        let path = self.path(id);
        let r = retrying(|| self.bucket.delete_object(&path)).await?;
        ensure!(r.status_code() / 100 == 2 || r.status_code() == 404, "S3 delete failed: {}", r.status_code());
        Ok(())
    }

    pub async fn get(&self, logical: &str) -> Result<Option<std::sync::Arc<[u8]>>> {
        let entry: Option<String> = self
            .db
            .connect()?
            .query_row(
                "SELECT envelope FROM objects WHERE logical=?",
                [logical],
                |r| r.get(0),
            )
            .optional()?;
        let Some(entry) = entry else { return Ok(None) };
        let e: Envelope = serde_json::from_str(&entry)?;
        if let Some(data) = self.shared.cache.locked().get(&e.object) {
            return Ok(Some(data));
        }
        // the same object asked for at once (a file's chunks fetched ahead, two clients):
        // one download
        let gate = self.gate(e.object);
        let data = async {
            let _one = gate.1.lock().await;
            if let Some(data) = self.shared.cache.locked().get(&e.object) {
                return Ok(data);
            }
            let key = self.key(&e.key_id)?;
            let kept = match &self.shared.disk {
                Some(d) => match d.pending(e.object) {
                    Some(data) => Some(data),
                    None => {
                        let (d, id) = (d.clone(), e.object);
                        tokio::task::spawn_blocking(move || d.read(id)).await.ok().flatten()
                    }
                }
                None => None,
            };
            // (kept but not decrypting: damaged on the disk; taken from the bucket again)
            let plain = match kept.map(|c| e.decrypt(&key, &c)) {
                Some(Ok(p)) => p,
                bad => {
                    if bad.is_some() {
                        eprintln!("S3 cache: object {} damaged, read again", e.object);
                        if let Some(d) = &self.shared.disk {
                            d.remove(e.object);
                        }
                    }
                    let _permit = self.shared.requests.acquire().await?;
                    let ciphertext = self.download(e.object).await?;
                    let plain = e.decrypt(&key, &ciphertext)?;
                    if let Some(d) = &self.shared.disk {
                        d.write_later(e.object, ciphertext);
                    }
                    plain
                }
            };
            let data = std::sync::Arc::<[u8]>::from(plain);
            self.shared.cache.locked().put(e.object, data.clone(), CACHE_BYTES);
            Ok::<_, anyhow::Error>(data)
        }
        .await;
        drop(gate);
        Ok(Some(data?))
    }

    /// Object `id`'s download gate, let go when dropped: also when the future holding it is
    /// dropped mid-download (a client gone away; review b01: its entry stayed).
    fn gate(&self, id: Uuid) -> Gate<'_> {
        let g = self.shared.inflight.locked().entry(id).or_default().clone();
        Gate(self, g, id)
    }

    /// Downloads waiting or running now (tests).
    #[cfg(test)]
    pub(crate) fn gates(&self) -> usize {
        self.shared.inflight.locked().len()
    }

    fn ungate(&self, id: Uuid, gate: &std::sync::Arc<tokio::sync::Mutex<()>>) {
        let mut f = self.shared.inflight.locked();
        // (the last one out: the map's and ours are all that are left)
        if f.get(&id).is_some_and(|g| std::sync::Arc::ptr_eq(g, gate)) && std::sync::Arc::strong_count(gate) == 2 {
            f.remove(&id);
        }
    }

    /// How a browser fetches object `logical` itself: a presigned GET of it (`expiry` seconds),
    /// its key and its UUID (crypto: [`Envelope::data_key`]). The browser then reads the bucket
    /// as fast as its own line allows, many at once, not through this server's line. None when
    /// the bucket lacks it. The bucket even when this server's disk has the object (until 2026-11-14 the disk was preferred as quicker to answer, but worker2's uplink
    /// gave ~160 KB/s under its other jobs and anemoi's chunks then took 95 s each).
    pub async fn direct(&self, logical: &str, expiry: u32) -> Result<Option<Direct>> {
        let entry: Option<String> = self.db.connect()?.query_row("SELECT envelope FROM objects WHERE logical=?", [logical], |r| r.get(0)).optional()?;
        let Some(entry) = entry else { return Ok(None) };
        let e: Envelope = serde_json::from_str(&entry)?;
        let key = e.data_key(&self.key(&e.key_id)?)?;
        let url = self.bucket.presign_get(self.path(e.object), expiry, None).await?;
        // (a pack made for an object since replaced holds the old one: not that)
        let span: Option<(String, i64, i64)> = self
            .db
            .connect()?
            .query_row("SELECT pack,offset,len FROM packs WHERE logical=? AND object=?", rusqlite::params![logical, e.object.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let pack = match span {
            Some((id, offset, len)) => {
                let id = Uuid::parse_str(&id)?;
                Some(PackSpan { url: self.bucket.presign_get(self.path(id), expiry, None).await?, id, offset: offset as u64, len: len as u64 })
            }
            None => None,
        };
        Ok(Some(Direct { url, key, object: e.object, pack }))
    }

    /// [`Storage::direct`] of each of `logicals` (those the store names), the database read
    /// once for them all on a blocking thread (review b01: a game start's 4096 answers opened
    /// two connections each, inline on a request thread).
    pub async fn direct_many(&self, logicals: Vec<String>, expiry: u32) -> Result<Vec<(String, Direct)>> {
        type Row = (String, String, Option<(String, i64, i64)>);
        let db = self.db.clone();
        let rows: Vec<Row> = tokio::task::spawn_blocking(move || -> Result<Vec<Row>> {
            let c = db.connect()?;
            let mut env = c.prepare_cached("SELECT envelope FROM objects WHERE logical=?")?;
            let mut pack = c.prepare_cached("SELECT pack,offset,len FROM packs WHERE logical=? AND object=?")?;
            let mut out = Vec::new();
            for l in logicals {
                let Some(entry): Option<String> = env.query_row([&l], |r| r.get(0)).optional()? else { continue };
                let e: Envelope = serde_json::from_str(&entry)?;
                let span = pack.query_row(rusqlite::params![l, e.object.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
                out.push((l, entry, span));
            }
            Ok(out)
        })
        .await??;
        let mut out = Vec::with_capacity(rows.len());
        for (l, entry, span) in rows {
            let e: Envelope = serde_json::from_str(&entry)?;
            let key = e.data_key(&self.key(&e.key_id)?)?;
            let url = self.bucket.presign_get(self.path(e.object), expiry, None).await?;
            let pack = match span {
                Some((id, offset, len)) => {
                    let id = Uuid::parse_str(&id)?;
                    Some(PackSpan { url: self.bucket.presign_get(self.path(id), expiry, None).await?, id, offset: offset as u64, len: len as u64 })
                }
                None => None,
            };
            out.push((l, Direct { url, key, object: e.object, pack }));
        }
        Ok(out)
    }

    /// Whether the store names `logical`.
    pub fn has(&self, logical: &str) -> Result<bool> {
        Ok(self.db.connect()?.query_row("SELECT 1 FROM objects WHERE logical=?", [logical], |_| Ok(())).optional()?.is_some())
    }

    /// Delivery packs (docs/s3-reads.md): the objects named `logicals` (a game's chunks, in the
    /// order it reads them), their ciphertexts as the bucket has them, side by side in new
    /// objects of about `target` bytes each. The objects stay as they are (and are read alone as
    /// before); a browser reads many of them with one ranged GET of a pack, the bucket's wait
    /// for an object paid once for them all. Those in a pack already are left out, so packing a
    /// game again packs only what was added. A pack whose objects all went to newer packs is
    /// retired ([`Storage::gc`] deletes it).
    /// Each pack is marked as `game`'s ([`Storage::usage`], [`Storage::unpack`]).
    #[cfg(test)]
    pub async fn pack_objects(&self, game: &str, logicals: &[String], target: usize) -> Result<Packed> {
        self.pack_objects_paced(game, logicals, target, None).await
    }

    /// [`Storage::pack_objects`] at most `rate` bytes a second on average (uploads; reading the
    /// objects waits as well), so a server on the same line still serves its players.
    pub async fn pack_objects_paced(&self, game: &str, logicals: &[String], target: usize, rate: Option<u64>) -> Result<Packed> {
        use futures_util::StreamExt;
        let started = std::time::Instant::now();
        let mut out = Packed::default();
        let mut todo: Vec<(String, Uuid)> = Vec::new();
        {
            let c = self.db.connect()?;
            let mut seen = std::collections::HashSet::new();
            for l in logicals {
                if !seen.insert(l.as_str()) {
                    continue;
                }
                let entry: Option<String> = c.query_row("SELECT envelope FROM objects WHERE logical=?", [l], |r| r.get(0)).optional()?;
                let Some(entry) = entry else {
                    out.skipped += 1;
                    continue;
                };
                let e: Envelope = serde_json::from_str(&entry)?;
                let packed: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM packs WHERE logical=? AND object=?)", rusqlite::params![l, e.object.to_string()], |r| r.get(0))?;
                if packed {
                    out.skipped += 1;
                    continue;
                }
                todo.push((l.clone(), e.object));
            }
        }
        // (read several at once, kept in order: the bucket waits for each)
        let mut read = futures_util::stream::iter(todo.into_iter())
            .map(|(l, id)| async move {
                let kept = match &self.shared.disk {
                    Some(d) => {
                        let d = d.clone();
                        tokio::task::spawn_blocking(move || d.read(id)).await.ok().flatten()
                    }
                    None => None,
                };
                let c = match kept {
                    Some(c) => c,
                    None => {
                        let _permit = self.shared.requests.acquire().await?;
                        std::sync::Arc::new(self.download(id).await?)
                    }
                };
                Ok::<_, anyhow::Error>((l, id, c))
            })
            .buffered(16);
        let mut buf: Vec<u8> = Vec::new();
        let mut rows: Vec<(String, Uuid, u64, u64)> = Vec::new();
        loop {
            let next = read.next().await.transpose()?;
            let end = next.is_none();
            if let Some((l, id, c)) = next {
                rows.push((l, id, buf.len() as u64, c.len() as u64));
                buf.extend_from_slice(&c);
                if buf.len() < target {
                    continue;
                }
            }
            if !rows.is_empty() {
                out.packs += 1;
                out.objects += rows.len();
                out.bytes += buf.len() as u64;
                self.record_pack(game, std::mem::take(&mut buf), std::mem::take(&mut rows)).await?;
                if let Some(t) = rate.and_then(|r| pace(out.bytes, r).checked_sub(started.elapsed())) {
                    tokio::time::sleep(t).await;
                }
            }
            if end {
                return Ok(out);
            }
        }
    }

    /// Uploads one pack and names where it holds each object.
    async fn record_pack(&self, game: &str, bytes: Vec<u8>, rows: Vec<(String, Uuid, u64, u64)>) -> Result<()> {
        let pack = Uuid::new_v4();
        self.upload(pack, &bytes, AtOnce::ALONE).await?;
        let mut c = self.db.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO pack_games(pack,game,made_ms) VALUES(?,?,?)", rusqlite::params![pack.to_string(), game, now_ms()])?;
        let mut old = std::collections::HashSet::new();
        for (l, object, offset, len) in &rows {
            if let Some(p) = tx.query_row("SELECT pack FROM packs WHERE logical=?", [l], |r| r.get::<_, String>(0)).optional()? {
                old.insert(p);
            }
            tx.execute(
                "INSERT INTO packs(logical,object,pack,offset,len) VALUES(?,?,?,?,?) ON CONFLICT(logical) DO UPDATE SET object=excluded.object,pack=excluded.pack,offset=excluded.offset,len=excluded.len",
                rusqlite::params![l, object.to_string(), pack.to_string(), *offset as i64, *len as i64],
            )?;
        }
        for p in old {
            let used: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM packs WHERE pack=?)", [&p], |r| r.get(0))?;
            if !used {
                tx.execute("DELETE FROM pack_games WHERE pack=?", [&p])?;
                retire(&tx, Uuid::parse_str(&p)?)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// What the bucket holds, to see what could go: the named objects by kind (the first part
    /// of their names: chunks, trees, games, saves, states, ...; bytes as stored before
    /// encryption), the delivery packs, and for each of `games` (id, its chunks' names) the
    /// bytes of its chunks no other of them has (what deleting it alone frees), of those it
    /// shares, and of its packs. Packs made before they were marked are game "?". Largest first.
    pub fn usage(&self, games: &[(String, Vec<String>)]) -> Result<Usage> {
        let c = self.db.connect()?;
        let mut out = Usage::default();
        {
            let mut q = c.prepare("SELECT substr(logical,1,instr(logical||'/','/')-1) k, COUNT(*), SUM(size) FROM objects GROUP BY k")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64)))?;
            out.kinds = rows.collect::<rusqlite::Result<_>>()?;
        }
        let packs: Vec<(String, u64, u64)> = {
            let mut q = c.prepare("SELECT COALESCE(g.game,'?'), COUNT(DISTINCT p.pack), SUM(p.len) FROM packs p LEFT JOIN pack_games g USING(pack) GROUP BY 1")?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let (n, b) = packs.iter().fold((0, 0), |a, p| (a.0 + p.1, a.1 + p.2));
        if n > 0 {
            out.kinds.push(("packs".into(), n, b));
        }
        let mut size = c.prepare("SELECT size FROM objects WHERE logical=?")?;
        let mut owners: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (_, ls) in games {
            for l in ls.iter().collect::<std::collections::HashSet<_>>() {
                *owners.entry(l.as_str()).or_default() += 1;
            }
        }
        for (id, ls) in games {
            let mut g = GameUsage { game: id.clone(), ..Default::default() };
            for l in ls.iter().collect::<std::collections::HashSet<_>>() {
                let Some(n) = size.query_row([l], |r| r.get::<_, i64>(0)).optional()? else { continue };
                if owners[l.as_str()] == 1 { g.own += n as u64 } else { g.shared += n as u64 }
            }
            g.packs = packs.iter().find(|p| p.0 == *id).map_or(0, |p| p.2);
            out.games.push(g);
        }
        for (id, _, b) in &packs {
            if !games.iter().any(|g| g.0 == *id) {
                out.games.push(GameUsage { game: id.clone(), packs: *b, ..Default::default() });
            }
        }
        out.kinds.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        out.games.sort_by(|a, b| (b.own + b.packs).cmp(&(a.own + a.packs)).then(a.game.cmp(&b.game)));
        Ok(out)
    }

    /// Drops the delivery packs of `games` (every pack: None; "?" names those made before packs
    /// were marked): their objects are read alone again, the packs are retired for [`Storage::gc`]
    /// to delete. The packs and bytes dropped.
    pub fn unpack(&self, games: Option<&[String]>) -> Result<(usize, u64)> {
        let mut c = self.db.connect()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let all: Vec<(String, String, i64)> = {
            let mut q = tx.prepare("SELECT p.pack, COALESCE(g.game,'?'), SUM(p.len) FROM packs p LEFT JOIN pack_games g USING(pack) GROUP BY p.pack")?;
            let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let (mut n, mut bytes) = (0, 0);
        for (pack, game, len) in all {
            if games.is_some_and(|gs| !gs.contains(&game)) {
                continue;
            }
            tx.execute("DELETE FROM packs WHERE pack=?", [&pack])?;
            tx.execute("DELETE FROM pack_games WHERE pack=?", [&pack])?;
            retire(&tx, Uuid::parse_str(&pack)?)?;
            n += 1;
            bytes += len as u64;
        }
        tx.commit()?;
        Ok((n, bytes))
    }

    /// Uploads the store's files not uploaded yet, `uploads` (storage.toml, [`PUTS`]) at once:
    /// chunks first, then trees, then games (a game's description goes up once what it names
    /// is there). The objects sent are named [`RECORD_BATCH`] at a time (a push cut short can
    /// leave up to that many sent and never named in the bucket).
    pub async fn push_store(&self, store: &Path) -> Result<()> {
        let _lock = crate::pack::StoreWrite::acquire_async(store).await?;
        let have = self.chunks_named()?;
        for top in ["chunks", "trees", "games"] {
            let mut files = Vec::new();
            let mut dirs = vec![store.join(top)];
            while let Some(dir) = dirs.pop() {
                if !dir.exists() {
                    continue;
                }
                for entry in std::fs::read_dir(dir)? {
                    let entry = entry?;
                    if entry.file_name().to_string_lossy().starts_with('.') {
                        continue;
                    }
                    let kind = entry.file_type()?;
                    if kind.is_dir() {
                        dirs.push(entry.path());
                    } else if kind.is_file() {
                        let path = entry.path();
                        // (with / between the parts on Windows too: the names the S3 store knows)
                        let logical = path
                            .strip_prefix(store)?
                            .components()
                            .map(|c| c.as_os_str().to_str().context("non-UTF8 store path"))
                            .collect::<Result<Vec<_>>>()?
                            .join("/");
                        files.push((path, logical));
                    }
                }
            }
            files.sort();
            files.retain(|(_, logical)| !have.contains(logical));
            self.send_files(files).await?;
        }
        Ok(())
    }

    /// The chunks of the store's trees `names` the bucket lacks, then the trees' lists: what the
    /// server made itself (sake's C:\ with its fonts and RTPs, the benchmark's programs), which
    /// the page can read only from the bucket. A chunk neither has is an error.
    pub async fn push_trees(&self, store: &Path, names: &[&str]) -> Result<()> {
        let _lock = crate::pack::StoreWrite::acquire_async(store).await?;
        let have = self.chunks_named()?;
        let mut chunks = std::collections::BTreeSet::new();
        let mut lists = Vec::new();
        for name in names {
            let logical = format!("trees/{name}.tsv");
            let path = store.join("trees").join(format!("{name}.tsv"));
            let tsv = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
            for l in crate::tree_list::good_lines(&tsv) {
                if let crate::tree_list::Kind::File(c) = l.kind {
                    chunks.extend(c.list().iter().filter_map(|c| c.id).map(|id| id.store_path()).filter(|p| !have.contains(p)));
                }
            }
            lists.push((path, logical));
        }
        let chunks: Vec<_> = chunks.into_iter().map(|logical| (logical.split('/').fold(store.to_path_buf(), |p, part| p.join(part)), logical)).collect();
        if let Some((path, logical)) = chunks.iter().find(|(p, _)| !p.is_file()) {
            anyhow::bail!("{logical} is neither in the bucket nor at {}", path.display());
        }
        self.send_files(chunks).await?;
        self.send_files(lists).await
    }

    /// The chunks the bucket has (named by their content: one there is never sent again).
    fn chunks_named(&self) -> Result<std::collections::HashSet<String>> {
        let c = self.db.connect()?;
        let mut q = c.prepare("SELECT logical FROM objects WHERE logical LIKE 'chunks/%'")?;
        let rows = q.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sends `files` (path, logical name), `uploads` at once, naming them [`RECORD_BATCH`] at a time.
    async fn send_files(&self, files: Vec<(std::path::PathBuf, String)>) -> Result<()> {
        use futures_util::TryStreamExt;
        let at_once = AtOnce::new(self.config.uploads.unwrap_or(PUTS));
        let mut sent = futures_util::stream::iter(files.into_iter().map(Ok::<_, anyhow::Error>))
            .map_ok(|(path, logical)| async move {
                let data = tokio::fs::read(&path).await?;
                self.send(&logical, &data, at_once).await
            })
            .try_buffer_unordered(at_once.get());
        let mut batch = Vec::new();
        while let Some(s) = sent.try_next().await? {
            batch.push(s);
            if batch.len() >= RECORD_BATCH {
                self.record(&batch)?;
                batch.clear();
            }
        }
        self.record(&batch)
    }

    fn backup_lock(&self) -> Result<crate::lockfile::LockFile> {
        crate::lockfile::LockFile::open(&self.db.0.with_extension("backup.lock"))
    }

    pub async fn backup(&self) -> Result<Uuid> {
        let writer = self.backup_lock()?;
        let _writer = tokio::task::spawn_blocking(move || -> std::io::Result<_> { writer.lock()?; Ok(writer) }).await??;
        let guard = crate::pack::StoreWrite::acquire_async(self.db.0.parent().context("missing database directory")?).await?;
        let created_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
        let dir = tempfile::tempdir_in(guard.path())?;
        let snapshot = dir.path().join("snapshot.sqlite");
        self.db.snapshot(&snapshot)?;
        let data = zstd::encode_all(std::fs::File::open(&snapshot)?, 3)?;
        let (envelope, ciphertext) = Envelope::encrypt(&self.key(&self.config.active_key)?, &self.config.active_key, &data)?;
        let id = Uuid::new_v4();
        {
            let mut c = self.db.connect()?;
            c.execute("ATTACH DATABASE ? AS snap", [snapshot.to_str().context("non-UTF8 snapshot path")?])?;
            let tx = c.transaction()?;
            tx.execute("INSERT INTO backup_sets(id,snapshot) VALUES(?,?)", [id.to_string(), envelope.object.to_string()])?;
            tx.execute("INSERT INTO backup_refs(id,object) SELECT ?, json_extract(envelope,'$.object') FROM snap.objects UNION SELECT ?,pack FROM snap.packs", [id.to_string(), id.to_string()])?;
            tx.commit()?;
        }
        drop(guard);
        drop(dir);
        self.upload(envelope.object, &ciphertext, AtOnce::ALONE).await?;
        // A snapshot contains envelopes from that date. Keep its required wrapping keys
        // encrypted alongside it so rotation can retire old keys without rewriting the DB.
        let (recovery, recovery_ciphertext) = Envelope::encrypt(
            &self.key(&self.config.active_key)?,
            &self.config.active_key,
            &serde_json::to_vec(&self.config.keys)?,
        )?;
        let backup = Backup {
            envelope,
            recovery,
            recovery_ciphertext,
            created_ms,
        };
        self.upload(id, &serde_json::to_vec(&backup)?, AtOnce::ALONE).await?;
        self.db.connect()?.execute(
            "INSERT INTO backups(id,snapshot,created_ms) VALUES(?,?,?)",
            rusqlite::params![id.to_string(), backup.envelope.object.to_string(), backup.created_ms as i64],
        )?;
        Ok(id)
    }

    /// Restores into a NEW database path, never over a running database.
    pub async fn restore(&self, id: Uuid, output: &Path) -> Result<()> {
        use std::io::{Read, Write};
        ensure!(!output.exists(), "restore destination already exists");
        let b: Backup = serde_json::from_slice(&self.download(id).await?)?;
        let recovered: BTreeMap<String, String> = serde_json::from_slice(
            &b.recovery
                .decrypt(&self.key(&b.recovery.key_id)?, &b.recovery_ciphertext)?,
        )?;
        let compressed = b.envelope.decrypt(
            &self.key(&b.envelope.key_id)?,
            &self.download(b.envelope.object).await?,
        )?;
        let temp = output.with_extension(format!("{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            let mut decoder = zstd::Decoder::new(compressed.as_slice())?.take((4u64 << 30) + 1);
            let bytes = std::io::copy(&mut decoder, &mut f)?;
            ensure!(bytes <= 4u64 << 30, "backup exceeds restore limit");
            f.flush()?;
            f.sync_all()?;
            let mut c = rusqlite::Connection::open(&temp)?;
            let check: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            ensure!(check == "ok", "invalid database snapshot");
            c.execute_batch(crate::library::SCHEMA)?;
            let tx = c.transaction()?;
            // The backup restored from is this database's too: its objects stay pinned here
            // (backup_refs), so a gc after the restored store moves on never deletes what
            // restoring it again needs. The snapshot was taken before `backup` registered it in
            // the store it came from (review e18bf44: restore, update, gc, restore lost objects).
            register_backup(&tx, id, b.envelope.object, b.created_ms)?;
            let rows: Vec<(String, String)> = {
                let mut q = tx.prepare("SELECT logical,envelope FROM objects")?;
                let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (logical, json) in rows {
                let e: Envelope = serde_json::from_str(&json)?;
                let hex = recovered
                    .get(&e.key_id)
                    .context("backup recovery key missing")?;
                ensure!(hex.len() == 64 && hex.is_ascii(), "invalid recovery key");
                let mut old = [0; 32];
                for (i, b) in old.iter_mut().enumerate() {
                    *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)?;
                }
                let next = e.rewrap(
                    &old,
                    &self.key(&self.config.active_key)?,
                    &self.config.active_key,
                )?;
                tx.execute(
                    "UPDATE objects SET envelope=? WHERE logical=?",
                    rusqlite::params![serde_json::to_string(&next)?, logical],
                )?;
            }
            tx.commit()?;
            drop(c);
            // hard_link fails rather than replacing a concurrently created destination.
            std::fs::hard_link(&temp, output)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(&temp);
        result
    }

    /// Merges another store's backup `id` (`masu storage backup` there, the same bucket and
    /// keys) into this store's database, so games added and pushed elsewhere show up here:
    /// - objects only the other names are added (rewrapped with the active key, as `restore`);
    /// - a catalog file (games/, trees/) both name differently is taken from the other, this
    ///   one's object retired: what was added there last wins;
    /// - anything else both name (chunks: the same content; saves) stays this one's, and the
    ///   other's object is retired (`gc` deletes it);
    /// - a save takes the higher of the two revisions; profiles, play sessions and game settings
    ///   only the other has are added. Backups and the retired list stay this one's.
    ///
    /// Then `hydrate_catalog` brings the new games' entries and trees here.
    pub async fn merge(&self, id: Uuid, store: &Path) -> Result<Merged> {
        let lock = crate::pack::StoreWrite::acquire_async(store).await?;
        let other = self.db.0.with_file_name(format!(".merge-{}.sqlite3", Uuid::new_v4()));
        self.restore(id, &other).await?;
        let mut replaced = Vec::new();
        let result = (|| -> Result<Merged> {
            let mut out = Merged::default();
            let mut c = self.db.connect()?;
            c.execute("ATTACH DATABASE ? AS other", [other.to_str().context("non-UTF8 path")?])?;
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let rows: Vec<(String, String, i64, Option<String>)> = {
                let mut q = tx.prepare("SELECT o.logical,o.envelope,o.size,m.envelope FROM other.objects o LEFT JOIN main.objects m ON m.logical=o.logical")?;
                let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for (logical, theirs, size, ours) in rows {
                use masu_kernel::store::{MergeObject, merge_object};

                let catalog = logical.starts_with("games/") || logical.starts_with("trees/");
                if catalog {
                    catalog_path(store, &logical)?;
                }
                let object = |e: &str| -> Result<Uuid> { Ok(serde_json::from_str::<Envelope>(e)?.object) };
                let incoming = object(&theirs)?;
                let local = ours.as_deref().map(object).transpose()?;
                tx.execute("INSERT OR IGNORE INTO imported_objects(object) VALUES(?)", [incoming.to_string()])?;
                match merge_object(local.map(|id| id.as_u128()), incoming.as_u128(), catalog) {
                    MergeObject::Add => {
                        tx.execute("INSERT INTO main.objects(logical,envelope,size) VALUES(?,?,?)", rusqlite::params![logical, theirs, size])?;
                        out.added += 1;
                    }
                    MergeObject::Same => out.same = out.same.saturating_add(1),
                    MergeObject::Replace(old) => {
                        tx.execute("UPDATE main.objects SET envelope=?,size=? WHERE logical=?", rusqlite::params![theirs, size, logical])?;
                        retire(&tx, Uuid::from_u128(old))?;
                        replaced.push(logical.clone());
                        out.replaced += 1;
                    }
                    MergeObject::Keep => {
                        out.kept += 1;
                    }
                }
            }
            let has = |t: &str| -> rusqlite::Result<bool> {
                tx.query_row("SELECT EXISTS(SELECT 1 FROM other.sqlite_master WHERE type='table' AND name=?)", [t], |r| r.get(0))
            };
            if has("saves")? {
                tx.execute_batch("CREATE TABLE IF NOT EXISTS main.saves(name TEXT PRIMARY KEY, revision INTEGER NOT NULL, data BLOB);")?;
                out.saves = tx.execute(
                    "INSERT INTO main.saves(name,revision,data) SELECT name,revision,data FROM other.saves WHERE true
                     ON CONFLICT(name) DO UPDATE SET revision=excluded.revision,data=excluded.data WHERE excluded.revision>main.saves.revision",
                    [],
                )?;
            }
            // (the browsers' records the other store's writes superseded: a browser asks the
            // merged store about them (saves.rs Saves::covered); generations are UUIDs, never
            // two browsers' one)
            if has("save_covers")? {
                tx.execute_batch("CREATE TABLE IF NOT EXISTS main.save_covers(name TEXT NOT NULL, generation TEXT NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(name, generation));")?;
                tx.execute("INSERT OR IGNORE INTO main.save_covers(name,generation,revision) SELECT name,generation,revision FROM other.save_covers", [])?;
            }
            // (packs made elsewhere, of the objects named here: taken; a pack no row names then,
            // here or the other's, is retired)
            if has("packs")? {
                let ids = |q: &str| -> rusqlite::Result<std::collections::HashSet<String>> {
                    let mut q = tx.prepare(q)?;
                    let r = q.query_map([], |r| r.get(0))?.collect();
                    r
                };
                let before = ids("SELECT DISTINCT pack FROM main.packs")?;
                tx.execute("INSERT OR IGNORE INTO imported_objects(object) SELECT DISTINCT pack FROM other.packs", [])?;
                out.packs = tx.execute(
                    "INSERT INTO main.packs(logical,object,pack,offset,len) SELECT p.logical,p.object,p.pack,p.offset,p.len FROM other.packs p JOIN main.objects m ON m.logical=p.logical
                     WHERE instr(m.envelope,p.object) > 0 AND NOT EXISTS(SELECT 1 FROM main.packs q WHERE q.logical=p.logical AND q.object=p.object)
                     ON CONFLICT(logical) DO UPDATE SET object=excluded.object,pack=excluded.pack,offset=excluded.offset,len=excluded.len",
                    [],
                )?;
                if has("pack_games")? {
                    tx.execute("INSERT OR IGNORE INTO main.pack_games(pack,game,made_ms) SELECT pack,game,made_ms FROM other.pack_games WHERE pack IN (SELECT pack FROM main.packs)", [])?;
                }
                let now = ids("SELECT DISTINCT pack FROM main.packs")?;
                for p in before.difference(&now) {
                    tx.execute("DELETE FROM main.pack_games WHERE pack=?", [p])?;
                    retire(&tx, Uuid::parse_str(p)?)?;
                }
            }
            for t in ["profiles", "sessions", "game_settings"] {
                if has(t)? {
                    out.records += tx.execute(&format!("INSERT OR IGNORE INTO main.{t} SELECT * FROM other.{t}"), [])?;
                }
            }
            // Invalidate disk copies before committing. If the transaction fails,
            // hydration can recover the old versions from its unchanged object rows.
            for logical in &replaced {
                match std::fs::remove_file(catalog_path(store, logical)?) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
            tx.commit()?;
            c.execute("DETACH DATABASE other", [])?;
            Ok(out)
        })();
        let _ = std::fs::remove_file(&other);
        let out = result?;
        self.hydrate_catalog_locked(&lock).await?;
        Ok(out)
    }

    pub fn rotate_objects(&self) -> Result<()> {
        let mut c = self.db.connect()?;
        let tx = c.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut q = tx.prepare("SELECT logical,envelope FROM objects")?;
            let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let new = self.key(&self.config.active_key)?;
        for (logical, json) in rows {
            let e: Envelope = serde_json::from_str(&json)?;
            let next = e.rewrap(&self.key(&e.key_id)?, &new, &self.config.active_key)?;
            tx.execute(
                "UPDATE objects SET envelope=? WHERE logical=?",
                rusqlite::params![serde_json::to_string(&next)?, logical],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub async fn rotate_backup(&self, id: Uuid) -> Result<()> {
        let mut b: Backup = serde_json::from_slice(&self.download(id).await?)
            .map_err(|_| anyhow!("not a backup envelope"))?;
        b.envelope = b.envelope.rewrap(
            &self.key(&b.envelope.key_id)?,
            &self.key(&self.config.active_key)?,
            &self.config.active_key,
        )?;
        b.recovery = b.recovery.rewrap(
            &self.key(&b.recovery.key_id)?,
            &self.key(&self.config.active_key)?,
            &self.config.active_key,
        )?;
        self.upload(id, &serde_json::to_vec(&b)?, AtOnce::ALONE).await
    }
}

/// How long `bytes` take at `rate` bytes a second.
fn pace(bytes: u64, rate: u64) -> std::time::Duration {
    std::time::Duration::from_secs_f64(bytes as f64 / rate.max(1) as f64)
}

/// How many uploads share the line (each gets its share of it): 1 or more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AtOnce(std::num::NonZeroUsize);

impl AtOnce {
    const ALONE: AtOnce = AtOnce(std::num::NonZeroUsize::MIN);
    /// `n` at once (0, a typo in storage.toml, as 1).
    fn new(n: usize) -> AtOnce {
        AtOnce(std::num::NonZeroUsize::new(n).unwrap_or(std::num::NonZeroUsize::MIN))
    }
    fn get(self) -> usize {
        self.0.get()
    }
}

/// How long an upload of `len` bytes, one of `at_once`, may take: 60 s (rust-s3's for any
/// request: the round trips) and a second for each 128 KiB of the bytes going up together (an
/// uplink of 1 Mbit/s still gets there).
fn upload_timeout(len: usize, at_once: AtOnce) -> std::time::Duration {
    let wide = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let together = wide(len).saturating_mul(wide(at_once.get()));
    std::time::Duration::from_secs(60u64.saturating_add(together.div_ceil(128 << 10)))
}

/// Notes that `object` is no longer named (gc deletes it later).
fn retire(tx: &rusqlite::Transaction, object: Uuid) -> Result<()> {
    tx.execute("INSERT OR IGNORE INTO retired(object,retired_ms) VALUES(?,?)", rusqlite::params![object.to_string(), now_ms()])?;
    Ok(())
}

/// Registers backup `id` (descriptor; `snapshot`: its database's object) in the database `tx`
/// is on, pinning what its rows and packs name: the database restored from it is the snapshot
/// itself, so its own rows are what the backup needs. Rewrapping keeps each envelope's object.
fn register_backup(tx: &rusqlite::Transaction, id: Uuid, snapshot: Uuid, created_ms: u128) -> Result<()> {
    let (id, snapshot) = (id.to_string(), snapshot.to_string());
    tx.execute("INSERT OR IGNORE INTO backup_sets(id,snapshot) VALUES(?,?)", [&id, &snapshot])?;
    tx.execute(
        "INSERT OR IGNORE INTO backup_refs(id,object) SELECT ?1, json_extract(envelope,'$.object') FROM objects UNION SELECT ?1,pack FROM packs",
        [&id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO backups(id,snapshot,created_ms) VALUES(?,?,?)",
        rusqlite::params![id, snapshot, i64::try_from(created_ms)?],
    )?;
    Ok(())
}

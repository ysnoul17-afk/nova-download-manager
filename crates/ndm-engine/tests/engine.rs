//! Tests d'intégration du moteur contre un serveur HTTP local configurable.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use ndm_engine::*;
use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Serveur de test
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Opts {
    ranges: bool,
    content_length: bool,
    /// Pause entre deux blocs de 64 Ko.
    chunk_delay_ms: u64,
    /// Coupe la connexion après N octets, une fois par requête jusqu'à épuisement du compteur.
    drop_after: Option<u64>,
    drops_left: usize,
    disposition: Option<String>,
    /// Refuse au-delà de N connexions simultanées (503).
    max_conns: Option<usize>,
}

struct Srv {
    data: Mutex<Arc<Vec<u8>>>,
    etag: Mutex<String>,
    opts: Mutex<Opts>,
    /// Début de plage de chaque requête de données (None = sans Range).
    starts: Mutex<Vec<Option<u64>>>,
    current: AtomicUsize,
    peak: AtomicUsize,
    bytes_sent: AtomicU64,
}

fn make_data(size: usize, seed: u8) -> Vec<u8> {
    (0..size).map(|i| ((i as u64 * 31 + seed as u64 + (i as u64 >> 13)) % 251) as u8).collect()
}

struct ConnGuard(Arc<Srv>);
impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.current.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn handler(State(s): State<Arc<Srv>>, headers: HeaderMap) -> Response {
    let data = s.data.lock().clone();
    let etag = s.etag.lock().clone();
    let (ranges, content_length, delay, disposition, max_conns, drop_after) = {
        let mut o = s.opts.lock();
        let drop_after = if o.drops_left > 0 { o.drop_after } else { None };
        // Les requêtes de sonde (bytes=0-0) ne consomment pas de coupure.
        let is_probe = headers.get(header::RANGE).map(|v| v == "bytes=0-0").unwrap_or(false);
        if drop_after.is_some() && !is_probe {
            o.drops_left -= 1;
        }
        (o.ranges, o.content_length, o.chunk_delay_ms, o.disposition.clone(), o.max_conns, if is_probe { None } else { drop_after })
    };
    let total = data.len() as u64;

    let now = s.current.fetch_add(1, Ordering::SeqCst) + 1;
    let guard = ConnGuard(s.clone());
    if let Some(m) = max_conns {
        if now > m {
            return Response::builder().status(503).body(Body::empty()).unwrap();
        }
    }
    s.peak.fetch_max(now, Ordering::SeqCst);

    let mut range: Option<(u64, u64)> = None;
    if ranges {
        if let Some(r) = headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
            let ok_if_range = headers
                .get(header::IF_RANGE)
                .map(|v| v.to_str().unwrap_or("") == etag)
                .unwrap_or(true);
            if ok_if_range {
                let spec = r.trim_start_matches("bytes=");
                let (a, b) = spec.split_once('-').unwrap();
                let a: u64 = a.parse().unwrap();
                let b: u64 = if b.is_empty() { total - 1 } else { b.parse::<u64>().unwrap().min(total - 1) };
                range = Some((a, b));
            }
        }
    }
    if range.map(|(a, _)| a != 0 || true).unwrap_or(true) {
        let is_probe = range == Some((0, 0));
        if !is_probe {
            s.starts.lock().push(range.map(|(a, _)| a));
        }
    }

    let (start, end) = range.unwrap_or((0, total.saturating_sub(1)));
    let slice_len = if total == 0 { 0 } else { end + 1 - start };

    let mut b = Response::builder()
        .header(header::ETAG, etag.clone())
        .header(header::CONTENT_TYPE, "application/octet-stream");
    if ranges {
        b = b.header(header::ACCEPT_RANGES, "bytes");
    }
    if let Some(d) = disposition {
        b = b.header(header::CONTENT_DISPOSITION, d);
    }
    if let Some((a, z)) = range {
        b = b.status(StatusCode::PARTIAL_CONTENT).header(header::CONTENT_RANGE, format!("bytes {a}-{z}/{total}"));
    } else {
        b = b.status(StatusCode::OK);
    }
    if content_length {
        b = b.header(header::CONTENT_LENGTH, slice_len);
    }

    let srv = s.clone();
    let stream = async_stream(data, start, slice_len, delay, drop_after, srv, guard);
    b.body(Body::from_stream(stream)).unwrap()
}

fn async_stream(
    data: Arc<Vec<u8>>,
    start: u64,
    len: u64,
    delay: u64,
    drop_after: Option<u64>,
    srv: Arc<Srv>,
    guard: ConnGuard,
) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    futures_util::stream::unfold((0u64, guard), move |(sent, guard)| {
        let data = data.clone();
        let srv = srv.clone();
        async move {
            if sent >= len {
                return None;
            }
            if let Some(d) = drop_after {
                if sent >= d {
                    return Some((Err(std::io::Error::other("coupure simulée")), (len, guard)));
                }
            }
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            let n = (64 * 1024).min(len - sent);
            let from = (start + sent) as usize;
            let chunk = data[from..from + n as usize].to_vec();
            srv.bytes_sent.fetch_add(n, Ordering::Relaxed);
            Some((Ok(chunk), (sent + n, guard)))
        }
    })
}

async fn spawn_server(size: usize, opts: Opts) -> (String, Arc<Srv>) {
    let srv = Arc::new(Srv {
        data: Mutex::new(Arc::new(make_data(size, 7))),
        etag: Mutex::new("\"v1\"".into()),
        opts: Mutex::new(opts),
        starts: Mutex::new(vec![]),
        current: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        bytes_sent: AtomicU64::new(0),
    });
    let app = Router::new().route("/{*path}", get(handler)).with_state(srv.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), srv)
}

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

fn config(dir: &Path) -> EngineConfig {
    EngineConfig {
        base_directory: dir.join("dl"),
        min_segment_size: 256 * 1024,
        connect_timeout_secs: 5,
        read_timeout_secs: 5,
        ..Default::default()
    }
}

async fn wait_for(engine: &Engine, id: DownloadId, timeout: Duration, pred: impl Fn(&DownloadInfo) -> bool) -> DownloadInfo {
    let t = Instant::now();
    loop {
        let info = engine.get(id).expect("téléchargement présent");
        if pred(&info) {
            return info;
        }
        if t.elapsed() > timeout {
            panic!("délai dépassé, état : {info:#?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_done(engine: &Engine, id: DownloadId, secs: u64) -> DownloadInfo {
    wait_for(engine, id, Duration::from_secs(secs), |i| {
        matches!(i.status, DownloadStatus::Completed | DownloadStatus::Failed)
    })
    .await
}

fn assert_file(info: &DownloadInfo, expected: &[u8]) {
    assert_eq!(info.status, DownloadStatus::Completed, "erreur : {:?}", info.error);
    let got = std::fs::read(info.full_path()).expect("fichier final");
    assert_eq!(got.len(), expected.len(), "taille");
    assert!(got == expected, "contenu différent");
    assert!(!ndm_engine::download::part_path(&info.full_path()).exists(), "partiel restant");
}

fn add(engine: &Engine, url: String) -> DownloadInfo {
    engine.add(AddRequest { url, ..Default::default() }).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multisegment_download() {
    let size = 20 * 1024 * 1024 + 123;
    let (base, srv) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 2, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();

    let info = add(&engine, format!("{base}/videos/film.mp4"));
    let done = wait_done(&engine, info.id, 60).await;
    assert_file(&done, &make_data(size, 7));
    assert_eq!(done.filename, "film.mp4");
    assert_eq!(done.category, Category::Video);
    assert!(done.directory.ends_with("Vidéos"));
    assert!(done.resumable);
    let peak = srv.peak.load(Ordering::SeqCst);
    assert!(peak >= 4, "connexions simultanées : {peak}");
    // Pas de téléchargement en double : on n'envoie guère plus que la taille du fichier.
    let sent = srv.bytes_sent.load(Ordering::Relaxed);
    assert!(sent < size as u64 + 8 * 1024 * 1024, "octets envoyés : {sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_without_ranges() {
    let size = 3 * 1024 * 1024;
    let (base, srv) = spawn_server(size, Opts { ranges: false, content_length: true, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();

    let info = add(&engine, format!("{base}/archive.zip"));
    let done = wait_done(&engine, info.id, 30).await;
    assert_file(&done, &make_data(size, 7));
    assert!(!done.resumable);
    // Une seule connexion de données ; la requête d'analyse (abandonnée) peut
    // encore être ouverte côté serveur au moment où le transfert démarre.
    assert!(srv.peak.load(Ordering::SeqCst) <= 2);
    assert_eq!(done.category, Category::Archive);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_size_stream() {
    let size = 1_500_000;
    let (base, _) = spawn_server(size, Opts { ranges: false, content_length: false, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();

    let info = add(&engine, format!("{base}/flux.bin"));
    let done = wait_done(&engine, info.id, 30).await;
    assert_file(&done, &make_data(size, 7));
    assert_eq!(done.total_size, Some(size as u64));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_file() {
    let (base, _) = spawn_server(0, Opts { ranges: false, content_length: true, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let info = add(&engine, format!("{base}/vide.txt"));
    let done = wait_done(&engine, info.id, 15).await;
    assert_file(&done, &[]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_then_resume_continues_where_it_stopped() {
    let size = 8 * 1024 * 1024;
    let (base, srv) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 20, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();

    let info = add(&engine, format!("{base}/doc.pdf"));
    wait_for(&engine, info.id, Duration::from_secs(20), |i| i.downloaded > 2 * 1024 * 1024).await;
    engine.pause(info.id).unwrap();
    let paused = wait_for(&engine, info.id, Duration::from_secs(5), |i| i.status == DownloadStatus::Paused && i.active_connections == 0).await;
    let at_pause = paused.downloaded;
    assert!(at_pause > 0 && at_pause < size as u64);
    assert!(ndm_engine::download::part_path(&paused.full_path()).exists());

    // Rien ne doit progresser pendant la pause.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(engine.get(info.id).unwrap().downloaded, at_pause);

    let before = srv.starts.lock().len();
    srv.opts.lock().chunk_delay_ms = 0;
    engine.resume(info.id).unwrap();
    let done = wait_done(&engine, info.id, 30).await;
    assert_file(&done, &make_data(size, 7));
    // Après reprise, les requêtes repartent du milieu des segments, pas de zéro.
    let after: Vec<_> = srv.starts.lock()[before..].to_vec();
    assert!(!after.is_empty());
    assert!(after.iter().all(|s| matches!(s, Some(x) if *x > 0)), "requêtes après reprise : {after:?}");
    let sent = srv.bytes_sent.load(Ordering::Relaxed);
    assert!(sent < size as u64 + 3 * 1024 * 1024, "re-téléchargement excessif : {sent}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn survives_engine_restart() {
    let size = 6 * 1024 * 1024;
    let (base, srv) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 20, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();

    let id = {
        let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
        let info = add(&engine, format!("{base}/setup.exe"));
        wait_for(&engine, info.id, Duration::from_secs(20), |i| i.downloaded > 1024 * 1024).await;
        engine.shutdown().await; // fermeture de l'application
        info.id
    };

    srv.opts.lock().chunk_delay_ms = 0;
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let done = wait_done(&engine, id, 30).await;
    assert_file(&done, &make_data(size, 7));
    assert_eq!(done.category, Category::Software);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_after_dropped_connections() {
    let size = 4 * 1024 * 1024;
    let (base, _) = spawn_server(
        size,
        Opts { ranges: true, content_length: true, drop_after: Some(300 * 1024), drops_left: 6, ..Default::default() },
    )
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let info = add(&engine, format!("{base}/image.png"));
    let done = wait_done(&engine, info.id, 60).await;
    assert_file(&done, &make_data(size, 7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_limited_server() {
    let size = 5 * 1024 * 1024;
    let (base, _) = spawn_server(
        size,
        Opts { ranges: true, content_length: true, max_conns: Some(2), chunk_delay_ms: 1, ..Default::default() },
    )
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let info = add(&engine, format!("{base}/musique.mp3"));
    let done = wait_done(&engine, info.id, 60).await;
    assert_file(&done, &make_data(size, 7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn content_disposition_and_duplicates() {
    let size = 100_000;
    let (base, _) = spawn_server(
        size,
        Opts {
            ranges: true,
            content_length: true,
            disposition: Some("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf".into()),
            ..Default::default()
        },
    )
    .await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let a = add(&engine, format!("{base}/get?id=1"));
    let a = wait_done(&engine, a.id, 15).await;
    let b = add(&engine, format!("{base}/get?id=1"));
    let b = wait_done(&engine, b.id, 15).await;
    assert_eq!(a.filename, "résumé.pdf");
    assert_eq!(b.filename, "résumé (1).pdf");
    assert_eq!(a.category, Category::Document);
    assert_file(&a, &make_data(size, 7));
    assert_file(&b, &make_data(size, 7));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_speed_limit() {
    let size = 3 * 1024 * 1024;
    let (base, _) = spawn_server(size, Opts { ranges: true, content_length: true, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = config(tmp.path());
    cfg.global_speed_limit = 1024 * 1024; // 1 Mo/s
    let engine = Engine::start(tmp.path(), cfg).unwrap();
    let t = Instant::now();
    let info = add(&engine, format!("{base}/lent.bin"));
    let done = wait_done(&engine, info.id, 30).await;
    let secs = t.elapsed().as_secs_f64();
    assert_file(&done, &make_data(size, 7));
    assert!(secs >= 2.3, "trop rapide pour 1 Mo/s : {secs:.2} s");
    assert!(secs < 6.0, "trop lent : {secs:.2} s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_file_changed_restarts_cleanly() {
    let size = 4 * 1024 * 1024;
    let (base, srv) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 20, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let info = add(&engine, format!("{base}/maj.bin"));
    wait_for(&engine, info.id, Duration::from_secs(20), |i| i.downloaded > 512 * 1024).await;
    engine.pause(info.id).unwrap();
    wait_for(&engine, info.id, Duration::from_secs(5), |i| i.active_connections == 0).await;

    // Le serveur publie une nouvelle version du fichier.
    let new_data = make_data(size, 99);
    *srv.data.lock() = Arc::new(new_data.clone());
    *srv.etag.lock() = "\"v2\"".into();
    srv.opts.lock().chunk_delay_ms = 0;

    engine.resume(info.id).unwrap();
    let done = wait_done(&engine, info.id, 30).await;
    assert_file(&done, &new_data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queue_respects_concurrency_and_priority() {
    let size = 1024 * 1024;
    let (base, _) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 10, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = config(tmp.path());
    cfg.max_concurrent_downloads = 1;
    let engine = Engine::start(tmp.path(), cfg).unwrap();

    let first = add(&engine, format!("{base}/1.bin"));
    let low = engine.add(AddRequest { url: format!("{base}/2.bin"), priority: Priority::Low, ..Default::default() }).unwrap();
    let high = engine.add(AddRequest { url: format!("{base}/3.bin"), priority: Priority::High, ..Default::default() }).unwrap();

    // Un seul actif à la fois.
    for _ in 0..10 {
        let active = engine.list().iter().filter(|i| i.status.is_active()).count();
        assert!(active <= 1);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let h = wait_done(&engine, high.id, 30).await;
    let l = wait_done(&engine, low.id, 30).await;
    let f = engine.get(first.id).unwrap();
    assert_eq!(f.status, DownloadStatus::Completed);
    // La priorité haute passe avant la basse, même ajoutée après.
    assert!(h.completed_at.unwrap() <= l.completed_at.unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_deletes_partial_and_optionally_file() {
    let size = 4 * 1024 * 1024;
    let (base, _) = spawn_server(size, Opts { ranges: true, content_length: true, chunk_delay_ms: 20, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let info = add(&engine, format!("{base}/a.bin"));
    let mid = wait_for(&engine, info.id, Duration::from_secs(20), |i| i.downloaded > 0 && !i.filename.is_empty()).await;
    engine.remove(info.id, false).await.unwrap();
    assert!(engine.get(info.id).is_none());
    assert!(!ndm_engine::download::part_path(&mid.full_path()).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_bad_urls() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    assert!(engine.add(AddRequest { url: "ftp://x/y".into(), ..Default::default() }).is_err());
    assert!(engine.add(AddRequest { url: "pas une url".into(), ..Default::default() }).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_404_fails_with_message() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let app = Router::new().route("/{*p}", get(|| async { StatusCode::NOT_FOUND }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let info = add(&engine, format!("http://{addr}/absent.zip"));
    let done = wait_done(&engine, info.id, 15).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert!(done.error.unwrap().contains("404"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn history_is_persisted_and_searchable() {
    let (base, _) = spawn_server(200_000, Opts { ranges: true, content_length: true, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    {
        let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
        for name in ["vacances.mp4", "facture.pdf", "album.zip"] {
            let i = add(&engine, format!("{base}/{name}"));
            wait_done(&engine, i.id, 15).await;
        }
        let mut cfg = engine.config();
        cfg.connections_per_download = 99; // ramené à 32
        cfg.max_concurrent_downloads = 3;
        assert_eq!(engine.set_config(cfg).unwrap().connections_per_download, 32);
        engine.shutdown().await;
    }
    // Après redémarrage : historique et réglages conservés.
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    assert_eq!(engine.list().len(), 3);
    assert_eq!(engine.config().max_concurrent_downloads, 3);

    let r = engine.query(HistoryQuery { search: Some("FACT".into()), ..Default::default() }).await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].filename, "facture.pdf");

    let r = engine.query(HistoryQuery { categories: vec![Category::Video, Category::Archive], ..Default::default() }).await.unwrap();
    assert_eq!(r.len(), 2);

    let r = engine.query(HistoryQuery { sort: SortKey::Name, ..Default::default() }).await.unwrap();
    let names: Vec<_> = r.iter().map(|i| i.filename.as_str()).collect();
    assert_eq!(names, ["album.zip", "facture.pdf", "vacances.mp4"]);

    let r = engine.query(HistoryQuery { statuses: vec![DownloadStatus::Failed], ..Default::default() }).await.unwrap();
    assert!(r.is_empty());
    assert!(engine.list().iter().all(|i| i.avg_speed.is_some()));

    // Recherche avec caractères spéciaux SQL : pas d'injection, pas de faux positifs.
    let r = engine.query(HistoryQuery { search: Some("%' OR 1=1 --".into()), ..Default::default() }).await.unwrap();
    assert!(r.is_empty());

    let id = engine.list()[0].id;
    engine.remove(id, true).await.unwrap();
    drop(engine);
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    assert_eq!(engine.list().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejects_directory_traversal() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let bad = AddRequest {
        url: "http://exemple.com/a.zip".into(),
        directory: Some(tmp.path().join("..").join("ailleurs")),
        ..Default::default()
    };
    assert!(engine.add(bad).is_err());
    let rel = AddRequest { url: "http://exemple.com/a.zip".into(), directory: Some("relatif".into()), ..Default::default() };
    assert!(engine.add(rel).is_err());
    // Un nom de fichier malveillant est neutralisé.
    let evil = AddRequest {
        url: "http://exemple.com/a.zip".into(),
        filename: Some("..\\..\\Windows\\evil.exe".into()),
        start_paused: true,
        ..Default::default()
    };
    let i = engine.add(evil).unwrap();
    assert!(!i.filename.contains('\\') && !i.filename.contains('/'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finds_completed_duplicates() {
    let (base, _) = spawn_server(50_000, Opts { ranges: true, content_length: true, ..Default::default() }).await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = Engine::start(tmp.path(), config(tmp.path())).unwrap();
    let url = format!("{base}/setup.exe");
    assert!(engine.find_completed(&url, None).is_none());
    let i = add(&engine, url.clone());
    let done = wait_done(&engine, i.id, 15).await;
    assert_eq!(engine.find_completed(&url, None).map(|d| d.id), Some(done.id));
    // Une vidéo de la même adresse n'est pas un doublon d'un fichier, et inversement.
    assert!(engine.find_completed(&url, Some("1080p MP4")).is_none());
    // Fichier supprimé du disque : plus considéré comme téléchargé.
    std::fs::remove_file(done.full_path()).unwrap();
    assert!(engine.find_completed(&url, None).is_none());
}

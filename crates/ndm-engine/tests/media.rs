//! Tests de bout en bout du téléchargement vidéo avec le vrai yt-dlp et le vrai FFmpeg.
//!
//! Ils nécessitent :
//! - `NDM_TEST_YTDLP` : chemin de yt-dlp ;
//! - `ffmpeg` et `ffprobe` dans le PATH.
//!
//! Sans ces outils, les tests s'affichent comme réussis mais ne font rien (message sur stderr).
//! Un flux HLS multi-qualités (vidéo et audio séparés, comme YouTube) est généré puis servi localement.

use axum::extract::{Path as AxPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use ndm_engine::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn which(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&file)).find(|p| p.is_file())
}

fn tools() -> Option<MediaTools> {
    let ytdlp = PathBuf::from(std::env::var_os("NDM_TEST_YTDLP")?);
    let ffmpeg = which("ffmpeg")?;
    which("ffprobe")?;
    Some(MediaTools { ytdlp: Some(ytdlp), ffmpeg_dir: ffmpeg.parent().map(Path::to_path_buf), ytdlp_version: None })
}

macro_rules! require_tools {
    () => {
        match tools() {
            Some(t) => t,
            None => {
                eprintln!("test ignoré : définir NDM_TEST_YTDLP et installer ffmpeg/ffprobe");
                return;
            }
        }
    };
}

fn ff(args: &[&str]) {
    let ok = Command::new("ffmpeg").args(["-loglevel", "error", "-y"]).args(args).status().unwrap().success();
    assert!(ok, "ffmpeg {args:?}");
}

/// Génère un flux HLS : 3 résolutions vidéo seules + 1 piste audio, et une page HTML5.
fn make_media(dir: &Path) {
    for (size, h, br) in [("1280x720", "720", "1500k"), ("854x480", "480", "700k"), ("640x360", "360", "400k")] {
        ff(&[
            "-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate=25:duration=6"),
            "-c:v", "libx264", "-preset", "ultrafast", "-b:v", br, "-g", "50", "-an",
            "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod",
            "-hls_segment_filename", dir.join(format!("v{h}_%03d.ts")).to_str().unwrap(),
            dir.join(format!("v{h}.m3u8")).to_str().unwrap(),
        ]);
    }
    ff(&[
        "-f", "lavfi", "-i", "sine=frequency=440:duration=6", "-c:a", "aac", "-b:a", "96k",
        "-f", "hls", "-hls_time", "2", "-hls_playlist_type", "vod",
        "-hls_segment_filename", dir.join("a_%03d.ts").to_str().unwrap(),
        dir.join("a.m3u8").to_str().unwrap(),
    ]);
    std::fs::write(
        dir.join("master.m3u8"),
        "#EXTM3U\n#EXT-X-VERSION:3\n\
         #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"fr\",LANGUAGE=\"fr\",DEFAULT=YES,AUTOSELECT=YES,URI=\"a.m3u8\"\n\
         #EXT-X-STREAM-INF:BANDWIDTH=1700000,RESOLUTION=1280x720,CODECS=\"avc1.64001f,mp4a.40.2\",AUDIO=\"aud\"\nv720.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=854x480,CODECS=\"avc1.64001e,mp4a.40.2\",AUDIO=\"aud\"\nv480.m3u8\n\
         #EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=640x360,CODECS=\"avc1.64001e,mp4a.40.2\",AUDIO=\"aud\"\nv360.m3u8\n",
    )
    .unwrap();
    ff(&[
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=25:duration=4", "-f", "lavfi", "-i", "sine=duration=4",
        "-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest",
        dir.join("clip.mp4").to_str().unwrap(),
    ]);
    std::fs::write(
        dir.join("page.html"),
        "<!doctype html><html><head><meta charset=utf-8><title>Démo produit</title></head>\
         <body><video controls src=\"clip.mp4\"></video></body></html>",
    )
    .unwrap();
}

async fn serve_file(State(dir): State<Arc<PathBuf>>, AxPath(name): AxPath<String>) -> Response {
    if name.contains("..") || name.contains('/') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match std::fs::read(dir.join(&name)) {
        Ok(data) => {
            let ct = match name.rsplit('.').next() {
                Some("m3u8") => "application/vnd.apple.mpegurl",
                Some("ts") => "video/mp2t",
                Some("mp4") => "video/mp4",
                Some("html") => "text/html; charset=utf-8",
                _ => "application/octet-stream",
            };
            ([(header::CONTENT_TYPE, ct)], data).into_response()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn spawn_media_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    make_media(dir.path());
    let app = Router::new().route("/{name}", get(serve_file)).with_state(Arc::new(dir.path().to_path_buf()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), dir)
}

/// (hauteur, nombre de pistes audio) via ffprobe.
fn probe_streams(path: &Path) -> (Option<u32>, usize) {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "stream=codec_type,height", "-of", "csv=p=0"])
        .arg(path)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let mut height = None;
    let mut audio = 0;
    for l in text.lines() {
        let mut p = l.split(',');
        match p.next() {
            Some("video") => height = p.next().and_then(|h| h.trim().parse().ok()),
            Some("audio") => audio += 1,
            _ => {}
        }
    }
    (height, audio)
}

async fn wait_done(engine: &Engine, id: DownloadId, secs: u64) -> DownloadInfo {
    let t = Instant::now();
    loop {
        let i = engine.get(id).unwrap();
        if matches!(i.status, DownloadStatus::Completed | DownloadStatus::Failed) {
            return i;
        }
        assert!(t.elapsed() < Duration::from_secs(secs), "délai dépassé : {i:#?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn spec_for(info: &MediaInfo, id: &str) -> MediaSpec {
    let o = info.options.iter().find(|o| o.id == id).unwrap_or_else(|| panic!("option {id} absente"));
    MediaSpec {
        format: o.format,
        subtitles: false,
        label: format!("{} {}", o.label, o.container),
        title: info.title.clone(),
        est_size: o.est_size,
    }
}

async fn engine_with(tools: MediaTools, dir: &Path) -> Engine {
    let cfg = EngineConfig { base_directory: dir.join("dl"), ..Default::default() };
    let e = Engine::start(dir.join("data"), cfg).unwrap();
    e.set_media_tools(tools);
    e
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn analyzes_multi_quality_hls() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let info = engine.analyze_media(&format!("{base}/master.m3u8")).await.unwrap();
    let ids: Vec<_> = info.options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["v720", "v480", "v360", "mp3", "m4a"]);
    assert!(info.duration_secs.unwrap() > 5.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_selected_quality_and_merges_audio() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let url = format!("{base}/master.m3u8");
    let info = engine.analyze_media(&url).await.unwrap();

    let req = AddRequest { url: url.clone(), media: Some(spec_for(&info, "v480")), ..Default::default() };
    let added = engine.add(req).unwrap();
    assert_eq!(added.category, Category::Video);
    let done = wait_done(&engine, added.id, 120).await;
    assert_eq!(done.status, DownloadStatus::Completed, "erreur : {:?}", done.error);
    assert!(done.filename.ends_with(".mp4"), "{}", done.filename);
    assert!(done.directory.ends_with("Vidéos"));
    let (height, audio) = probe_streams(&done.full_path());
    assert_eq!(height, Some(480), "la qualité demandée est respectée");
    assert_eq!(audio, 1, "la piste audio séparée a été fusionnée");
    assert_eq!(done.total_size, Some(std::fs::metadata(done.full_path()).unwrap().len()));
    assert_eq!(done.media_label.as_deref(), Some("480p MP4"));
    // L'estimation affichée avant le téléchargement est proche de la taille réelle.
    let est = info.options.iter().find(|o| o.id == "v480").unwrap().est_size.unwrap() as f64;
    let real = done.total_size.unwrap() as f64;
    assert!((est - real).abs() / real < 0.25, "estimation {est} pour {real} octets réels");

    // Aucun fichier temporaire ne doit rester.
    let leftovers: Vec<_> = std::fs::read_dir(&done.directory)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != &done.filename)
        .collect();
    assert!(leftovers.is_empty(), "fichiers restants : {leftovers:?}");

    // Même vidéo une seconde fois : nouveau nom, pas d'écrasement.
    let again = engine
        .add(AddRequest { url, media: Some(spec_for(&info, "v360")), ..Default::default() })
        .unwrap();
    let again = wait_done(&engine, again.id, 120).await;
    assert_eq!(again.status, DownloadStatus::Completed, "erreur : {:?}", again.error);
    assert_ne!(again.filename, done.filename);
    assert_eq!(probe_streams(&again.full_path()).0, Some(360));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn extracts_mp3_into_music() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let url = format!("{base}/master.m3u8");
    let info = engine.analyze_media(&url).await.unwrap();
    let added = engine.add(AddRequest { url, media: Some(spec_for(&info, "mp3")), ..Default::default() }).unwrap();
    assert_eq!(added.category, Category::Music);
    let done = wait_done(&engine, added.id, 120).await;
    assert_eq!(done.status, DownloadStatus::Completed, "erreur : {:?}", done.error);
    assert!(done.filename.ends_with(".mp3"));
    assert!(done.directory.ends_with("Musique"));
    let (height, audio) = probe_streams(&done.full_path());
    assert_eq!((height, audio), (None, 1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn html5_page_and_batch_without_analysis() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let url = format!("{base}/page.html");
    let info = engine.analyze_media(&url).await.unwrap();
    assert_eq!(info.options[0].format, MediaFormat::Best);

    // Ajout par lots : titre inconnu, l'analyse se fait au démarrage.
    let spec = MediaSpec { format: MediaFormat::Best, subtitles: false, label: "Meilleure qualité MP4".into(), title: String::new(), est_size: None };
    let added = engine.add(AddRequest { url, media: Some(spec), ..Default::default() }).unwrap();
    let done = wait_done(&engine, added.id, 120).await;
    assert_eq!(done.status, DownloadStatus::Completed, "erreur : {:?}", done.error);
    assert!(done.filename.starts_with("Démo produit"), "{}", done.filename);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explains_pages_without_video_and_missing_tools() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(_keep.path().join("vide.html"), "<html><body>Pas de vidéo</body></html>").unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let err = engine.analyze_media(&format!("{base}/vide.html")).await.unwrap_err().to_string();
    assert!(err.contains("Aucune vidéo"), "{err}");

    // Sans outils : échec clair au lieu d'un plantage.
    engine.set_media_tools(MediaTools::default());
    let spec = MediaSpec { format: MediaFormat::Best, subtitles: false, label: "x".into(), title: "x".into(), est_size: None };
    let added = engine.add(AddRequest { url: format!("{base}/page.html"), media: Some(spec), ..Default::default() }).unwrap();
    let done = wait_done(&engine, added.id, 30).await;
    assert_eq!(done.status, DownloadStatus::Failed);
    assert!(done.error.unwrap().contains("yt-dlp n'est pas installé"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_kills_ytdlp_and_resume_finishes() {
    let tools = require_tools!();
    let (base, _keep) = spawn_media_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine_with(tools, tmp.path()).await;
    let url = format!("{base}/master.m3u8");
    let info = engine.analyze_media(&url).await.unwrap();
    // Limite de vitesse basse pour avoir le temps de mettre en pause.
    engine.set_global_speed_limit(150_000).unwrap();
    let added = engine.add(AddRequest { url, media: Some(spec_for(&info, "v720")), ..Default::default() }).unwrap();
    let t = Instant::now();
    loop {
        let i = engine.get(added.id).unwrap();
        if i.downloaded > 0 {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(60), "pas de progression : {i:#?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    engine.pause(added.id).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let paused = engine.get(added.id).unwrap();
    assert_eq!(paused.status, DownloadStatus::Paused);
    // Aucun yt-dlp ne doit continuer à écrire pendant la pause.
    let size_of_dir = |d: &Path| -> u64 {
        std::fs::read_dir(d).map(|r| r.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0)
    };
    let s1 = size_of_dir(&paused.directory);
    if std::env::var_os("NDM_DEBUG").is_some() {
        let o = Command::new("sh").args(["-c", "ps -eo pid,pgid,ppid,stat,args | grep -E 'yt-dlp|ffmpeg' | grep -v grep"]).output().unwrap();
        eprintln!("PS après pause:\n{}", String::from_utf8_lossy(&o.stdout));
        for e in std::fs::read_dir(&paused.directory).unwrap().flatten() { eprintln!("  {:?} {}", e.file_name(), e.metadata().unwrap().len()); }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(size_of_dir(&paused.directory), s1, "le processus a été arrêté");

    engine.set_global_speed_limit(0).unwrap();
    engine.resume(added.id).unwrap();
    let done = wait_done(&engine, added.id, 120).await;
    assert_eq!(done.status, DownloadStatus::Completed, "erreur : {:?}", done.error);
    assert_eq!(probe_streams(&done.full_path()), (Some(720), 1));
}

/// Installation réelle depuis GitHub (réseau requis) : `cargo test --test media -- --ignored`.
#[tokio::test]
#[ignore]
async fn installs_ytdlp_with_verified_checksum() {
    let tmp = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();
    let path = ndm_engine::tools::install_ytdlp(&client, tmp.path(), &|_, _| {}).await.unwrap();
    let status = ndm_engine::tools::detect(tmp.path()).await;
    let y = status.ytdlp.expect("yt-dlp détecté");
    assert_eq!(y.path, path);
    assert!(y.managed);
    assert!(y.version.unwrap().starts_with("20"), "version au format AAAA.MM.JJ");
}

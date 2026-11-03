//! Exécution d'un téléchargement : analyse, segmentation dynamique, connexions
//! parallèles, reprise et finalisation.
//!
//! Principe (proche d'IDM) : le fichier est découpé en N segments. Quand une
//! connexion termine le sien, elle « vole » la moitié restante du segment le plus
//! en retard. Les connexions rapides absorbent donc le travail des lentes.

use crate::error::{EngineError, Result};
use crate::filename;
use crate::limiter::{self, RateLimiter};
use crate::types::*;
use futures_util::StreamExt;
use parking_lot::{Mutex, RwLock};
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

/// Taille du tampon par connexion avant écriture disque.
const WRITE_BUFFER: usize = 512 * 1024;
pub const PART_EXT: &str = "ndmpart";

pub fn part_path(final_path: &Path) -> PathBuf {
    let mut s = final_path.as_os_str().to_owned();
    s.push(".");
    s.push(PART_EXT);
    PathBuf::from(s)
}

/// Identifiants de version du fichier distant, pour détecter un changement à la reprise.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl Validator {
    fn if_range(&self) -> Option<&str> {
        // Un ETag faible (W/) n'est pas accepté dans If-Range.
        match &self.etag {
            Some(e) if !e.starts_with("W/") => Some(e),
            _ => self.last_modified.as_deref(),
        }
    }
    /// Deux validateurs sont incompatibles si une même information diffère.
    fn conflicts_with(&self, other: &Validator) -> bool {
        let differ = |a: &Option<String>, b: &Option<String>| matches!((a, b), (Some(x), Some(y)) if x != y);
        differ(&self.etag, &other.etag) || differ(&self.last_modified, &other.last_modified)
    }
}

/// État persistant d'un téléchargement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: DownloadId,
    pub url: String,
    pub final_url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Vide tant que le nom n'est pas connu.
    pub filename: String,
    /// Nom imposé par l'utilisateur.
    pub filename_locked: bool,
    pub directory: PathBuf,
    pub directory_locked: bool,
    pub category: Category,
    pub status: DownloadStatus,
    pub priority: Priority,
    pub total_size: Option<u64>,
    pub size_hint: Option<u64>,
    pub resumable: bool,
    pub segments: Vec<Segment>,
    pub validator: Validator,
    pub mime: Option<String>,
    pub max_connections: Option<u8>,
    /// Limite propre au fichier, octets/s (0 = aucune).
    #[serde(default)]
    pub speed_limit: u64,
    pub error: Option<String>,
    pub added_at: u64,
    pub completed_at: Option<u64>,
    /// Ordre d'arrivée, pour départager les priorités égales.
    pub seq: u64,
    /// Temps de téléchargement effectif cumulé (ms).
    #[serde(default)]
    pub active_ms: u64,
    /// Vitesse moyenne constatée à la fin (octets/s).
    #[serde(default)]
    pub avg_speed: Option<u64>,
    /// Téléchargement vidéo (yt-dlp).
    #[serde(default)]
    pub media: Option<crate::media::MediaSpec>,
    #[serde(default)]
    pub stage: Option<crate::media::Stage>,
}

impl DownloadRecord {
    pub fn final_path(&self) -> Option<PathBuf> {
        (!self.filename.is_empty()).then(|| self.directory.join(&self.filename))
    }

    pub fn downloaded(&self) -> u64 {
        self.segments
            .iter()
            .map(|s| match s.len() {
                Some(l) => s.downloaded.min(l),
                None => s.downloaded,
            })
            .sum()
    }

    /// Base des fichiers yt-dlp (dossier + nom sans extension).
    pub fn media_stem(&self) -> Option<PathBuf> {
        let p = self.final_path()?;
        let stem = p.file_stem()?.to_os_string();
        Some(p.with_file_name(stem))
    }

    /// Supprime les fichiers temporaires (partiel HTTP ou fichiers intermédiaires yt-dlp).
    pub fn remove_partials(&self) {
        if let Some(p) = self.final_path() {
            let _ = std::fs::remove_file(part_path(&p));
        }
        if self.media.is_some() {
            if let Some(stem) = self.media_stem() {
                crate::media::cleanup_partials(&stem);
            }
        }
    }

    pub fn reset_progress(&mut self) {
        self.stage = None;
        self.segments.clear();
        self.total_size = None;
        self.resumable = false;
        self.validator = Validator::default();
    }
}

/// Partie partagée entre le gestionnaire et les tâches de téléchargement.
pub struct DownloadShared {
    pub record: Mutex<DownloadRecord>,
    /// Octets reçus (monotone), sert au calcul de la vitesse.
    pub received: AtomicU64,
    pub active_connections: AtomicU32,
    pub limiter: RateLimiter,
}

impl DownloadShared {
    pub fn new(record: DownloadRecord) -> Self {
        let limit = record.speed_limit;
        Self {
            record: Mutex::new(record),
            received: AtomicU64::new(0),
            active_connections: AtomicU32::new(0),
            limiter: RateLimiter::new(limit),
        }
    }
}

/// Contexte global du moteur.
pub struct EngineCtx {
    pub client: Client,
    pub config: RwLock<EngineConfig>,
    pub global_limiter: RateLimiter,
    pub media_tools: RwLock<crate::media::MediaTools>,
}

impl EngineCtx {
    pub fn new(config: EngineConfig) -> Result<Self> {
        let client = Client::builder()
            .user_agent(config.user_agent.clone())
            .connect_timeout(Duration::from_secs(config.connect_timeout_secs))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        Ok(Self {
            global_limiter: RateLimiter::new(config.global_speed_limit),
            media_tools: RwLock::new(Default::default()),
            client,
            config: RwLock::new(config),
        })
    }
}

// ---------------------------------------------------------------------------
// Analyse de l'URL
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub final_url: String,
    pub total_size: Option<u64>,
    pub accept_ranges: bool,
    pub filename: Option<String>,
    pub mime: Option<String>,
    pub validator: Validator,
}

fn build_headers(extra: &BTreeMap<String, String>) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in extra {
        let lk = k.to_ascii_lowercase();
        // En-têtes gérés par le moteur : on ignore ceux venant du navigateur.
        if matches!(
            lk.as_str(),
            "range" | "if-range" | "accept-encoding" | "content-length" | "host" | "connection"
        ) {
            continue;
        }
        if let (Ok(name), Ok(val)) = (HeaderName::try_from(k.as_str()), HeaderValue::from_str(v)) {
            h.insert(name, val);
        }
    }
    // Pas de compression : elle fausserait les tailles et les plages d'octets.
    h.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    h
}

/// Analyse `Content-Range: bytes a-b/total`.
fn parse_content_range(v: &str) -> Option<(u64, u64, Option<u64>)> {
    let v = v.trim().strip_prefix("bytes")?.trim();
    let (range, total) = v.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?, total))
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(|s| s.to_string())
}

pub async fn probe(ctx: &EngineCtx, url: &str, headers: &BTreeMap<String, String>) -> Result<ProbeResult> {
    // Un GET avec `Range: bytes=0-0` est plus fiable qu'un HEAD (souvent mal géré).
    let resp = ctx
        .client
        .get(url)
        .headers(build_headers(headers))
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(EngineError::HttpStatus { status: status.as_u16(), url: url.to_string() });
    }
    let h = resp.headers().clone();
    let final_url = resp.url().to_string();
    drop(resp); // on n'a pas besoin du corps

    let (total_size, accept_ranges) = if status == StatusCode::PARTIAL_CONTENT {
        match h.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(parse_content_range) {
            Some((0, 0, Some(total))) => (Some(total), true),
            Some((_, _, None)) => (None, false),
            _ => (None, false),
        }
    } else {
        let len = h
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        (len, false)
    };

    let mime = header_str(&h, header::CONTENT_TYPE);
    let mut name = h
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok().map(str::to_string).or_else(|| Some(String::from_utf8_lossy(v.as_bytes()).into_owned())))
        .and_then(|v| filename::from_content_disposition(&v))
        .or_else(|| url::Url::parse(&final_url).ok().and_then(|u| filename::from_url(&u)))
        .or_else(|| url::Url::parse(url).ok().and_then(|u| filename::from_url(&u)));

    // Ajoute l'extension déduite du type MIME si le nom n'en a pas.
    if let (Some(n), Some(m)) = (&mut name, &mime)
        && filename::split_ext(n).1.is_none()
            && let Some(ext) = filename::extension_for_mime(m) {
                n.push('.');
                n.push_str(ext);
            }

    Ok(ProbeResult {
        final_url,
        total_size,
        accept_ranges,
        filename: name,
        mime,
        validator: Validator {
            etag: header_str(&h, header::ETAG),
            last_modified: header_str(&h, header::LAST_MODIFIED),
        },
    })
}

// ---------------------------------------------------------------------------
// Exécution
// ---------------------------------------------------------------------------

/// Réglages figés au démarrage d'une exécution.
struct RunParams {
    url: String,
    headers: HeaderMap,
    validator: Validator,
    resumable: bool,
    min_segment: u64,
    read_timeout: Duration,
    max_retries: u32,
}

/// Lance (ou reprend) un téléchargement jusqu'à la fin, l'erreur ou l'annulation.
/// `name_taken` permet au gestionnaire d'éviter deux téléchargements vers le même fichier.
pub async fn run(
    shared: Arc<DownloadShared>,
    ctx: Arc<EngineCtx>,
    cancel: CancellationToken,
    name_taken: impl Fn(&Path) -> bool,
) -> Result<()> {
    if shared.record.lock().media.is_some() {
        return run_media(shared, ctx, cancel, name_taken).await;
    }
    let (url, headers) = {
        let r = shared.record.lock();
        (r.url.clone(), r.headers.clone())
    };

    // 1. Analyse, avec quelques tentatives.
    let max_retries = ctx.config.read().max_retries;
    let mut attempt = 0;
    let info = loop {
        let res = tokio::select! {
            _ = cancel.cancelled() => return Err(EngineError::Cancelled),
            r = probe(&ctx, &url, &headers) => r,
        };
        match res {
            Ok(i) => break i,
            Err(e) if e.is_retryable() && attempt < max_retries.min(4) => {
                attempt += 1;
                backoff(attempt, &cancel).await?;
            }
            Err(e) => return Err(e),
        }
    };

    // 2. Préparation de l'état et du fichier.
    let (part, params, size) = {
        let cfg = ctx.config.read().clone();
        let mut r = shared.record.lock();

        let can_resume = r.resumable
            && !r.segments.is_empty()
            && info.accept_ranges
            && r.total_size == info.total_size
            && !r.validator.conflicts_with(&info.validator)
            && r.final_path().map(|p| part_path(&p).exists()).unwrap_or(false);

        if !can_resume {
            if !r.segments.is_empty() {
                // Fichier distant changé ou serveur sans reprise : on repart de zéro.
                if let Some(p) = r.final_path() {
                    let _ = std::fs::remove_file(part_path(&p));
                }
            }
            r.reset_progress();

            if !r.filename_locked && r.segments.is_empty() {
                let name = filename::sanitize(info.filename.as_deref().unwrap_or("telechargement"));
                r.category = filename::categorize(&name);
                if !r.directory_locked {
                    r.directory = cfg.directory_for(r.category);
                }
                std::fs::create_dir_all(&r.directory)?;
                let unique = filename::unique_path(&r.directory, &name, &name_taken);
                r.filename = unique.file_name().unwrap().to_string_lossy().into_owned();
            } else {
                r.filename = filename::sanitize(&r.filename);
                r.category = filename::categorize(&r.filename);
                std::fs::create_dir_all(&r.directory)?;
            }

            r.total_size = info.total_size;
            r.resumable = info.accept_ranges && info.total_size.is_some();
            r.validator = info.validator.clone();
            r.segments = initial_segments(
                info.total_size,
                r.resumable,
                r.max_connections.unwrap_or(cfg.connections_per_download),
                cfg.min_segment_size,
            );
        }
        r.final_url = Some(info.final_url.clone());
        r.mime = info.mime.clone();
        r.status = DownloadStatus::Downloading;
        r.error = None;

        let final_path = r.final_path().expect("nom défini");
        let part = part_path(&final_path);
        let params = RunParams {
            url: info.final_url.clone(),
            headers: build_headers(&r.headers),
            validator: r.validator.clone(),
            resumable: r.resumable,
            min_segment: cfg.min_segment_size.max(64 * 1024),
            read_timeout: Duration::from_secs(cfg.read_timeout_secs.max(5)),
            max_retries: cfg.max_retries,
        };
        (part, params, r.total_size)
    };

    let file = open_part(&part, size)?;
    let file = Arc::new(file);

    // 3. Connexions parallèles.
    let n_workers = {
        let r = shared.record.lock();
        let wanted = r.max_connections.unwrap_or(ctx.config.read().connections_per_download).clamp(1, 32) as usize;
        if params.resumable { wanted } else { 1 }
    };
    let params = Arc::new(params);
    let last_error: Arc<Mutex<Option<EngineError>>> = Arc::new(Mutex::new(None));
    let mut set = JoinSet::new();
    let has_work = shared.record.lock().segments.iter().any(|s| !s.is_done());
    if has_work {
        for _ in 0..n_workers {
            set.spawn(worker(
                shared.clone(),
                ctx.clone(),
                file.clone(),
                params.clone(),
                cancel.clone(),
                last_error.clone(),
            ));
        }
    }
    while set.join_next().await.is_some() {}

    if cancel.is_cancelled() {
        file.sync_data().ok();
        return Err(EngineError::Cancelled);
    }

    // 4. Vérification et finalisation.
    {
        let mut r = shared.record.lock();
        // Taille inconnue : le flux s'est terminé, on fige la taille réelle.
        if r.total_size.is_none() && last_error.lock().is_none() {
            r.segments.retain(|s| !(s.end.is_none() && s.downloaded == 0));
            if r.segments.iter().all(|s| s.end.is_some()) {
                r.total_size = Some(r.downloaded());
            }
        }
        let complete = r.segments.iter().all(|s| s.is_done()) && r.total_size.is_some();
        if !complete {
            return Err(last_error.lock().take().unwrap_or(EngineError::Truncated));
        }
        r.status = DownloadStatus::Finalizing;
    }

    file.sync_all()?;
    drop(file);
    // Taille inconnue au départ : on tronque à la taille réelle.
    if size.is_none() {
        let len = shared.record.lock().total_size.unwrap_or(0);
        OpenOptions::new().write(true).open(&part)?.set_len(len)?;
    }

    // Contrôle d'intégrité : la taille sur disque doit correspondre exactement.
    let expected = shared.record.lock().total_size.unwrap_or(0);
    let on_disk = std::fs::metadata(&part)?.len();
    if on_disk != expected {
        return Err(EngineError::Integrity { expected, actual: on_disk });
    }

    let final_path = {
        let mut r = shared.record.lock();
        let mut p = r.final_path().unwrap();
        if p.exists() {
            // Un fichier homonyme est apparu entre-temps : on ne l'écrase pas.
            p = filename::unique_path(&r.directory, &r.filename.clone(), |_| false);
            r.filename = p.file_name().unwrap().to_string_lossy().into_owned();
        }
        p
    };
    std::fs::rename(&part, &final_path)?;
    Ok(())
}

/// Téléchargement d'une vidéo par yt-dlp : même file d'attente, même suivi, même reprise.
async fn run_media(
    shared: Arc<DownloadShared>,
    ctx: Arc<EngineCtx>,
    cancel: CancellationToken,
    name_taken: impl Fn(&Path) -> bool,
) -> Result<()> {
    use crate::media::{self, MediaProgress, Stage};

    let tools = ctx.media_tools.read().clone();
    if tools.ytdlp.is_none() {
        return Err(EngineError::Other(
            "yt-dlp n'est pas installé. Installez-le dans Paramètres > Outils vidéo, puis reprenez.".into(),
        ));
    }
    if tools.ffmpeg_dir.is_none() {
        return Err(EngineError::Other(
            "FFmpeg est nécessaire pour assembler les vidéos. Installez-le dans Paramètres > Outils vidéo, puis reprenez.".into(),
        ));
    }

    let (url, mut spec) = {
        let r = shared.record.lock();
        (r.url.clone(), r.media.clone().expect("vidéo"))
    };

    // Ajout par lots sans analyse préalable : on récupère le titre maintenant.
    if spec.title.trim().is_empty() {
        let info = media::analyze(&tools, &url, &cancel).await?;
        spec.title = info.title;
        if spec.est_size.is_none() {
            spec.est_size = info.options.iter().find(|o| o.format == spec.format).and_then(|o| o.est_size);
        }
        shared.record.lock().media = Some(spec.clone());
    }

    let stem = {
        let cfg = ctx.config.read().clone();
        let mut r = shared.record.lock();
        if r.filename.is_empty() {
            let mut title: String = spec.title.chars().take(150).collect();
            if title.trim().is_empty() {
                title = "video".into();
            }
            let name = filename::sanitize(&format!("{}.{}", title.trim(), spec.format.extension()));
            r.category = if spec.format.is_audio() { Category::Music } else { Category::Video };
            if !r.directory_locked {
                r.directory = cfg.directory_for(r.category);
            }
            std::fs::create_dir_all(&r.directory)?;
            let unique = filename::unique_path(&r.directory, &name, &name_taken);
            r.filename = unique.file_name().unwrap().to_string_lossy().into_owned();
        } else {
            std::fs::create_dir_all(&r.directory)?;
        }
        r.status = DownloadStatus::Downloading;
        r.error = None;
        r.resumable = true;
        r.total_size = spec.est_size;
        r.stage = None;
        r.segments = vec![Segment { start: 0, end: spec.est_size.map(|t| t.saturating_sub(1)), downloaded: 0, active: true }];
        r.media_stem().expect("nom défini")
    };

    let rate = [ctx.global_limiter.rate(), shared.limiter.rate()].into_iter().filter(|r| *r > 0).min().unwrap_or(0);
    shared.active_connections.store(1, Ordering::Relaxed);
    let sh = shared.clone();
    let mut last_done = 0u64;
    let result = media::download(&tools, &url, &spec, &stem, rate, &cancel, move |p| {
        let mut r = sh.record.lock();
        match p {
            MediaProgress::Bytes { done, total, stage } => {
                if done > last_done {
                    sh.received.fetch_add(done - last_done, Ordering::Relaxed);
                }
                last_done = done;
                r.total_size = total.or(r.total_size);
                let end = r.total_size.map(|t| t.max(done).saturating_sub(1));
                r.segments = vec![Segment { start: 0, end, downloaded: done, active: true }];
                r.stage = Some(stage);
                if r.status == DownloadStatus::Finalizing {
                    r.status = DownloadStatus::Downloading;
                }
            }
            MediaProgress::Stage(s) => {
                r.stage = Some(s);
                if matches!(s, Stage::Merge | Stage::ConvertAudio | Stage::Subtitles | Stage::Finishing) {
                    r.status = DownloadStatus::Finalizing;
                }
            }
            MediaProgress::Done(_) => r.stage = Some(Stage::Finishing),
        }
    })
    .await;
    shared.active_connections.store(0, Ordering::Relaxed);

    let path = result?;
    let len = std::fs::metadata(&path)?.len();
    let mut r = shared.record.lock();
    if let (Some(name), Some(dir)) = (path.file_name(), path.parent()) {
        r.filename = name.to_string_lossy().into_owned();
        r.directory = dir.to_path_buf();
    }
    r.total_size = Some(len);
    r.segments = vec![Segment { start: 0, end: len.checked_sub(1), downloaded: len, active: false }];
    r.stage = None;
    Ok(())
}

fn open_part(part: &Path, size: Option<u64>) -> Result<File> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(part)?;
    if let Some(sz) = size {
        // Pré-allocation : réduit la fragmentation et détecte tôt un disque plein.
        if file.metadata()?.len() != sz {
            file.set_len(sz)?;
        }
    }
    Ok(file)
}

fn initial_segments(total: Option<u64>, resumable: bool, connections: u8, min_seg: u64) -> Vec<Segment> {
    match total {
        Some(0) => vec![],
        Some(t) if resumable => {
            let n = (t / min_seg.max(1)).clamp(1, connections.max(1) as u64);
            let base = t / n;
            (0..n)
                .map(|i| {
                    let start = i * base;
                    let end = if i == n - 1 { t - 1 } else { start + base - 1 };
                    Segment { start, end: Some(end), downloaded: 0, active: false }
                })
                .collect()
        }
        Some(t) => vec![Segment { start: 0, end: Some(t - 1), downloaded: 0, active: false }],
        None => vec![Segment { start: 0, end: None, downloaded: 0, active: false }],
    }
}

/// Attribue un segment à une connexion : d'abord un segment orphelin, sinon
/// on scinde le segment actif qui a le plus de travail restant.
fn claim_segment(shared: &DownloadShared, params: &RunParams) -> Option<usize> {
    let mut r = shared.record.lock();
    if let Some(i) = r.segments.iter().position(|s| !s.active && !s.is_done()) {
        r.segments[i].active = true;
        return Some(i);
    }
    if !params.resumable {
        return None;
    }
    let (idx, remaining) = r
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.active)
        .filter_map(|(i, s)| s.remaining().map(|rem| (i, rem)))
        .max_by_key(|(_, rem)| *rem)?;
    if remaining < params.min_segment * 2 {
        return None;
    }
    let victim = &mut r.segments[idx];
    let old_end = victim.end?;
    let split_at = victim.cursor() + remaining / 2;
    victim.end = Some(split_at - 1);
    r.segments.push(Segment { start: split_at, end: Some(old_end), downloaded: 0, active: true });
    Some(r.segments.len() - 1)
}

async fn backoff(attempt: u32, cancel: &CancellationToken) -> Result<()> {
    let secs = (1u64 << attempt.min(5)).min(30);
    tokio::select! {
        _ = cancel.cancelled() => Err(EngineError::Cancelled),
        _ = tokio::time::sleep(Duration::from_secs(secs)) => Ok(()),
    }
}

async fn worker(
    shared: Arc<DownloadShared>,
    ctx: Arc<EngineCtx>,
    file: Arc<File>,
    params: Arc<RunParams>,
    cancel: CancellationToken,
    last_error: Arc<Mutex<Option<EngineError>>>,
) {
    let mut attempts = 0u32;
    while !cancel.is_cancelled() {
        let Some(idx) = claim_segment(&shared, &params) else { break };
        shared.active_connections.fetch_add(1, Ordering::Relaxed);
        let before = shared.record.lock().segments[idx].downloaded;
        let res = fetch_segment(&shared, &ctx, &file, &params, idx, &cancel).await;
        shared.active_connections.fetch_sub(1, Ordering::Relaxed);
        let progressed = shared.record.lock().segments[idx].downloaded > before;
        shared.record.lock().segments[idx].active = false;

        match res {
            // Flux de taille inconnue terminé (éventuellement vide) : plus rien à faire.
            Ok(()) if !params.resumable => break,
            Ok(()) => attempts = 0,
            Err(EngineError::Cancelled) => break,
            Err(e) => {
                if progressed {
                    attempts = 0;
                }
                let others = shared.active_connections.load(Ordering::Relaxed);
                // Serveur qui limite le nombre de connexions : on se retire
                // silencieusement, les autres connexions reprendront ce segment.
                if matches!(e, EngineError::HttpStatus { status: 429 | 503, .. }) && others > 0 {
                    break;
                }
                if e.is_retryable() && attempts < params.max_retries {
                    attempts += 1;
                    tracing::debug!("segment {idx} : nouvelle tentative {attempts} après {e}");
                    if backoff(attempts, &cancel).await.is_err() {
                        break;
                    }
                    continue;
                }
                *last_error.lock() = Some(e);
                break;
            }
        }
    }
}

/// Télécharge un segment jusqu'à sa borne (qui peut reculer si un autre worker le scinde).
async fn fetch_segment(
    shared: &DownloadShared,
    ctx: &EngineCtx,
    file: &Arc<File>,
    params: &RunParams,
    idx: usize,
    cancel: &CancellationToken,
) -> Result<()> {
    let (start, end) = {
        let mut r = shared.record.lock();
        let seg = &mut r.segments[idx];
        if !params.resumable && seg.downloaded > 0 {
            // Pas de reprise possible : on recommence le flux depuis le début.
            seg.downloaded = 0;
        }
        (seg.cursor(), seg.end)
    };
    if let Some(e) = end
        && start > e {
            return Ok(());
        }

    let mut req = ctx.client.get(&params.url).headers(params.headers.clone());
    if params.resumable {
        let range = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };
        req = req.header(header::RANGE, range);
        if let Some(v) = params.validator.if_range() {
            req = req.header(header::IF_RANGE, v);
        }
    }

    let resp = tokio::select! {
        _ = cancel.cancelled() => return Err(EngineError::Cancelled),
        r = tokio::time::timeout(params.read_timeout, req.send()) => match r {
            Ok(r) => r?,
            Err(_) => return Err(EngineError::Stalled),
        },
    };

    let status = resp.status();
    if !status.is_success() {
        return Err(EngineError::HttpStatus { status: status.as_u16(), url: params.url.clone() });
    }
    if params.resumable {
        if status != StatusCode::PARTIAL_CONTENT {
            // 200 en réponse à If-Range = le fichier a changé.
            return Err(if params.validator.if_range().is_some() { EngineError::RemoteChanged } else { EngineError::BadRange });
        }
        let cr = resp
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range);
        if !matches!(cr, Some((a, _, _)) if a == start) {
            return Err(EngineError::BadRange);
        }
    }

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::with_capacity(WRITE_BUFFER);
    let mut buf_offset = start;

    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                flush(shared, file, idx, &mut buf, &mut buf_offset).await?;
                return Err(EngineError::Cancelled);
            }
            n = tokio::time::timeout(params.read_timeout, stream.next()) => n,
        };
        let chunk = match next {
            Err(_) => {
                flush(shared, file, idx, &mut buf, &mut buf_offset).await?;
                return Err(EngineError::Stalled);
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                flush(shared, file, idx, &mut buf, &mut buf_offset).await?;
                return Err(e.into());
            }
            Ok(Some(Ok(c))) => c,
        };

        let n = chunk.len() as u64;
        shared.received.fetch_add(n, Ordering::Relaxed);
        limiter::acquire_all(&[&ctx.global_limiter, &shared.limiter], n).await;
        buf.extend_from_slice(&chunk);

        if buf.len() >= WRITE_BUFFER {
            let finished = flush(shared, file, idx, &mut buf, &mut buf_offset).await?;
            if finished {
                return Ok(());
            }
        }
    }

    flush(shared, file, idx, &mut buf, &mut buf_offset).await?;
    let mut r = shared.record.lock();
    let seg = &mut r.segments[idx];
    match seg.end {
        Some(_) if !seg.is_done() => Err(EngineError::Truncated),
        Some(_) => Ok(()),
        None => {
            // Flux de taille inconnue terminé : on fige la borne.
            // (Un corps vide laisse le segment ouvert ; il est retiré à la finalisation.)
            if seg.downloaded > 0 {
                seg.end = Some(seg.start + seg.downloaded - 1);
            }
            Ok(())
        }
    }
}

/// Écrit le tampon à sa position, sans dépasser la borne actuelle du segment.
/// Renvoie `true` si le segment est terminé.
async fn flush(
    shared: &DownloadShared,
    file: &Arc<File>,
    idx: usize,
    buf: &mut Vec<u8>,
    offset: &mut u64,
) -> Result<bool> {
    if buf.is_empty() {
        let r = shared.record.lock();
        return Ok(r.segments.get(idx).map(|s| s.is_done()).unwrap_or(true));
    }
    let allowed = {
        let r = shared.record.lock();
        match r.segments[idx].end {
            Some(e) => (e + 1).saturating_sub(*offset).min(buf.len() as u64) as usize,
            None => buf.len(),
        }
    };
    if allowed > 0 {
        let data = buf[..allowed].to_vec();
        let f = file.clone();
        let off = *offset;
        tokio::task::spawn_blocking(move || crate::fileio::write_all_at(&f, &data, off))
            .await
            .map_err(|e| EngineError::Other(e.to_string()))??;
    }
    let done = {
        let mut r = shared.record.lock();
        let seg = &mut r.segments[idx];
        seg.downloaded += allowed as u64;
        if let Some(l) = seg.len() {
            seg.downloaded = seg.downloaded.min(l);
        }
        seg.is_done()
    };
    *offset += allowed as u64;
    buf.clear();
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range("bytes 0-0/1234"), Some((0, 0, Some(1234))));
        assert_eq!(parse_content_range("bytes 10-20/*"), Some((10, 20, None)));
        assert_eq!(parse_content_range("garbage"), None);
    }

    #[test]
    fn segments_cover_file_exactly() {
        for total in [1u64, 5, 1_000_000, 10_485_761, 123_456_789] {
            let segs = initial_segments(Some(total), true, 8, 1024 * 1024);
            assert_eq!(segs.first().unwrap().start, 0);
            assert_eq!(segs.last().unwrap().end, Some(total - 1));
            for w in segs.windows(2) {
                assert_eq!(w[0].end.unwrap() + 1, w[1].start);
            }
            assert!(segs.len() <= 8);
        }
    }
}

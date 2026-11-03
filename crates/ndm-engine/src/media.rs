//! Téléchargement vidéo via yt-dlp (analyse des formats, sélection, progression)
//! et FFmpeg (fusion audio/vidéo, extraction audio).
//!
//! yt-dlp est lancé comme processus externe, sans console. NDM ne contourne aucun
//! DRM : les formats chiffrés sont écartés et signalés.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Outils
// ---------------------------------------------------------------------------

/// Emplacement des outils externes (dossier d'outils de l'application ou PATH).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaTools {
    pub ytdlp: Option<PathBuf>,
    /// Dossier contenant `ffmpeg` et `ffprobe`.
    pub ffmpeg_dir: Option<PathBuf>,
}

/// Prépare une commande sans fenêtre console (Windows) et dans son propre groupe
/// de processus (Unix), pour pouvoir arrêter yt-dlp et ses FFmpeg d'un coup.
pub fn command(program: &Path) -> Command {
    let mut c = Command::new(program);
    c.stdin(Stdio::null()).kill_on_drop(true);
    // Sorties en UTF-8 quel que soit le code de page de la console Windows.
    c.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    c.process_group(0);
    c
}

/// Arrête un processus et tous ses enfants.
async fn kill_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let mut k = Command::new("taskkill");
            k.args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null());
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            k.creation_flags(CREATE_NO_WINDOW);
            let _ = k.status().await;
        }
        #[cfg(unix)]
        {
            // « -- » obligatoire : sinon « -PID » est lu comme une option de kill.
            let _ = Command::new("kill").args(["-TERM", "--", &format!("-{pid}")]).status().await;
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let _ = child.kill().await;
}

// ---------------------------------------------------------------------------
// Analyse
// ---------------------------------------------------------------------------

/// Ordre de préférence des formats, identique à l'analyse et au téléchargement :
/// résolution d'abord, puis conteneur MP4/M4A ; yt-dlp départage ensuite (codec, débit…).
/// Avec ce même tri, la liste renvoyée par `yt-dlp -J` va du moins bon au meilleur format,
/// ce qui permet d'estimer la taille du flux réellement choisi.
const FORMAT_SORT: &str = "res,ext:mp4:m4a";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaFormat {
    /// Meilleure qualité disponible, en MP4.
    Best,
    /// Vidéo MP4 limitée à cette hauteur (ex. 1080).
    Video { max_height: u32 },
    AudioMp3,
    AudioM4a,
}

impl MediaFormat {
    pub fn is_audio(self) -> bool {
        matches!(self, Self::AudioMp3 | Self::AudioM4a)
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Best | Self::Video { .. } => "mp4",
            Self::AudioMp3 => "mp3",
            Self::AudioM4a => "m4a",
        }
    }
    /// Arguments de sélection passés à yt-dlp.
    pub fn ytdlp_args(self, subtitles: bool) -> Vec<String> {
        let mut a: Vec<String> = match self {
            // Résolution d'abord, puis préférence pour MP4/M4A (compatibilité maximale).
            Self::Best => vec!["-f".into(), "bv*+ba/b".into(), "-S".into(), FORMAT_SORT.into()],
            Self::Video { max_height: h } => vec![
                "-f".into(),
                format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"),
                "-S".into(),
                FORMAT_SORT.into(),
            ],
            Self::AudioMp3 => vec![
                "-f".into(),
                "ba/b".into(),
                "-x".into(),
                "--audio-format".into(),
                "mp3".into(),
                "--audio-quality".into(),
                "0".into(),
            ],
            Self::AudioM4a => {
                vec!["-f".into(), "ba[ext=m4a]/ba/b".into(), "-x".into(), "--audio-format".into(), "m4a".into()]
            }
        };
        if !self.is_audio() {
            a.extend(["--merge-output-format".into(), "mp4".into()]);
            if subtitles {
                a.extend([
                    "--write-subs".into(),
                    "--sub-langs".into(),
                    "fr.*,en.*".into(),
                    "--embed-subs".into(),
                ]);
            }
        }
        a
    }
}

/// Une option proposée à l'utilisateur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaOption {
    pub id: String,
    /// « 1080p », « Audio uniquement »…
    pub label: String,
    /// « MP4 », « MP3 »…
    pub container: String,
    pub format: MediaFormat,
    /// Taille estimée en octets, si calculable.
    pub est_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaInfo {
    pub url: String,
    pub title: String,
    pub duration_secs: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub site: Option<String>,
    pub options: Vec<MediaOption>,
    /// Langues de sous-titres disponibles (hors sous-titres automatiques).
    pub subtitle_langs: Vec<String>,
    /// Certaines qualités étaient protégées par DRM et ont été écartées.
    pub drm_filtered: bool,
}

fn f64_of(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Taille d'un format : exacte, approchée, ou déduite du débit et de la durée.
fn format_size(f: &Value, duration: Option<f64>) -> Option<u64> {
    if let Some(s) = f64_of(f, "filesize").or_else(|| f64_of(f, "filesize_approx")) {
        return Some(s as u64);
    }
    match (f64_of(f, "tbr"), duration) {
        (Some(kbps), Some(d)) if kbps > 0.0 && d > 0.0 => Some((kbps * 1000.0 / 8.0 * d) as u64),
        _ => None,
    }
}

/// Construit les options à partir de la sortie JSON de `yt-dlp -J`.
pub fn build_info(url: &str, j: &Value) -> Result<MediaInfo> {
    if j.get("_type").and_then(Value::as_str) == Some("playlist") {
        let n = j.get("entries").and_then(Value::as_array).map(|e| e.len()).unwrap_or(0);
        return Err(EngineError::Other(format!(
            "Cette adresse est une liste de lecture ({n} vidéos). Ouvrez une vidéo précise : les listes arriveront dans une prochaine version."
        )));
    }
    if j.get("is_live").and_then(Value::as_bool) == Some(true) {
        return Err(EngineError::Other("Les vidéos en direct ne sont pas encore prises en charge.".into()));
    }

    let duration = f64_of(j, "duration");
    let all: Vec<Value> = match j.get("formats").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a.clone(),
        _ => vec![j.clone()], // vidéo sans liste de formats (fichier direct)
    };
    let drm = |f: &Value| f.get("has_drm").and_then(Value::as_bool).unwrap_or(false);
    let drm_filtered = all.iter().any(drm);
    let formats: Vec<&Value> = all
        .iter()
        .filter(|f| !drm(f))
        // Les « storyboards » (vignettes) ne sont pas des vidéos.
        .filter(|f| str_of(f, "format_note") != Some("storyboard") && str_of(f, "ext") != Some("mhtml"))
        .collect();
    if formats.is_empty() {
        return Err(EngineError::Other(
            "Cette vidéo est protégée par DRM : son téléchargement n'est pas possible.".into(),
        ));
    }

    let vcodec = |f: &Value| str_of(f, "vcodec").map(str::to_string);
    let acodec = |f: &Value| str_of(f, "acodec").map(str::to_string);
    let is_audio_only = |f: &Value| vcodec(f).as_deref() == Some("none");
    let has_audio = |f: &Value| matches!(acodec(f).as_deref(), Some(a) if a != "none") || is_audio_only(f);

    // `formats` est trié du moins bon au meilleur (voir FORMAT_SORT) : le dernier
    // format qui satisfait un filtre est celui que yt-dlp sélectionnera.
    let best_audio = formats.iter().rev().find(|f| is_audio_only(f)).copied();
    let best_audio_size = best_audio.and_then(|f| format_size(f, duration));
    let m4a_audio_size = formats
        .iter()
        .rev()
        .find(|f| is_audio_only(f) && str_of(f, "ext") == Some("m4a"))
        .or(best_audio.as_ref())
        .and_then(|f| format_size(f, duration));
    let any_audio = formats.iter().any(|f| has_audio(f)) || vcodec(formats[0]).is_none();

    // Hauteurs disponibles, de la plus grande à la plus petite.
    let mut heights: Vec<u32> = formats
        .iter()
        .filter(|f| !is_audio_only(f))
        .filter_map(|f| f.get("height").and_then(Value::as_u64))
        .map(|h| h as u32)
        .filter(|h| *h >= 144)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();

    let mut options = Vec::new();
    for h in &heights {
        // Même règle que « bv*[height<=h] » : meilleur flux vidéo dont la hauteur ne dépasse pas h.
        let chosen = formats.iter().rev().find(|f| {
            !is_audio_only(f) && f.get("height").and_then(Value::as_u64).is_some_and(|fh| fh <= *h as u64)
        });
        let est = chosen.and_then(|v| {
            let vs = format_size(v, duration)?;
            let separate_audio = !matches!(acodec(v).as_deref(), Some(a) if a != "none");
            Some(vs + if separate_audio { best_audio_size.unwrap_or(0) } else { 0 })
        });
        let label = match h {
            2160 => "2160p (4K)".to_string(),
            1440 => "1440p (2K)".to_string(),
            _ => format!("{h}p"),
        };
        options.push(MediaOption {
            id: format!("v{h}"),
            label,
            container: "MP4".into(),
            format: MediaFormat::Video { max_height: *h },
            est_size: est,
        });
    }
    if heights.is_empty() && formats.iter().any(|f| !is_audio_only(f)) {
        let est = formats.iter().filter(|f| !is_audio_only(f)).filter_map(|f| format_size(f, duration)).max();
        options.push(MediaOption {
            id: "best".into(),
            label: "Meilleure qualité".into(),
            container: "MP4".into(),
            format: MediaFormat::Best,
            est_size: est,
        });
    }
    if any_audio {
        // MP3 au débit maximal (~320 kb/s) : taille déduite de la durée.
        let mp3_est = duration.map(|d| (d * 320_000.0 / 8.0) as u64);
        options.push(MediaOption {
            id: "mp3".into(),
            label: "Audio uniquement".into(),
            container: "MP3".into(),
            format: MediaFormat::AudioMp3,
            est_size: mp3_est,
        });
        options.push(MediaOption {
            id: "m4a".into(),
            label: "Audio uniquement".into(),
            container: "M4A".into(),
            format: MediaFormat::AudioM4a,
            est_size: m4a_audio_size,
        });
    }
    if options.is_empty() {
        return Err(EngineError::Other("Aucun format téléchargeable n'a été trouvé.".into()));
    }

    let mut subtitle_langs: Vec<String> = j
        .get("subtitles")
        .and_then(Value::as_object)
        .map(|m| m.keys().filter(|k| *k != "live_chat").cloned().collect())
        .unwrap_or_default();
    subtitle_langs.sort();

    Ok(MediaInfo {
        url: url.to_string(),
        title: str_of(j, "title").unwrap_or("Vidéo").to_string(),
        duration_secs: duration,
        thumbnail: str_of(j, "thumbnail").map(str::to_string),
        uploader: str_of(j, "uploader").or_else(|| str_of(j, "channel")).map(str::to_string),
        site: site_name(url, str_of(j, "extractor_key").or_else(|| str_of(j, "extractor"))),
        options,
        subtitle_langs,
        drm_filtered,
    })
}

/// Nom de site lisible : extracteurs génériques remplacés par le nom de domaine.
fn site_name(url: &str, extractor: Option<&str>) -> Option<String> {
    let host = || {
        url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
    };
    match extractor {
        None | Some("Generic") | Some("generic") | Some("HTML5MediaEmbed") => host(),
        Some("Youtube") | Some("YoutubeTab") => Some("YouTube".into()),
        Some("TikTok") => Some("TikTok".into()),
        Some("Twitter") => Some("X (Twitter)".into()),
        Some(e) => Some(e.to_string()),
    }
}

/// Traduit les erreurs de yt-dlp en messages compréhensibles.
pub fn friendly_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim();
    let l = line.to_ascii_lowercase();
    let msg = if l.contains("unsupported url") {
        "Aucune vidéo reconnue sur cette page."
    } else if l.contains("drm") {
        "Cette vidéo est protégée par DRM : son téléchargement n'est pas possible."
    } else if l.contains("not a bot") {
        "Le site demande une vérification anti-robot. Réessayez plus tard ou mettez à jour yt-dlp dans les paramètres."
    } else if l.contains("confirm your age") || l.contains("age-restricted") || l.contains("inappropriate") {
        "Vidéo soumise à une limite d'âge : elle nécessite d'être connecté sur le site."
    } else if l.contains("private video") || l.contains("members-only") || l.contains("join this channel") {
        "Vidéo privée ou réservée aux abonnés."
    } else if l.contains("video unavailable") || l.contains("has been removed") || l.contains("not available") && !l.contains("format") {
        "Vidéo indisponible : supprimée, privée ou bloquée dans votre pays."
    } else if l.contains("requested format is not available") {
        "La qualité choisie n'est plus disponible. Relancez l'analyse."
    } else if l.contains("ffmpeg") || l.contains("ffprobe") {
        "FFmpeg est nécessaire pour cette opération. Installez-le dans Paramètres > Outils vidéo."
    } else if l.contains("http error 403") || l.contains("forbidden") {
        "Accès refusé par le site (403). Mettre à jour yt-dlp corrige souvent ce problème."
    } else if l.contains("http error 404") {
        "Vidéo introuvable (404)."
    } else if l.contains("http error 429") || l.contains("too many requests") {
        "Le site limite les requêtes (429). Réessayez dans quelques minutes."
    } else if l.contains("unable to download") || l.contains("timed out") || l.contains("connection") || l.contains("getaddrinfo") {
        "Connexion au site impossible. Vérifiez votre connexion Internet."
    } else if l.contains("no space left") || l.contains("errno 28") {
        "Espace disque insuffisant dans le dossier de destination."
    } else if l.contains("permission denied") || l.contains("errno 13") {
        "Permission refusée : impossible d'écrire dans le dossier de destination."
    } else {
        let raw = line.trim_start_matches("ERROR:").trim();
        let short: String = raw.chars().take(220).collect();
        return if short.is_empty() { "yt-dlp a échoué sans message.".into() } else { format!("yt-dlp : {short}") };
    };
    msg.to_string()
}

/// Analyse une page avec `yt-dlp -J`.
pub async fn analyze(tools: &MediaTools, url: &str, cancel: &CancellationToken) -> Result<MediaInfo> {
    let ytdlp = tools.ytdlp.as_ref().ok_or_else(|| EngineError::Other("yt-dlp n'est pas installé.".into()))?;
    let mut c = command(ytdlp);
    c.args(["--ignore-config", "-J", "--no-playlist", "--no-warnings", "--encoding", "utf-8", "-S", FORMAT_SORT]);
    if let Some(d) = &tools.ffmpeg_dir {
        c.arg("--ffmpeg-location").arg(d);
    }
    c.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = c.spawn().map_err(|e| EngineError::Other(format!("Impossible de lancer yt-dlp : {e}")))?;
    let out = tokio::select! {
        _ = cancel.cancelled() => return Err(EngineError::Cancelled),
        r = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output()) => match r {
            Ok(o) => o?,
            Err(_) => return Err(EngineError::Other("L'analyse de la page a pris trop de temps.".into())),
        },
    };
    if !out.status.success() {
        return Err(EngineError::Other(friendly_error(&String::from_utf8_lossy(&out.stderr))));
    }
    let j: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| EngineError::Other(format!("Réponse de yt-dlp illisible : {e}")))?;
    build_info(url, &j)
}

// ---------------------------------------------------------------------------
// Téléchargement
// ---------------------------------------------------------------------------

/// Ce que l'utilisateur a choisi, enregistré avec le téléchargement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaSpec {
    pub format: MediaFormat,
    #[serde(default)]
    pub subtitles: bool,
    /// Libellé lisible (« 1080p MP4 »).
    pub label: String,
    pub title: String,
    pub est_size: Option<u64>,
}

/// Événements remontés pendant le téléchargement.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaProgress {
    /// Octets terminés au total (tous flux confondus) et taille totale estimée.
    Bytes { done: u64, total: Option<u64>, stage: Stage },
    Stage(Stage),
    Done(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Download,
    Video,
    Audio,
    Merge,
    ConvertAudio,
    Subtitles,
    Finishing,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Download => "Téléchargement",
            Self::Video => "Piste vidéo",
            Self::Audio => "Piste audio",
            Self::Merge => "Fusion audio et vidéo",
            Self::ConvertAudio => "Conversion audio",
            Self::Subtitles => "Intégration des sous-titres",
            Self::Finishing => "Finalisation",
        }
    }
}

/// Suit la progression sur plusieurs flux successifs (vidéo puis audio).
#[derive(Debug, Default)]
pub struct ProgressTracker {
    finished_bytes: u64,
    cur_done: u64,
    cur_total: Option<u64>,
    streams_finished: u32,
    est_total: Option<u64>,
    expect_two: bool,
}

impl ProgressTracker {
    pub fn new(est_total: Option<u64>, format: MediaFormat) -> Self {
        Self { est_total, expect_two: !format.is_audio(), ..Default::default() }
    }

    fn stage(&self) -> Stage {
        match (self.expect_two, self.streams_finished) {
            (false, _) => Stage::Download,
            (true, 0) => Stage::Video,
            _ => Stage::Audio,
        }
    }

    /// Ligne `[ndm] statut|téléchargés|total|total_estimé`.
    pub fn on_line(&mut self, line: &str) -> Option<MediaProgress> {
        let num = |s: &str| s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64);
        if let Some(rest) = line.strip_prefix("[ndm] ") {
            let p: Vec<&str> = rest.split('|').collect();
            if p.len() < 4 {
                return None;
            }
            let downloaded = num(p[1]).unwrap_or(0);
            let total = num(p[2]).or_else(|| num(p[3]));
            let stage = self.stage();
            if p[0] == "finished" {
                self.finished_bytes += total.unwrap_or(downloaded).max(downloaded);
                self.cur_done = 0;
                self.cur_total = None;
                self.streams_finished += 1;
            } else {
                self.cur_done = downloaded;
                self.cur_total = total;
            }
            let done = self.finished_bytes + self.cur_done;
            let seen = self.finished_bytes + self.cur_total.unwrap_or(self.cur_done);
            // L'estimation issue de l'analyse est plus stable que l'extrapolation de yt-dlp
            // (très imprécise au début d'un flux HLS) : on la garde si elle existe.
            let total = match self.est_total {
                Some(e) => Some(e.max(done)),
                None if self.cur_total.is_some() || self.streams_finished > 0 => Some(seen),
                None => None,
            };
            return Some(MediaProgress::Bytes { done, total, stage });
        }
        if let Some(rest) = line.strip_prefix("[ndm-pp] ") {
            let pp = rest.split('|').next().unwrap_or("");
            let stage = match pp {
                "Merger" | "FFmpegMerger" => Stage::Merge,
                "ExtractAudio" | "FFmpegExtractAudio" => Stage::ConvertAudio,
                "EmbedSubtitle" | "FFmpegEmbedSubtitle" => Stage::Subtitles,
                _ => Stage::Finishing,
            };
            return Some(MediaProgress::Stage(stage));
        }
        if let Some(path) = line.strip_prefix("[ndm-done] ") {
            return Some(MediaProgress::Done(PathBuf::from(path.trim())));
        }
        None
    }
}

/// Lance yt-dlp et remonte la progression. Renvoie le chemin final du fichier.
pub async fn download(
    tools: &MediaTools,
    url: &str,
    spec: &MediaSpec,
    output_stem: &Path,
    rate_limit: u64,
    cancel: &CancellationToken,
    mut on_progress: impl FnMut(MediaProgress),
) -> Result<PathBuf> {
    let ytdlp = tools.ytdlp.as_ref().ok_or_else(|| EngineError::Other("yt-dlp n'est pas installé.".into()))?;
    let mut c = command(ytdlp);
    c.args([
        "--ignore-config",
        "--no-playlist",
        "--newline",
        "--quiet",
        "--progress",
        "--no-warnings",
        "--color",
        "no_color",
        "--encoding",
        "utf-8",
        "--no-mtime",
        "--windows-filenames",
        "--concurrent-fragments",
        "4",
        "--progress-template",
        "download:[ndm] %(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s",
        "--progress-template",
        "postprocess:[ndm-pp] %(progress.postprocessor)s|%(progress.status)s",
        "--print",
        "after_move:[ndm-done] %(filepath)s",
    ]);
    if let Some(d) = &tools.ffmpeg_dir {
        c.arg("--ffmpeg-location").arg(d);
    }
    if rate_limit > 0 {
        c.args(["--limit-rate", &rate_limit.to_string()]);
    }
    c.args(spec.format.ytdlp_args(spec.subtitles));
    // `%` est un caractère spécial des modèles yt-dlp : on l'échappe dans le nom.
    let template = format!("{}.%(ext)s", output_stem.to_string_lossy().replace('%', "%%"));
    c.arg("-o").arg(template);
    c.arg("--").arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = c.spawn().map_err(|e| EngineError::Other(format!("Impossible de lancer yt-dlp : {e}")))?;
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    // stderr lu en parallèle (jamais bloquant), dernières lignes conservées pour le message d'erreur.
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        while let Ok(Some(l)) = lines.next_line().await {
            tail.push(l);
            if tail.len() > 40 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });

    let mut tracker = ProgressTracker::new(spec.est_size, spec.format);
    let mut final_path: Option<PathBuf> = None;
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => {
                kill_tree(&mut child).await;
                return Err(EngineError::Cancelled);
            }
            l = lines.next_line() => l,
        };
        match next {
            Ok(Some(line)) => {
                if let Some(p) = tracker.on_line(line.trim_end()) {
                    if let MediaProgress::Done(path) = &p {
                        final_path = Some(path.clone());
                    }
                    on_progress(p);
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    let status = tokio::select! {
        _ = cancel.cancelled() => {
            kill_tree(&mut child).await;
            return Err(EngineError::Cancelled);
        }
        s = child.wait() => s?,
    };
    let stderr_text = err_task.await.unwrap_or_default();
    if !status.success() {
        return Err(EngineError::Other(friendly_error(&stderr_text)));
    }
    let path = final_path
        .or_else(|| {
            let p = PathBuf::from(format!("{}.{}", output_stem.to_string_lossy(), spec.format.extension()));
            p.exists().then_some(p)
        })
        .ok_or_else(|| EngineError::Other("yt-dlp n'a pas indiqué de fichier final.".into()))?;
    if !path.exists() {
        return Err(EngineError::Other(format!("Fichier final introuvable : {}", path.display())));
    }
    Ok(path)
}

/// Supprime les fichiers temporaires laissés par yt-dlp pour ce nom de base.
pub fn cleanup_partials(output_stem: &Path) {
    let (Some(dir), Some(stem)) = (output_stem.parent(), output_stem.file_name().map(|s| s.to_string_lossy().into_owned()))
    else {
        return;
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&stem) else { continue };
        // Fichiers intermédiaires : .f137.mp4, .part, .ytdl, .temp.mp4, sous-titres.
        let temp = rest.ends_with(".part")
            || rest.ends_with(".ytdl")
            || rest.contains(".part-Frag")
            || rest.starts_with(".f") && rest[2..].chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false)
            || rest.starts_with(".temp.")
            || rest.ends_with(".vtt")
            || rest.ends_with(".srt");
        if temp {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn yt_like() -> Value {
        json!({
            "title": "Conférence — Les arbres",
            "duration": 600.0,
            "extractor_key": "Youtube",
            "uploader": "Chaîne Test",
            "thumbnail": "https://i.ytimg.com/vi/x/maxresdefault.jpg",
            "subtitles": {"fr": [], "en": [], "live_chat": []},
            // Ordre renvoyé par `yt-dlp -J -S res,ext:mp4:m4a` : du moins bon au meilleur.
            "formats": [
                {"format_id": "sb0", "format_note": "storyboard", "ext": "mhtml", "vcodec": "none", "acodec": "none"},
                {"format_id": "251", "ext": "webm", "vcodec": "none", "acodec": "opus", "filesize": 10_000_000.0},
                {"format_id": "140", "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2", "filesize": 9_700_000.0},
                {"format_id": "18", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a", "height": 360, "filesize": 30_000_000.0},
                {"format_id": "136", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 720, "tbr": 1300.0},
                {"format_id": "248", "ext": "webm", "vcodec": "vp9", "acodec": "none", "height": 1080, "filesize_approx": 150_000_000.0, "tbr": 2000.0},
                {"format_id": "137", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 1080, "filesize": 200_000_000.0, "tbr": 2600.0},
                {"format_id": "399", "ext": "mp4", "vcodec": "av01", "acodec": "none", "height": 1080, "filesize": 82_000_000.0, "tbr": 1100.0},
                {"format_id": "drm1", "ext": "mp4", "vcodec": "avc1", "acodec": "none", "height": 2160, "has_drm": true}
            ]
        })
    }

    #[test]
    fn builds_youtube_like_options() {
        let info = build_info("https://x", &yt_like()).unwrap();
        let labels: Vec<_> = info.options.iter().map(|o| format!("{} {}", o.label, o.container)).collect();
        assert_eq!(labels, ["1080p MP4", "720p MP4", "360p MP4", "Audio uniquement MP3", "Audio uniquement M4A"]);
        // 1080p : le flux que yt-dlp choisira (399, AV1 en MP4, 82 Mo), pas le plus lourd (137, 200 Mo),
        // plus la meilleure piste audio selon le même tri (140, M4A, 9,7 Mo).
        assert_eq!(info.options[0].est_size, Some(82_000_000 + 9_700_000));
        // 720p : estimé depuis le débit (1300 kb/s × 600 s) + audio.
        assert_eq!(info.options[1].est_size, Some(97_500_000 + 9_700_000));
        // 360p : flux combiné, pas d'audio ajouté.
        assert_eq!(info.options[2].est_size, Some(30_000_000));
        // M4A : la piste M4A, pas l'Opus.
        assert_eq!(info.options[4].est_size, Some(9_700_000));
        assert!(info.drm_filtered, "la 4K chiffrée est écartée");
        assert_eq!(info.subtitle_langs, ["en", "fr"]);
        assert_eq!(info.site.as_deref(), Some("YouTube"));
        assert_eq!(site_name("http://www.exemple.fr/v", Some("Generic")).as_deref(), Some("exemple.fr"));
    }

    #[test]
    fn drm_only_is_refused() {
        let j = json!({"title": "x", "formats": [{"format_id": "a", "vcodec": "avc1", "height": 720, "has_drm": true}]});
        let e = build_info("u", &j).unwrap_err().to_string();
        assert!(e.contains("DRM"));
    }

    #[test]
    fn playlist_and_live_are_explained() {
        assert!(build_info("u", &json!({"_type": "playlist", "entries": [{}, {}]})).unwrap_err().to_string().contains("2 vidéos"));
        assert!(build_info("u", &json!({"is_live": true, "formats": [{}]})).unwrap_err().to_string().contains("direct"));
    }

    #[test]
    fn single_unknown_format_gives_best() {
        let j = json!({"title": "Démo", "formats": [{"format_id": "0", "ext": "mp4"}]});
        let info = build_info("u", &j).unwrap();
        assert_eq!(info.options[0].format, MediaFormat::Best);
        assert!(info.options.iter().any(|o| o.format == MediaFormat::AudioMp3));
    }

    #[test]
    fn tracks_two_streams_then_merge() {
        let mut t = ProgressTracker::new(Some(110), MediaFormat::Video { max_height: 1080 });
        assert_eq!(
            t.on_line("[ndm] downloading|50|100|NA"),
            Some(MediaProgress::Bytes { done: 50, total: Some(110), stage: Stage::Video })
        );
        t.on_line("[ndm] finished|100|100|NA");
        assert_eq!(
            t.on_line("[ndm] downloading|5|20|NA"),
            Some(MediaProgress::Bytes { done: 105, total: Some(110), stage: Stage::Audio })
        );
        assert_eq!(t.on_line("[ndm-pp] Merger|started"), Some(MediaProgress::Stage(Stage::Merge)));
        assert_eq!(t.on_line("[ndm-done] C:\\v\\a.mp4"), Some(MediaProgress::Done(PathBuf::from("C:\\v\\a.mp4"))));
        assert_eq!(t.on_line("bruit"), None);
        // Valeurs « NA » ou décimales tolérées.
        assert!(t.on_line("[ndm] downloading|NA|NA|1234.5").is_some());
        // Sans estimation préalable : la taille annoncée par yt-dlp est utilisée.
        let mut u = ProgressTracker::new(None, MediaFormat::AudioMp3);
        assert_eq!(
            u.on_line("[ndm] downloading|10|NA|40"),
            Some(MediaProgress::Bytes { done: 10, total: Some(40), stage: Stage::Download })
        );
    }

    #[test]
    fn friendly_errors() {
        assert!(friendly_error("ERROR: Unsupported URL: https://x").contains("Aucune vidéo"));
        assert!(friendly_error("ERROR: [youtube] x: Sign in to confirm you're not a bot").contains("anti-robot"));
        assert!(friendly_error("ERROR: [youtube] x: Video unavailable").contains("indisponible"));
        assert!(friendly_error("ERROR: This video is DRM protected").contains("DRM"));
        assert!(friendly_error("ERROR: étrange").starts_with("yt-dlp : "));
    }

    #[test]
    fn selectors() {
        let a = MediaFormat::Video { max_height: 720 }.ytdlp_args(true);
        assert!(a.contains(&"bv*[height<=720]+ba/b[height<=720]/bv*+ba/b".to_string()));
        assert!(a.contains(&"--embed-subs".to_string()));
        let m = MediaFormat::AudioMp3.ytdlp_args(true);
        assert!(m.contains(&"mp3".to_string()) && !m.contains(&"--embed-subs".to_string()));
    }
}

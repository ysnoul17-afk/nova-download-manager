//! Commandes liées aux vidéos : analyse d'adresse, outils yt-dlp/FFmpeg, ajout d'une vidéo.

use crate::AppState;
use ndm_engine::tools::{self, Tool, ToolsStatus};
use ndm_engine::{AddRequest, DownloadInfo, MediaInfo, MediaSpec, Priority};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

pub const EVENT_TOOLS: &str = "ndm://tools";

pub fn tools_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap_or_else(|_| PathBuf::from(".")).join("tools")
}

/// Détecte les outils et les communique au moteur.
pub async fn refresh_tools(app: &AppHandle) -> ToolsStatus {
    let status = tools::detect(&tools_dir(app)).await;
    if let Some(state) = app.try_state::<AppState>() {
        state.engine.set_media_tools(status.media_tools());
    }
    status
}

#[tauri::command]
pub async fn tools_status(app: AppHandle) -> ToolsStatus {
    refresh_tools(&app).await
}

#[derive(Clone, Serialize)]
struct ToolProgress {
    tool: Tool,
    /// "download" | "done" | "error"
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    error: Option<String>,
}

/// Installe ou met à jour les outils demandés, avec progression diffusée à l'interface.
#[tauri::command]
pub async fn install_tools(app: AppHandle, state: State<'_, AppState>, which: Vec<Tool>) -> Result<ToolsStatus, String> {
    let client = state.engine.http_client();
    let dir = tools_dir(&app);
    let mut errors = Vec::new();
    for tool in which {
        let a = app.clone();
        let progress = move |downloaded: u64, total: Option<u64>| {
            let _ = a.emit(EVENT_TOOLS, ToolProgress { tool, phase: "download", downloaded, total, error: None });
        };
        let res = match tool {
            Tool::YtDlp => tools::install_ytdlp(&client, &dir, &progress).await,
            Tool::Ffmpeg => tools::install_ffmpeg(&client, &dir, &progress).await,
        };
        let (phase, error) = match res {
            Ok(_) => ("done", None),
            Err(e) => {
                errors.push(e.to_string());
                ("error", Some(e.to_string()))
            }
        };
        let _ = app.emit(EVENT_TOOLS, ToolProgress { tool, phase, downloaded: 0, total: None, error });
    }
    let status = refresh_tools(&app).await;
    if errors.is_empty() { Ok(status) } else { Err(errors.join("\n")) }
}

/// Sites dont les adresses sont des pages vidéo, jamais des fichiers directs.
fn is_video_host(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    let host = u.host_str().unwrap_or("").trim_start_matches("www.").trim_start_matches("m.");
    const HOSTS: [&str; 18] = [
        "youtube.com", "youtu.be", "music.youtube.com", "vimeo.com", "dailymotion.com", "dai.ly",
        "twitch.tv", "tiktok.com", "x.com", "twitter.com", "instagram.com", "facebook.com", "fb.watch",
        "reddit.com", "v.redd.it", "soundcloud.com", "bilibili.com", "rumble.com",
    ];
    HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// Liste de lecture HLS (.m3u8) ou manifeste DASH (.mpd) : flux vidéo à assembler, pas un fichier.
fn is_stream_manifest(url: &str, mime: &str) -> bool {
    let path = url::Url::parse(url).map(|u| u.path().to_ascii_lowercase()).unwrap_or_default();
    path.ends_with(".m3u8")
        || path.ends_with(".mpd")
        || mime.contains("mpegurl")
        || mime.contains("dash+xml")
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UrlAnalysis {
    /// Fichier téléchargeable directement.
    File { filename: Option<String>, size: Option<u64>, mime: Option<String> },
    /// Page contenant une ou plusieurs vidéos reconnues.
    Media { info: MediaInfo },
    /// Page web sans vidéo reconnue.
    Page { message: String },
    /// yt-dlp ou FFmpeg manquant pour analyser ou télécharger cette page.
    NeedsTools { missing: Vec<Tool> },
    Error { message: String },
}

#[tauri::command]
pub async fn analyze_url(app: AppHandle, state: State<'_, AppState>, url: String) -> Result<UrlAnalysis, String> {
    let url = url.trim().to_string();
    let parsed = url::Url::parse(&url).map_err(|_| format!("Adresse invalide : {url}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Protocole non pris en charge ({}).", parsed.scheme()));
    }

    let mut probe_error = None;
    if !is_video_host(&url) {
        match state.engine.inspect(&url).await {
            Ok(p) => {
                let mime = p.mime.clone().unwrap_or_default().to_ascii_lowercase();
                let is_page = mime.starts_with("text/html") || mime.starts_with("application/xhtml");
                if !is_page && !is_stream_manifest(&url, &mime) {
                    return Ok(UrlAnalysis::File { filename: p.filename, size: p.total_size, mime: p.mime });
                }
            }
            Err(e) => probe_error = Some(e.to_string()),
        }
    }

    let status = refresh_tools(&app).await;
    let mut missing = Vec::new();
    if status.ytdlp.is_none() {
        missing.push(Tool::YtDlp);
    }
    if status.ffmpeg.is_none() {
        missing.push(Tool::Ffmpeg);
    }
    if !missing.is_empty() {
        // Sans yt-dlp, une erreur réseau sur un site inconnu reste une erreur réseau.
        if let (Some(e), false) = (&probe_error, is_video_host(&url)) {
            if status.ytdlp.is_none() {
                return Ok(UrlAnalysis::Error { message: e.clone() });
            }
        }
        return Ok(UrlAnalysis::NeedsTools { missing });
    }

    match state.engine.analyze_media(&url).await {
        Ok(info) => Ok(UrlAnalysis::Media { info }),
        Err(e) => {
            let message = e.to_string();
            match probe_error {
                // Ni fichier, ni vidéo : on remonte la cause la plus parlante.
                Some(pe) if message.contains("Aucune vidéo") => Ok(UrlAnalysis::Error { message: pe }),
                _ if message.contains("Aucune vidéo") => Ok(UrlAnalysis::Page { message }),
                _ => Ok(UrlAnalysis::Error { message }),
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddMediaOptions {
    pub url: String,
    pub spec: MediaSpec,
    pub directory: Option<PathBuf>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub start_paused: bool,
}

#[tauri::command]
pub fn add_media(state: State<'_, AppState>, options: AddMediaOptions) -> Result<DownloadInfo, String> {
    state
        .engine
        .add(AddRequest {
            url: options.url,
            directory: options.directory,
            priority: options.priority,
            start_paused: options.start_paused,
            size_hint: options.spec.est_size,
            media: Some(options.spec),
            ..Default::default()
        })
        .map_err(|e| e.to_string())
}

/// Pour l'ajout par lots : vidéo « meilleure qualité » si l'adresse est un site vidéo connu.
pub fn batch_media_spec(url: &str) -> Option<MediaSpec> {
    is_video_host(url).then(|| MediaSpec {
        format: ndm_engine::MediaFormat::Best,
        subtitles: false,
        label: "Meilleure qualité MP4".into(),
        title: String::new(),
        est_size: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_hosts() {
        assert!(is_video_host("https://youtu.be/_n61GGUD6Pc"));
        assert!(is_video_host("https://www.youtube.com/watch?v=x"));
        assert!(is_video_host("https://m.youtube.com/watch?v=x"));
        assert!(is_video_host("https://player.vimeo.com/video/1"));
        assert!(!is_video_host("https://notyoutube.com/x"));
        assert!(!is_video_host("https://proof.ovh.net/files/100Mb.dat"));
    }

    #[test]
    fn manifests() {
        assert!(is_stream_manifest("https://cdn.x/live/master.m3u8?token=1", "application/octet-stream"));
        assert!(is_stream_manifest("https://cdn.x/a/manifest", "application/dash+xml"));
        assert!(!is_stream_manifest("https://cdn.x/a/film.mp4", "video/mp4"));
    }
}

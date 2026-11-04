//! Intégration système : notifications, surveillance du presse-papiers, démarrage avec Windows,
//! mises à jour automatiques signées.

use crate::{show_main_window, AppState};
use ndm_engine::{DownloadId, DownloadInfo, DownloadStatus, EngineEvent};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

pub const EVENT_CLIPBOARD: &str = "ndm://clipboard";
pub const EVENT_UPDATE: &str = "ndm://update";

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Suit les changements d'état et notifie la fin ou l'échec d'un téléchargement.
pub struct Notifier {
    last: HashMap<DownloadId, DownloadStatus>,
}

impl Notifier {
    pub fn new(initial: &[DownloadInfo]) -> Self {
        Self { last: initial.iter().map(|i| (i.id, i.status)).collect() }
    }

    pub fn on_event(&mut self, app: &AppHandle, ev: &EngineEvent) {
        let info = match ev {
            EngineEvent::StatusChanged { info } | EngineEvent::Added { info } => info,
            EngineEvent::Removed { id } => {
                self.last.remove(id);
                return;
            }
            _ => return,
        };
        let before = self.last.insert(info.id, info.status);
        // Seules les transitions observées pendant cette session déclenchent une notification.
        let was_running = matches!(
            before,
            Some(DownloadStatus::Probing | DownloadStatus::Downloading | DownloadStatus::Finalizing)
        );
        if !was_running || !matches!(info.status, DownloadStatus::Completed | DownloadStatus::Failed) {
            return;
        }
        let Some(state) = app.try_state::<AppState>() else { return };
        if !state.ui_settings().notifications {
            return;
        }
        // Fenêtre au premier plan : l'utilisateur voit déjà le résultat.
        let focused = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
        if focused {
            return;
        }
        let name = if info.filename.is_empty() { info.url.clone() } else { info.filename.clone() };
        let (title, body) = if info.status == DownloadStatus::Completed {
            let size = info.total_size.map(format_size).map(|s| format!(" ({s})")).unwrap_or_default();
            ("Téléchargement terminé".to_string(), format!("{name}{size}"))
        } else {
            let why = info.error.clone().unwrap_or_default();
            ("Échec du téléchargement".to_string(), format!("{name}\n{why}"))
        };
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            tracing::warn!("notification impossible : {e}");
        }
    }
}

/// Notification ponctuelle (interception, doublon), si l'utilisateur les a activées
/// et que la fenêtre de NDM n'est pas déjà au premier plan.
pub fn notify(app: &AppHandle, title: &str, body: &str) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if !state.ui_settings().notifications {
        return;
    }
    let focused = app
        .get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false);
    if focused {
        return;
    }
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("notification impossible : {e}");
    }
}

fn format_size(b: u64) -> String {
    const U: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{v:.0} {}", U[i]) } else { format!("{v:.1} {}", U[i]).replace('.', ",") }
}

// ---------------------------------------------------------------------------
// Presse-papiers
// ---------------------------------------------------------------------------

/// Extensions qui méritent de proposer un téléchargement quand une adresse est copiée.
const CLIPBOARD_EXTS: &[&str] = &[
    "3gp", "7z", "aac", "apk", "avi", "bin", "bz2", "dmg", "epub", "exe", "flac", "gz", "img", "iso", "m4a", "m4v",
    "mkv", "mov", "mp3", "mp4", "mpg", "msi", "msix", "ogg", "opus", "pdf", "rar", "tar", "tgz", "wav", "webm", "wmv",
    "xz", "zip", "zst", "m3u8", "mpd",
];

/// Adresse téléchargeable dans un texte copié (fichier reconnu ou page vidéo connue).
pub fn downloadable_url(text: &str) -> Option<String> {
    let text = text.trim();
    // Un texte long n'est pas une adresse copiée volontairement.
    if text.is_empty() || text.len() > 4096 || text.lines().count() > 3 {
        return None;
    }
    let start = text.find("http://").or_else(|| text.find("https://"))?;
    let url: String = text[start..].chars().take_while(|c| !c.is_whitespace() && !"\"'<>".contains(*c)).collect();
    let parsed = url::Url::parse(&url).ok()?;
    let path = parsed.path().to_ascii_lowercase();
    let ext = path.rsplit('/').next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_string());
    let known_file = ext.is_some_and(|e| CLIPBOARD_EXTS.contains(&e.as_str()));
    (known_file || crate::media_cmds::is_video_host(&url)).then_some(url)
}

/// Surveille le presse-papiers (si l'utilisateur l'a activé) et propose les liens copiés.
pub fn start_clipboard_watch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Le contenu présent au démarrage n'est jamais proposé.
        let mut last = app.clipboard().read_text().unwrap_or_default();
        loop {
            tokio::time::sleep(Duration::from_millis(900)).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            if !state.ui_settings().clipboard_watch {
                last = app.clipboard().read_text().unwrap_or_default();
                continue;
            }
            let Ok(text) = app.clipboard().read_text() else { continue };
            if text == last {
                continue;
            }
            last = text.clone();
            let Some(url) = downloadable_url(&text) else { continue };
            // Déjà dans la liste (y compris « Copier l'adresse » depuis NDM) : rien à proposer.
            if state.engine.list().iter().any(|d| d.url == url || d.final_url.as_deref() == Some(url.as_str())) {
                continue;
            }
            show_main_window(&app);
            let _ = app.emit(EVENT_CLIPBOARD, url);
        }
    });
}

// ---------------------------------------------------------------------------
// Démarrage avec Windows
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let al = app.autolaunch();
    let r = if enabled { al.enable() } else { al.disable() };
    r.map_err(|e| format!("Réglage du démarrage automatique impossible : {e}"))?;
    Ok(al.is_enabled().unwrap_or(enabled))
}

// ---------------------------------------------------------------------------
// Mises à jour
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    pub date: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
enum UpdateProgress {
    Available { info: UpdateInfo },
    Download { downloaded: u64, total: Option<u64> },
    Installing,
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut b = app.updater_builder();
    // Adresse de test (serveur local) ; en production, celle de tauri.conf.json.
    if let Ok(ep) = std::env::var("NDM_UPDATE_ENDPOINT") {
        let url = ep.parse().map_err(|e| format!("adresse de mise à jour invalide : {e}"))?;
        b = b.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    b.timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())
}

fn friendly_update_error(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    let l = s.to_ascii_lowercase();
    if l.contains("signature") {
        "La mise à jour n'a pas une signature valide : elle a été refusée.".into()
    } else if l.contains("404") || l.contains("not found") || l.contains("release") {
        "Aucune version publiée n'a été trouvée.".into()
    } else if l.contains("network") || l.contains("connect") || l.contains("dns") || l.contains("timed out") {
        "Serveur de mises à jour injoignable. Vérifiez votre connexion.".into()
    } else {
        format!("Mise à jour impossible : {s}")
    }
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = updater(&app)?.check().await.map_err(friendly_update_error)?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.map(|d| d.to_string()),
    }))
}

/// Télécharge, vérifie la signature, enregistre l'état des téléchargements puis installe.
/// Sous Windows, l'installateur ferme l'application et la relance une fois la mise à jour faite.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let Some(update) = updater(&app)?.check().await.map_err(friendly_update_error)? else {
        return Err("Vous avez déjà la dernière version.".into());
    };
    let a = app.clone();
    let mut downloaded = 0u64;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = a.emit(EVENT_UPDATE, UpdateProgress::Download { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(friendly_update_error)?;
    let _ = app.emit(EVENT_UPDATE, UpdateProgress::Installing);
    // État enregistré sans arrêter le moteur : si l'installation échoue, NDM continue normalement ;
    // si elle réussit, les téléchargements en cours reprennent au redémarrage.
    if let Some(state) = app.try_state::<AppState>() {
        state.engine.save().await;
    }
    update.install(bytes).map_err(friendly_update_error)?;
    app.restart();
}

/// Vérification discrète quelques secondes après le démarrage.
pub fn schedule_update_check(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        let enabled = app.try_state::<AppState>().map(|s| s.ui_settings().auto_update_check).unwrap_or(false);
        if !enabled {
            return;
        }
        match check_update(app.clone()).await {
            Ok(Some(info)) => {
                tracing::info!("mise à jour disponible : {}", info.version);
                let _ = app.emit(EVENT_UPDATE, UpdateProgress::Available { info });
            }
            Ok(None) => {}
            Err(e) => tracing::info!("vérification des mises à jour : {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_urls() {
        assert_eq!(downloadable_url("https://ex.com/a/setup.exe").as_deref(), Some("https://ex.com/a/setup.exe"));
        assert_eq!(downloadable_url("  voir https://ex.com/film.MKV  ").as_deref(), Some("https://ex.com/film.MKV"));
        assert!(downloadable_url("https://youtu.be/_n61GGUD6Pc").is_some());
        assert!(downloadable_url("https://ex.com/article").is_none(), "page ordinaire");
        assert!(downloadable_url("bonjour").is_none());
        assert!(downloadable_url(&"https://ex.com/a.zip\n".repeat(10)).is_none(), "long texte ignoré");
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 o");
        assert_eq!(format_size(92 * 1024 * 1024), "92,0 Mo");
    }
}

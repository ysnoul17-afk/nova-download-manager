//! Commandes appelées par l'interface React.
//!
//! Toutes renvoient `Result<_, String>` : le message est affiché tel quel à l'utilisateur.

use crate::{AppState, UiSettings};
use ndm_engine::{
    AddRequest, Category, DownloadId, DownloadInfo, DownloadStatus, EngineConfig, HistoryQuery, Priority,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> Vec<DownloadInfo> {
    state.engine.list()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddOptions {
    pub urls: Vec<String>,
    pub directory: Option<PathBuf>,
    pub filename: Option<String>,
    #[serde(default)]
    pub start_paused: bool,
    #[serde(default)]
    pub priority: Priority,
    pub connections: Option<u8>,
    /// Ajouter même si le fichier a déjà été téléchargé.
    #[serde(default)]
    pub allow_duplicates: bool,
}

#[derive(Debug, Serialize)]
pub struct AddFailure {
    pub url: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct AddResult {
    pub added: Vec<DownloadInfo>,
    pub failed: Vec<AddFailure>,
    /// Adresses déjà téléchargées (fichier présent), non ajoutées : à confirmer.
    pub duplicates: Vec<DownloadInfo>,
}

/// Ajout unitaire ou par lots. Les lignes vides et les doublons sont ignorés.
#[tauri::command]
pub fn add_downloads(state: State<'_, AppState>, options: AddOptions) -> AddResult {
    let mut seen = std::collections::HashSet::new();
    let urls: Vec<String> = options
        .urls
        .iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty() && seen.insert(u.clone()))
        .collect();
    // Un nom imposé n'a de sens que pour un seul fichier.
    let filename = if urls.len() == 1 { options.filename.filter(|f| !f.trim().is_empty()) } else { None };

    let mut added = Vec::new();
    let mut failed = Vec::new();
    let mut duplicates = Vec::new();
    for url in urls {
        // Ajout par lots : les adresses de sites vidéo passent par yt-dlp (meilleure qualité).
        let media = crate::media_cmds::batch_media_spec(&url);
        if !options.allow_duplicates {
            if let Some(d) = state.engine.find_completed(&url, media.as_ref().map(|m| m.label.as_str())) {
                duplicates.push(d);
                continue;
            }
        }
        let req = AddRequest {
            url: url.clone(),
            filename: if media.is_some() { None } else { filename.clone() },
            media,
            directory: options.directory.clone(),
            priority: options.priority,
            start_paused: options.start_paused,
            connections: options.connections,
            ..Default::default()
        };
        match state.engine.add(req) {
            Ok(i) => added.push(i),
            Err(e) => failed.push(AddFailure { url, error: e.to_string() }),
        }
    }
    AddResult { added, failed, duplicates }
}

/// « Télécharger à nouveau » pour un doublon reçu du navigateur.
#[tauri::command]
pub fn redownload_duplicate(state: State<'_, AppState>, key: String) -> CmdResult<DownloadInfo> {
    let req = state
        .pending_duplicates
        .lock()
        .map_err(err)?
        .remove(&key)
        .ok_or("Cette demande a expiré : relancez le téléchargement depuis le navigateur.")?;
    state.engine.add(req).map_err(err)
}

#[tauri::command]
pub fn dismiss_duplicate(state: State<'_, AppState>, key: String) {
    if let Ok(mut p) = state.pending_duplicates.lock() {
        p.remove(&key);
    }
}

fn for_each(ids: Vec<DownloadId>, mut f: impl FnMut(DownloadId) -> ndm_engine::Result<()>) -> CmdResult<()> {
    let errors: Vec<String> = ids.into_iter().filter_map(|id| f(id).err()).map(err).collect();
    if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
}

#[tauri::command]
pub fn pause_downloads(state: State<'_, AppState>, ids: Vec<DownloadId>) -> CmdResult<()> {
    for_each(ids, |id| state.engine.pause(id))
}

#[tauri::command]
pub fn resume_downloads(state: State<'_, AppState>, ids: Vec<DownloadId>) -> CmdResult<()> {
    for_each(ids, |id| state.engine.resume(id))
}

#[tauri::command]
pub fn stop_downloads(state: State<'_, AppState>, ids: Vec<DownloadId>) -> CmdResult<()> {
    for_each(ids, |id| state.engine.stop(id))
}

#[tauri::command]
pub async fn restart_downloads(state: State<'_, AppState>, ids: Vec<DownloadId>) -> CmdResult<()> {
    for id in ids {
        state.engine.restart(id).await.map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn remove_downloads(state: State<'_, AppState>, ids: Vec<DownloadId>, delete_files: bool) -> CmdResult<()> {
    for id in ids {
        state.engine.remove(id, delete_files).await.map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_priority(state: State<'_, AppState>, ids: Vec<DownloadId>, priority: Priority) -> CmdResult<()> {
    for_each(ids, |id| state.engine.set_priority(id, priority))
}

#[tauri::command]
pub fn set_download_speed_limit(state: State<'_, AppState>, id: DownloadId, bytes_per_sec: u64) -> CmdResult<()> {
    state.engine.set_speed_limit(id, bytes_per_sec).map_err(err)
}

#[derive(Serialize)]
pub struct SegmentView {
    start: u64,
    end: Option<u64>,
    downloaded: u64,
    active: bool,
}

#[tauri::command]
pub fn get_segments(state: State<'_, AppState>, id: DownloadId) -> Vec<SegmentView> {
    let mut v: Vec<SegmentView> = state
        .engine
        .segments(id)
        .unwrap_or_default()
        .into_iter()
        .map(|s| SegmentView { start: s.start, end: s.end, downloaded: s.downloaded, active: s.active })
        .collect();
    v.sort_by_key(|s| s.start);
    v
}

#[tauri::command]
pub fn pause_all(state: State<'_, AppState>) {
    state.engine.pause_all();
}

#[tauri::command]
pub fn resume_all(state: State<'_, AppState>) {
    state.engine.resume_all();
}

#[tauri::command]
pub async fn query_history(state: State<'_, AppState>, query: HistoryQuery) -> CmdResult<Vec<DownloadInfo>> {
    state.engine.query(query).await.map_err(err)
}

/// Ouvre un fichier terminé. On ne reçoit qu'un identifiant, jamais un chemin libre :
/// l'interface ne peut pas faire ouvrir un fichier arbitraire.
#[tauri::command]
pub fn open_file(app: tauri::AppHandle, state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    let info = state.engine.get(id).ok_or("Téléchargement introuvable.")?;
    if info.status != DownloadStatus::Completed {
        return Err("Le fichier n'est pas encore terminé.".into());
    }
    let path = info.full_path();
    if !path.exists() {
        return Err("Le fichier a été déplacé ou supprimé.".into());
    }
    app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(err)
}

/// Affiche le fichier dans l'Explorateur (ou le dossier s'il n'existe pas encore).
#[tauri::command]
pub fn open_folder(app: tauri::AppHandle, state: State<'_, AppState>, id: DownloadId) -> CmdResult<()> {
    let info = state.engine.get(id).ok_or("Téléchargement introuvable.")?;
    let path = info.full_path();
    if info.status == DownloadStatus::Completed && path.exists() {
        return app.opener().reveal_item_in_dir(&path).map_err(err);
    }
    if !info.directory.exists() {
        return Err("Le dossier de destination n'existe pas encore.".into());
    }
    app.opener().open_path(info.directory.to_string_lossy(), None::<&str>).map_err(err)
}

#[tauri::command]
pub fn get_engine_config(state: State<'_, AppState>) -> EngineConfig {
    state.engine.config()
}

#[tauri::command]
pub fn set_engine_config(state: State<'_, AppState>, config: EngineConfig) -> CmdResult<EngineConfig> {
    state.engine.set_config(config).map_err(err)
}

#[tauri::command]
pub fn get_ui_settings(state: State<'_, AppState>) -> UiSettings {
    state.ui_settings()
}

#[tauri::command]
pub fn set_ui_settings(state: State<'_, AppState>, settings: UiSettings) -> CmdResult<()> {
    state.save_ui_settings(&settings)
}

/// Dossier effectif de chaque catégorie (réglage ou valeur par défaut).
#[tauri::command]
pub fn get_category_directories(state: State<'_, AppState>) -> BTreeMap<Category, PathBuf> {
    let cfg = state.engine.config();
    Category::ALL.iter().map(|c| (*c, cfg.directory_for(*c))).collect()
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    data_dir: String,
}

#[tauri::command]
pub fn app_info(app: tauri::AppHandle) -> AppInfo {
    use tauri::Manager;
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        data_dir: app.path().app_data_dir().map(|p| p.display().to_string()).unwrap_or_default(),
    }
}

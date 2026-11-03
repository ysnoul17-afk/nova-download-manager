//! Application de bureau Nova Download Manager (Tauri 2).

mod commands;
mod media_cmds;
mod tray;

use ndm_engine::{Engine, EngineConfig, EngineEvent};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

pub const EVENT_ENGINE: &str = "ndm://engine";
pub const EVENT_RESYNC: &str = "ndm://resync";
const UI_SETTINGS_KEY: &str = "ui_settings";

/// Réglages propres à l'interface (le moteur garde les siens).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// Fermer la fenêtre réduit l'application dans la zone de notification.
    pub close_to_tray: bool,
    /// Demander confirmation avant d'ouvrir un exécutable téléchargé.
    pub confirm_open_executables: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { theme: "system".into(), close_to_tray: true, confirm_open_executables: true }
    }
}

pub struct AppState {
    pub engine: Engine,
}

impl AppState {
    pub fn ui_settings(&self) -> UiSettings {
        self.engine
            .store()
            .get_setting(UI_SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save_ui_settings(&self, s: &UiSettings) -> Result<(), String> {
        let json = serde_json::to_string(s).map_err(|e| e.to_string())?;
        self.engine.store().set_setting(UI_SETTINGS_KEY, &json).map_err(|e| e.to_string())
    }
}

pub fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Arrête proprement le moteur (état enregistré) puis quitte.
pub fn quit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(state) = app.try_state::<AppState>() {
            state.engine.shutdown().await;
        }
        app.exit(0);
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,ndm_engine=info".into()),
        )
        .init();

    let app = tauri::Builder::default()
        // Une seule instance : relancer l'application ramène la fenêtre existante.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let engine = tauri::async_runtime::block_on(async {
                Engine::start(&data_dir, EngineConfig::default())
            })?;
            tracing::info!("données : {}", data_dir.display());

            // Relais des événements du moteur vers l'interface.
            let mut rx = engine.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = handle.emit(EVENT_ENGINE, &ev);
                            if let EngineEvent::Stats { total_speed, active, .. } = &ev {
                                tray::update_tooltip(&handle, *active, *total_speed);
                            }
                        }
                        // Trop d'événements en retard : l'interface recharge la liste complète.
                        Err(RecvError::Lagged(_)) => {
                            let _ = handle.emit(EVENT_RESYNC, ());
                        }
                        Err(RecvError::Closed) => break,
                    }
                }
            });

            app.manage(AppState { engine });
            tray::create(app.handle())?;

            // Détection de yt-dlp et FFmpeg en arrière-plan (sans retarder l'ouverture).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                media_cmds::refresh_tools(&handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let to_tray = app.try_state::<AppState>().map(|s| s.ui_settings().close_to_tray).unwrap_or(true);
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    api.prevent_close();
                    quit(app);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_downloads,
            commands::add_downloads,
            commands::pause_downloads,
            commands::resume_downloads,
            commands::stop_downloads,
            commands::restart_downloads,
            commands::remove_downloads,
            commands::set_priority,
            commands::set_download_speed_limit,
            commands::get_segments,
            commands::pause_all,
            commands::resume_all,
            commands::query_history,
            commands::open_file,
            commands::open_folder,
            commands::get_engine_config,
            commands::set_engine_config,
            commands::get_ui_settings,
            commands::set_ui_settings,
            commands::get_category_directories,
            commands::app_info,
            media_cmds::tools_status,
            media_cmds::install_tools,
            media_cmds::analyze_url,
            media_cmds::add_media,
        ])
        .build(tauri::generate_context!())
        .expect("impossible de démarrer Nova Download Manager");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { api, code, .. } = &event {
            // Fermeture demandée par le système (ex. arrêt de Windows) : on enregistre d'abord.
            if code.is_none() {
                api.prevent_exit();
                quit(handle);
            }
        }
    });
}

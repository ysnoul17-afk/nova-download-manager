//! Intégration avec l'extension du navigateur : serveur local et enregistrement de l'hôte natif.

use crate::{media_cmds, show_main_window, system, AppState};
use ndm_engine::{AddRequest, DownloadStatus, MediaSpec};
use ndm_ipc::{server, Reply, HOST_NAME};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

/// Identifiant fixe de l'extension (dérivé de la clé publique de son manifeste).
pub const EXTENSION_ID: &str = "eogmknmllnjfblmecmcngbbpgikhigjc";

/// Démarre le serveur local qui reçoit les requêtes de l'hôte natif.
pub fn start_server(app: &AppHandle, data_dir: PathBuf) {
    let handle = app.clone();
    let handler: server::Handler = Arc::new(move |req: Value| {
        let app = handle.clone();
        Box::pin(async move { handle_request(&app, req).await })
    });
    tauri::async_runtime::spawn(async move {
        if let Err(e) = server::serve(data_dir, handler).await {
            tracing::error!("serveur de l'extension indisponible : {e}");
        }
    });
}

fn str_field<'a>(req: &'a Value, key: &str) -> Option<&'a str> {
    req.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

async fn handle_request(app: &AppHandle, req: Value) -> Reply {
    let Some(state) = app.try_state::<AppState>() else {
        return Reply::err("starting", "Nova Download Manager démarre, réessayez dans un instant.");
    };
    let engine = &state.engine;
    let ty = req.get("type").and_then(Value::as_str).unwrap_or_default();
    match ty {
        "ping" => Reply::ok(json!({
            "app": "Nova Download Manager",
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": ndm_ipc::PROTOCOL_VERSION,
        })),

        // Fichier intercepté ou envoyé depuis le menu contextuel.
        "download" => {
            let Some(url) = str_field(&req, "url") else {
                return Reply::err("invalid_request", "Adresse manquante.");
            };
            let mut headers = BTreeMap::new();
            if let Some(r) = str_field(&req, "referrer") {
                headers.insert("Referer".to_string(), r.to_string());
            }
            if let Some(ua) = str_field(&req, "user_agent") {
                headers.insert("User-Agent".to_string(), ua.to_string());
            }
            // Cookies : uniquement si l'utilisateur l'a autorisé dans l'extension.
            if let Some(c) = str_field(&req, "cookies") {
                headers.insert("Cookie".to_string(), c.to_string());
            }
            // Vérification avant d'accepter : si NDM ne peut pas récupérer ce fichier (lien à usage
            // unique, session requise…), le navigateur garde son téléchargement.
            match engine.inspect_with(url, &headers).await {
                Err(e) => return Reply::err("probe_failed", e.to_string()),
                Ok(p) => {
                    let mime = p.mime.unwrap_or_default().to_ascii_lowercase();
                    let html_expected = url.to_ascii_lowercase().split('?').next().unwrap_or("").ends_with(".html");
                    if mime.starts_with("text/html") && !html_expected {
                        return Reply::err(
                            "not_a_file",
                            "Le serveur renvoie une page web au lieu du fichier (connexion requise ?).",
                        );
                    }
                }
            }
            let request = AddRequest {
                url: url.to_string(),
                filename: str_field(&req, "filename").map(str::to_string),
                headers,
                size_hint: req.get("size").and_then(Value::as_u64).filter(|s| *s > 0),
                ..Default::default()
            };
            if let Some(existing) = engine.find_completed(url, None) {
                return hold_duplicate(app, &state, existing, request);
            }
            match engine.add(request) {
                Ok(i) => {
                    let name = if i.filename.is_empty() { url.to_string() } else { i.filename.clone() };
                    system::notify(app, "Repris par Nova Download Manager", &name);
                    Reply::ok(json!({ "id": i.id, "filename": i.filename }))
                }
                Err(e) => Reply::err("add_failed", e.to_string()),
            }
        }

        "analyze" => {
            let Some(url) = str_field(&req, "url") else {
                return Reply::err("invalid_request", "Adresse manquante.");
            };
            match media_cmds::analyze(app, engine, url).await {
                Ok(a) => Reply::ok(serde_json::to_value(a).unwrap_or(Value::Null)),
                Err(e) => Reply::err("analyze_failed", e),
            }
        }

        "download_media" => {
            let Some(url) = str_field(&req, "url") else {
                return Reply::err("invalid_request", "Adresse manquante.");
            };
            let spec: MediaSpec = match req.get("spec").cloned().map(serde_json::from_value) {
                Some(Ok(s)) => s,
                _ => return Reply::err("invalid_request", "Qualité invalide."),
            };
            let label = spec.label.clone();
            let request = AddRequest {
                url: url.to_string(),
                size_hint: spec.est_size,
                media: Some(spec),
                ..Default::default()
            };
            if let Some(existing) = engine.find_completed(url, Some(&label)) {
                return hold_duplicate(app, &state, existing, request);
            }
            match engine.add(request) {
                Ok(i) => Reply::ok(json!({ "id": i.id })),
                Err(e) => Reply::err("add_failed", e.to_string()),
            }
        }

        // Pour la fenêtre de l'extension : téléchargements en cours et en attente.
        "status" => {
            let list = engine.list();
            let items: Vec<Value> = list
                .iter()
                .filter(|i| {
                    i.status.is_active() || matches!(i.status, DownloadStatus::Queued | DownloadStatus::Paused)
                })
                .take(8)
                .map(|i| {
                    json!({
                        "id": i.id, "filename": i.filename, "status": i.status, "downloaded": i.downloaded,
                        "total": i.total_size, "speed": i.speed, "media_label": i.media_label, "stage": i.stage,
                    })
                })
                .collect();
            let active = list.iter().filter(|i| i.status.is_active()).count();
            let queued = list.iter().filter(|i| i.status == DownloadStatus::Queued).count();
            Reply::ok(json!({ "version": env!("CARGO_PKG_VERSION"), "active": active, "queued": queued, "items": items }))
        }

        "show" => {
            show_main_window(app);
            Reply::ok(Value::Null)
        }

        _ => Reply::err("invalid_request", "Requête inconnue."),
    }
}

pub const EVENT_DUPLICATE: &str = "ndm://duplicate";

/// Fichier déjà téléchargé : on ne crée pas de copie, on demande à l'utilisateur.
/// La requête (avec ses en-têtes) est conservée pour « Télécharger à nouveau ».
fn hold_duplicate(app: &AppHandle, state: &AppState, existing: ndm_engine::DownloadInfo, request: AddRequest) -> Reply {
    let key = uuid::Uuid::new_v4().to_string();
    if let Ok(mut p) = state.pending_duplicates.lock() {
        // Pas d'accumulation si l'utilisateur ignore les demandes.
        if p.len() > 20 {
            p.clear();
        }
        p.insert(key.clone(), request);
    }
    show_main_window(app);
    let _ = app.emit(EVENT_DUPLICATE, json!({ "key": key, "existing": existing }));
    Reply::ok(json!({ "duplicate": true, "id": existing.id, "filename": existing.filename }))
}

// ---------------------------------------------------------------------------
// Enregistrement de l'hôte natif auprès des navigateurs
// ---------------------------------------------------------------------------

fn host_executable() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "ndm-native-host.exe" } else { "ndm-native-host" };
    let p = exe.parent()?.join(name);
    p.is_file().then_some(p)
}

fn manifest(host: &Path) -> Value {
    json!({
        "name": HOST_NAME,
        "description": "Nova Download Manager",
        "path": host,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    })
}

/// Déclare l'hôte natif à Chrome, Edge, Chromium et Brave pour l'utilisateur courant.
/// Refait à chaque démarrage : le chemin change si l'application est réinstallée ailleurs.
pub fn register_native_host(data_dir: &Path) -> Result<PathBuf, String> {
    let host = host_executable().ok_or("composant ndm-native-host absent à côté de l'application")?;
    let json = serde_json::to_string_pretty(&manifest(&host)).map_err(|e| e.to_string())?;
    let dir = data_dir.join("native-messaging");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let manifest_path = dir.join(format!("{HOST_NAME}.json"));
    std::fs::write(&manifest_path, &json).map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        for base in [
            r"Software\Google\Chrome\NativeMessagingHosts",
            r"Software\Microsoft\Edge\NativeMessagingHosts",
            r"Software\Chromium\NativeMessagingHosts",
            r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
        ] {
            let key = format!(r"HKCU\{base}\{HOST_NAME}");
            let status = std::process::Command::new(ndm_engine::media::system32("reg.exe"))
                .args(["add", &key, "/ve", "/t", "REG_SZ", "/d"])
                .arg(&manifest_path)
                .arg("/f")
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("écriture du registre impossible ({key})"));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let config = dirs::config_dir().ok_or("dossier de configuration introuvable")?;
        for browser in [
            "google-chrome",
            "google-chrome-for-testing",
            "chromium",
            "microsoft-edge",
            "BraveSoftware/Brave-Browser",
        ] {
            let d = config.join(browser).join("NativeMessagingHosts");
            if std::fs::create_dir_all(&d).is_ok() {
                let _ = std::fs::write(d.join(format!("{HOST_NAME}.json")), &json);
            }
        }
    }
    Ok(manifest_path)
}

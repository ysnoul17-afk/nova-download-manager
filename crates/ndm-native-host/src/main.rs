//! Hôte Native Messaging de Nova Download Manager.
//!
//! Lancé par Chrome ou Edge (jamais par l'utilisateur), il lit les messages de l'extension sur
//! l'entrée standard, les valide, les relaie à l'application par le canal local et renvoie la
//! réponse. Si l'application n'est pas lancée, il la démarre réduite dans la zone de notification
//! (sauf si la requête précise `"launch": false`).

#![cfg_attr(windows, windows_subsystem = "windows")]

use ndm_ipc::{data_dir, is_not_running, read_native_message, request, validate_request, write_native_message, Reply};
use serde_json::Value;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn app_executable() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NDM_APP_PATH") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "nova-download-manager.exe" } else { "nova-download-manager" };
    let p = exe.parent()?.join(name);
    p.is_file().then_some(p)
}

fn launch_app() -> io::Result<()> {
    let exe = app_executable().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "application introuvable"))?;
    let mut c = Command::new(exe);
    c.arg("--minimized");
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    c.spawn().map(|_| ())
}

fn forward(msg: &Value) -> Reply {
    if let Err(e) = validate_request(msg) {
        return Reply::err("invalid_request", e);
    }
    let Some(dir) = data_dir() else {
        return Reply::err("internal", "Dossier de données introuvable.");
    };
    let may_launch = msg.get("launch").and_then(Value::as_bool).unwrap_or(true);

    let first = request(&dir, msg);
    match first {
        Ok(r) => return r,
        Err(ref e) if !is_not_running(e) && e.kind() != io::ErrorKind::NotFound => {
            return Reply::err("ipc_error", format!("Communication avec l'application impossible : {e}"));
        }
        Err(_) => {}
    }
    if !may_launch {
        return Reply::err("app_not_running", "Nova Download Manager n'est pas lancé.");
    }
    if let Err(e) = launch_app() {
        return Reply::err("app_not_installed", format!("Impossible de lancer Nova Download Manager : {e}"));
    }
    // L'application crée le canal en une à deux secondes.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        std::thread::sleep(Duration::from_millis(250));
        match request(&dir, msg) {
            Ok(r) => return r,
            Err(e) if Instant::now() < deadline && (is_not_running(&e) || e.kind() == io::ErrorKind::NotFound) => {}
            Err(e) => return Reply::err("app_not_running", format!("Nova Download Manager ne répond pas : {e}")),
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = BufReader::new(stdin.lock());
    let mut output = BufWriter::new(stdout.lock());
    loop {
        let msg = match read_native_message(&mut input) {
            Ok(Some(m)) => m,
            Ok(None) => return, // le navigateur a fermé le canal
            Err(e) => {
                let _ = write_native_message(&mut output, &Reply::err("invalid_request", e.to_string()).to_value());
                return;
            }
        };
        // L'identifiant de requête de l'extension est renvoyé tel quel.
        let id = msg.get("id").cloned();
        let mut reply = forward(&msg).to_value();
        if let (Some(id), Some(obj)) = (id, reply.as_object_mut()) {
            obj.insert("id".into(), id);
        }
        if write_native_message(&mut output, &reply).is_err() {
            return;
        }
    }
}

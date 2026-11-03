//! Test de l'exécutable hôte : framing Native Messaging, relais vers un serveur, application absente.

use ndm_ipc::{read_native_message, server, write_native_message, Reply};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

fn run_host(data_dir: &std::path::Path, msgs: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ndm-native-host"))
        .env("NDM_DATA_DIR", data_dir)
        .env("NDM_APP_PATH", "/nonexistent/nova-download-manager")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for m in msgs {
            write_native_message(&mut stdin, m).unwrap();
        }
        stdin.flush().unwrap();
    } // fermeture de stdin : l'hôte se termine après les réponses
    let out = child.wait_with_output().unwrap();
    let mut r = &out.stdout[..];
    let mut replies = Vec::new();
    while let Some(v) = read_native_message(&mut r).unwrap() {
        replies.push(v);
    }
    replies
}

#[tokio::test(flavor = "multi_thread")]
async fn relays_requests_and_keeps_ids() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_path_buf();
    let handler: server::Handler = Arc::new(|req: Value| {
        Box::pin(async move { Reply::ok(json!({"type": req["type"], "url": req.get("url")})) })
    });
    tokio::spawn(server::serve(dir.clone(), handler));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let replies = tokio::task::spawn_blocking(move || {
        run_host(
            &dir,
            &[
                json!({"id": 1, "type": "ping"}),
                json!({"id": 2, "type": "download", "url": "https://exemple.fr/a.zip"}),
                json!({"id": 3, "type": "download", "url": "file:///etc/passwd"}),
            ],
        )
    })
    .await
    .unwrap();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["ok"], true);
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["data"]["url"], "https://exemple.fr/a.zip");
    assert_eq!(replies[2]["ok"], false);
    assert_eq!(replies[2]["code"], "invalid_request");
}

#[test]
fn reports_app_not_running_without_launch() {
    let d = tempfile::tempdir().unwrap();
    ndm_ipc::ensure_token(d.path()).unwrap();
    let replies = run_host(d.path(), &[json!({"type": "status", "launch": false})]);
    assert_eq!(replies[0]["code"], "app_not_running");
    // Avec lancement autorisé mais application introuvable : message explicite.
    let replies = run_host(d.path(), &[json!({"type": "ping"})]);
    assert_eq!(replies[0]["code"], "app_not_installed");
}

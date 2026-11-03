//! # ndm-ipc
//!
//! Communication entre l'extension du navigateur et l'application Nova Download Manager.
//!
//! ```text
//! Extension ──Native Messaging (stdio)──▶ ndm-native-host ──canal local + jeton──▶ application
//! ```
//!
//! - **Native Messaging** : chaque message est un JSON précédé de sa longueur (u32 little-endian).
//! - **Canal local** : canal nommé Windows (`\\.\pipe\…`, clients distants refusés) ou socket Unix
//!   dans le dossier de données. Aucun port réseau n'est ouvert.
//! - **Jeton** : 256 bits aléatoires stockés dans le dossier de données de l'utilisateur ; toute
//!   requête sans le bon jeton est rejetée. Seul un programme de cette session utilisateur peut le lire.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

/// Nom de l'hôte natif déclaré au navigateur.
pub const HOST_NAME: &str = "app.novadm.host";
/// Identifiant Tauri de l'application (nom du dossier de données).
pub const APP_IDENTIFIER: &str = "app.novadownloadmanager.desktop";
pub const PROTOCOL_VERSION: u32 = 1;
/// Taille maximale d'un message (requête ou réponse).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
const TOKEN_FILE: &str = "ipc-token";

/// Types de requêtes acceptés de l'extension.
pub const ALLOWED_REQUESTS: [&str; 6] = ["ping", "download", "analyze", "download_media", "status", "show"];

/// Dossier de données de l'application (identique à celui de Tauri).
/// `NDM_DATA_DIR` permet de le remplacer (tests).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("NDM_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    dirs::data_dir().map(|d| d.join(APP_IDENTIFIER))
}

/// Nom du canal (Windows) ou chemin de la socket (Unix).
pub fn endpoint(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        let _ = data_dir;
        let user: String = std::env::var("USERNAME")
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        // Le suffixe dépend aussi du dossier de données, pour isoler les tests.
        let salt = fnv(data_dir_str(data_dir).as_bytes());
        format!(r"\\.\pipe\nova-download-manager-{user}-{salt:08x}")
    }
    #[cfg(not(windows))]
    {
        data_dir.join("ipc.sock").to_string_lossy().into_owned()
    }
}

#[cfg(windows)]
fn data_dir_str(d: &Path) -> String {
    d.to_string_lossy().to_lowercase()
}

#[cfg(windows)]
fn fnv(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5u32, |h, b| (h ^ *b as u32).wrapping_mul(0x0100_0193))
}

// ---------------------------------------------------------------------------
// Jeton
// ---------------------------------------------------------------------------

/// Lit le jeton, ou le crée s'il n'existe pas (côté application).
pub fn ensure_token(data_dir: &Path) -> io::Result<String> {
    if let Ok(t) = read_token(data_dir) {
        return Ok(t);
    }
    std::fs::create_dir_all(data_dir)?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let path = data_dir.join(TOKEN_FILE);
    let tmp = data_dir.join(format!("{TOKEN_FILE}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(token)
}

pub fn read_token(data_dir: &Path) -> io::Result<String> {
    let t = std::fs::read_to_string(data_dir.join(TOKEN_FILE))?;
    let t = t.trim().to_string();
    if t.len() != 64 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "jeton invalide"));
    }
    Ok(t)
}

/// Comparaison en temps constant (évite de deviner le jeton caractère par caractère).
pub fn token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub token: String,
    pub request: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Code machine pour l'extension (`app_not_running`, `invalid_request`…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl Reply {
    pub fn ok(data: Value) -> Self {
        Self { ok: true, data, error: None, code: None }
    }
    pub fn err(code: &str, message: impl Into<String>) -> Self {
        Self { ok: false, data: Value::Null, error: Some(message.into()), code: Some(code.into()) }
    }
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Vérifie la forme d'une requête venant de l'extension.
pub fn validate_request(v: &Value) -> Result<&str, String> {
    let obj = v.as_object().ok_or("la requête doit être un objet JSON")?;
    let ty = obj.get("type").and_then(Value::as_str).ok_or("champ « type » manquant")?;
    if !ALLOWED_REQUESTS.contains(&ty) {
        return Err(format!("type de requête inconnu : {ty}"));
    }
    for key in ["url", "referrer", "filename", "user_agent", "cookies"] {
        if let Some(val) = obj.get(key) {
            let s = val.as_str().ok_or(format!("« {key} » doit être une chaîne"))?;
            if s.len() > 64 * 1024 {
                return Err(format!("« {key} » est trop long"));
            }
            if s.contains(['\r', '\n', '\0']) {
                return Err(format!("« {key} » contient des caractères interdits"));
            }
        }
    }
    if let Some(u) = obj.get("url").and_then(Value::as_str) {
        let lower = u.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            return Err("seules les adresses http et https sont acceptées".into());
        }
    }
    Ok(ty)
}

// ---------------------------------------------------------------------------
// Native Messaging (stdio du navigateur)
// ---------------------------------------------------------------------------

/// Lit un message ; `Ok(None)` en fin de flux (le navigateur a fermé le canal).
pub fn read_native_message(r: &mut impl Read) -> io::Result<Option<Value>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message trop volumineux"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_native_message(w: &mut impl Write, v: &Value) -> io::Result<()> {
    let data = serde_json::to_vec(v).map_err(io::Error::other)?;
    // Limite du navigateur pour les messages de l'hôte vers l'extension : 1 Mo.
    if data.len() > 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "réponse trop volumineuse"));
    }
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(&data)?;
    w.flush()
}

// ---------------------------------------------------------------------------
// Client (hôte natif → application), synchrone
// ---------------------------------------------------------------------------

#[cfg(windows)]
type Stream = std::fs::File;
#[cfg(unix)]
type Stream = std::os::unix::net::UnixStream;

fn connect(endpoint: &str) -> io::Result<Stream> {
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new().read(true).write(true).open(endpoint)
    }
    #[cfg(unix)]
    {
        let s = std::os::unix::net::UnixStream::connect(endpoint)?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
        Ok(s)
    }
}

/// Envoie une requête à l'application et attend la réponse.
/// Une erreur de connexion (`NotFound`, `ConnectionRefused`) signifie que l'application n'est pas lancée.
pub fn request(data_dir: &Path, req: &Value) -> io::Result<Reply> {
    let token = read_token(data_dir)?;
    let stream = connect(&endpoint(data_dir))?;
    let mut writer = stream.try_clone()?;
    let mut line = serde_json::to_vec(&Envelope { token, request: req.clone() }).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()?;
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.by_ref().take(MAX_MESSAGE as u64).read_line(&mut resp)?;
    if resp.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connexion fermée par l'application"));
    }
    serde_json::from_str(&resp).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Vrai si l'erreur indique que l'application n'écoute pas.
pub fn is_not_running(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
        // ERROR_PIPE_BUSY (231) : toutes les instances occupées, on réessaie.
        || e.raw_os_error() == Some(231)
}

// ---------------------------------------------------------------------------
// Serveur (application), asynchrone
// ---------------------------------------------------------------------------

pub mod server {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

    pub type Handler = Arc<dyn Fn(Value) -> Pin<Box<dyn Future<Output = Reply> + Send>> + Send + Sync>;

    /// Écoute les connexions locales jusqu'à l'arrêt du runtime.
    /// Échoue si un autre programme occupe déjà le canal (pas d'usurpation possible).
    pub async fn serve(data_dir: PathBuf, handler: Handler) -> io::Result<()> {
        let token = Arc::new(ensure_token(&data_dir)?);
        let ep = endpoint(&data_dir);
        #[cfg(windows)]
        {
            use tokio::net::windows::named_pipe::ServerOptions;
            let mut server = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&ep)?;
            loop {
                server.connect().await?;
                let client = server;
                server = ServerOptions::new().reject_remote_clients(true).create(&ep)?;
                tokio::spawn(handle(client, token.clone(), handler.clone()));
            }
        }
        #[cfg(unix)]
        {
            // Une ancienne socket peut subsister après un arrêt brutal.
            if std::path::Path::new(&ep).exists() {
                if std::os::unix::net::UnixStream::connect(&ep).is_ok() {
                    return Err(io::Error::new(io::ErrorKind::AddrInUse, "une autre instance écoute déjà"));
                }
                let _ = std::fs::remove_file(&ep);
            }
            let listener = tokio::net::UnixListener::bind(&ep)?;
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&ep, std::fs::Permissions::from_mode(0o600))?;
            }
            loop {
                let (stream, _) = listener.accept().await?;
                tokio::spawn(handle(stream, token.clone(), handler.clone()));
            }
        }
    }

    async fn handle<S: AsyncRead + AsyncWrite + Unpin>(stream: S, token: Arc<String>, handler: Handler) {
        let (r, mut w) = tokio::io::split(stream);
        let mut lines = BufReader::new(r.take(MAX_MESSAGE as u64 * 8));
        loop {
            let mut line = String::new();
            match lines.read_line(&mut line).await {
                Ok(0) | Err(_) => return,
                Ok(n) if n > MAX_MESSAGE => return,
                Ok(_) => {}
            }
            let reply = match serde_json::from_str::<Envelope>(&line) {
                Err(_) => Reply::err("invalid_request", "Message illisible."),
                Ok(env) if !token_eq(&env.token, &token) => Reply::err("unauthorized", "Jeton invalide."),
                Ok(env) => match validate_request(&env.request) {
                    Err(e) => Reply::err("invalid_request", e),
                    Ok(_) => handler(env.request).await,
                },
            };
            let mut out = serde_json::to_vec(&reply).unwrap_or_default();
            out.push(b'\n');
            if w.write_all(&out).await.is_err() || w.flush().await.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn native_framing_roundtrip() {
        let mut buf = Vec::new();
        write_native_message(&mut buf, &json!({"type": "ping"})).unwrap();
        assert_eq!(&buf[..4], &(15u32).to_le_bytes());
        let mut r = &buf[..];
        assert_eq!(read_native_message(&mut r).unwrap(), Some(json!({"type": "ping"})));
        assert_eq!(read_native_message(&mut r).unwrap(), None, "fin de flux");
        // Longueur annoncée trop grande : refusée sans allouer.
        let mut bad = &(u32::MAX).to_le_bytes()[..];
        assert!(read_native_message(&mut bad).is_err());
    }

    #[test]
    fn validates_requests() {
        assert_eq!(validate_request(&json!({"type": "download", "url": "https://a.b/c"})), Ok("download"));
        assert!(validate_request(&json!({"type": "rm -rf"})).is_err());
        assert!(validate_request(&json!({"type": "download", "url": "file:///C:/Windows"})).is_err());
        assert!(validate_request(&json!({"type": "download", "url": "javascript:alert(1)"})).is_err());
        assert!(validate_request(&json!({"type": "download", "url": "https://a\r\nX-Evil: 1"})).is_err());
        assert!(validate_request(&json!({"type": "download", "url": 42})).is_err());
        assert!(validate_request(&json!(["type"])).is_err());
    }

    #[test]
    fn token_is_stable_and_private() {
        let d = tempfile::tempdir().unwrap();
        let t1 = ensure_token(d.path()).unwrap();
        let t2 = ensure_token(d.path()).unwrap();
        assert_eq!(t1, t2);
        assert_eq!(t1.len(), 64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(d.path().join(TOKEN_FILE)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(token_eq(&t1, &t2));
        assert!(!token_eq(&t1, &"0".repeat(64)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_server_with_token() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let handler: server::Handler =
            Arc::new(|req: Value| Box::pin(async move { Reply::ok(json!({"echo": req["type"]})) }));
        tokio::spawn(server::serve(dir.clone(), handler));
        // Attente du démarrage du serveur.
        let mut reply = None;
        for _ in 0..50 {
            let dir = dir.clone();
            match tokio::task::spawn_blocking(move || request(&dir, &json!({"type": "ping"}))).await.unwrap() {
                Ok(r) => {
                    reply = Some(r);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        assert_eq!(reply.unwrap(), Reply::ok(json!({"echo": "ping"})));

        // Requête invalide : rejetée par le serveur même si le client l'envoie.
        let dir2 = dir.clone();
        let r = tokio::task::spawn_blocking(move || request(&dir2, &json!({"type": "format_disk"}))).await.unwrap().unwrap();
        assert_eq!(r.code.as_deref(), Some("invalid_request"));

        // Mauvais jeton : refusé.
        std::fs::write(dir.join(TOKEN_FILE), "f".repeat(64)).unwrap();
        let dir3 = dir.clone();
        let r = tokio::task::spawn_blocking(move || request(&dir3, &json!({"type": "ping"}))).await.unwrap().unwrap();
        assert_eq!(r.code.as_deref(), Some("unauthorized"));
    }

    #[test]
    fn not_running_is_detected() {
        let d = tempfile::tempdir().unwrap();
        ensure_token(d.path()).unwrap();
        let e = request(d.path(), &json!({"type": "ping"})).unwrap_err();
        assert!(is_not_running(&e), "{e:?}");
    }
}

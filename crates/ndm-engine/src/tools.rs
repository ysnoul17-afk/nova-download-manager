//! Installation et mise à jour de yt-dlp et FFmpeg.
//!
//! Les binaires sont téléchargés depuis les dépôts officiels sur GitHub et leur
//! empreinte SHA-256 est comparée au fichier de sommes publié avec chaque version.
//! Un fichier dont l'empreinte ne correspond pas n'est jamais installé.

use crate::error::{EngineError, Result};
use crate::media::{command, MediaTools};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const YTDLP_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/";
const FFMPEG_BASE: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    YtDlp,
    Ffmpeg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub path: PathBuf,
    pub version: Option<String>,
    /// Installé par NDM (sinon trouvé dans le PATH du système).
    pub managed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsStatus {
    pub ytdlp: Option<ToolInfo>,
    pub ffmpeg: Option<ToolInfo>,
    /// L'installation automatique de FFmpeg est possible sur ce système.
    pub ffmpeg_installable: bool,
}

impl ToolsStatus {
    pub fn media_tools(&self) -> MediaTools {
        MediaTools {
            ytdlp: self.ytdlp.as_ref().map(|t| t.path.clone()),
            ffmpeg_dir: self.ffmpeg.as_ref().and_then(|t| t.path.parent().map(Path::to_path_buf)),
        }
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let file = exe(name);
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|d| d.join(&file)).find(|p| p.is_file())
    })
}

async fn version_of(path: &Path, args: &[&str]) -> Option<String> {
    let mut c = command(path);
    c.args(args).stdout(Stdio::piped()).stderr(Stdio::null());
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), c.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?.trim().to_string();
    (!first.is_empty()).then_some(first)
}

/// Détecte les outils : d'abord ceux installés par NDM, sinon ceux du système.
pub async fn detect(tools_dir: &Path) -> ToolsStatus {
    let mut s = ToolsStatus { ffmpeg_installable: cfg!(all(windows, target_arch = "x86_64")), ..Default::default() };

    let managed = tools_dir.join(exe("yt-dlp"));
    let (path, is_managed) = if managed.is_file() { (Some(managed), true) } else { (find_in_path("yt-dlp"), false) };
    if let Some(p) = path {
        let version = version_of(&p, &["--version"]).await;
        if version.is_some() {
            s.ytdlp = Some(ToolInfo { path: p, version, managed: is_managed });
        }
    }

    let managed = tools_dir.join(exe("ffmpeg"));
    let (path, is_managed) = if managed.is_file() { (Some(managed), true) } else { (find_in_path("ffmpeg"), false) };
    if let Some(p) = path {
        // « ffmpeg version N-12345-gabc Copyright… » → on garde l'identifiant de version.
        let version = version_of(&p, &["-hide_banner", "-version"])
            .await
            .map(|v| v.split_whitespace().nth(2).unwrap_or(&v).to_string());
        if version.is_some() {
            s.ffmpeg = Some(ToolInfo { path: p, version, managed: is_managed });
        }
    }
    s
}

/// Progression d'une installation (octets reçus / total).
pub type ProgressFn<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);

/// Télécharge `url` dans `dest` en calculant son SHA-256. Renvoie l'empreinte hexadécimale.
async fn fetch_hashed(client: &reqwest::Client, url: &str, dest: &Path, progress: ProgressFn<'_>) -> Result<String> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(EngineError::HttpStatus { status: resp.status().as_u16(), url: url.into() });
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    let mut stream = resp.bytes_stream();
    let mut last_report = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
        file.write_all(&chunk)?;
        got += chunk.len() as u64;
        if got - last_report > 256 * 1024 {
            progress(got, total);
            last_report = got;
        }
    }
    file.sync_all()?;
    progress(got, total);
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Cherche l'empreinte d'un fichier dans un fichier de sommes (`<sha256>  <nom>`).
pub fn expected_hash(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let name = it.next()?.trim_start_matches('*');
        (name == file_name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let r = client.get(url).send().await?;
    if !r.status().is_success() {
        return Err(EngineError::HttpStatus { status: r.status().as_u16(), url: url.into() });
    }
    Ok(r.text().await?)
}

fn ytdlp_asset() -> Result<&'static str> {
    if cfg!(windows) {
        Ok("yt-dlp.exe")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("yt-dlp_linux")
    } else if cfg!(target_os = "macos") {
        Ok("yt-dlp_macos")
    } else {
        Err(EngineError::Other("Installation automatique de yt-dlp non prise en charge sur ce système.".into()))
    }
}

/// Installe (ou met à jour) yt-dlp dans `tools_dir`.
pub async fn install_ytdlp(client: &reqwest::Client, tools_dir: &Path, progress: ProgressFn<'_>) -> Result<PathBuf> {
    std::fs::create_dir_all(tools_dir)?;
    let asset = ytdlp_asset()?;
    let sums = fetch_text(client, &format!("{YTDLP_BASE}SHA2-256SUMS")).await?;
    let expected = expected_hash(&sums, asset)
        .ok_or_else(|| EngineError::Other("Empreinte de yt-dlp introuvable dans le fichier de sommes officiel.".into()))?;

    let tmp = tools_dir.join(format!("{}.download", exe("yt-dlp")));
    let got = fetch_hashed(client, &format!("{YTDLP_BASE}{asset}"), &tmp, progress).await?;
    if got != expected {
        let _ = std::fs::remove_file(&tmp);
        return Err(EngineError::Other(
            "Le fichier yt-dlp téléchargé ne correspond pas à l'empreinte officielle : installation annulée.".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    let dest = tools_dir.join(exe("yt-dlp"));
    replace_file(&tmp, &dest)?;
    Ok(dest)
}

/// Remplace un fichier, y compris sous Windows où la cible peut exister.
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        // Sous Windows, un exécutable en cours d'utilisation ne peut pas être supprimé.
        let old = to.with_extension("old");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(to, &old).map_err(|_| {
            EngineError::Other("L'outil est en cours d'utilisation. Attendez la fin des téléchargements vidéo.".into())
        })?;
        let _ = std::fs::remove_file(&old);
    }
    std::fs::rename(from, to)?;
    Ok(())
}

/// Installe FFmpeg et FFprobe (Windows 64 bits) dans `tools_dir`.
pub async fn install_ffmpeg(client: &reqwest::Client, tools_dir: &Path, progress: ProgressFn<'_>) -> Result<PathBuf> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err(EngineError::Other(
            "Sur ce système, installez FFmpeg avec le gestionnaire de paquets : NDM le détectera automatiquement.".into(),
        ));
    }
    std::fs::create_dir_all(tools_dir)?;
    let asset = "ffmpeg-master-latest-win64-gpl.zip";
    let sums = fetch_text(client, &format!("{FFMPEG_BASE}checksums.sha256")).await?;
    let expected = expected_hash(&sums, asset)
        .ok_or_else(|| EngineError::Other("Empreinte de FFmpeg introuvable dans le fichier de sommes officiel.".into()))?;

    let archive = tools_dir.join("ffmpeg.zip.download");
    let got = fetch_hashed(client, &format!("{FFMPEG_BASE}{asset}"), &archive, progress).await?;
    if got != expected {
        let _ = std::fs::remove_file(&archive);
        return Err(EngineError::Other(
            "L'archive FFmpeg téléchargée ne correspond pas à l'empreinte officielle : installation annulée.".into(),
        ));
    }
    let dir = tools_dir.to_path_buf();
    let arch = archive.clone();
    let res = tokio::task::spawn_blocking(move || extract_ffmpeg(&arch, &dir))
        .await
        .map_err(|e| EngineError::Other(e.to_string()))?;
    let _ = std::fs::remove_file(&archive);
    res?;
    Ok(tools_dir.join(exe("ffmpeg")))
}

/// Extrait uniquement `bin/ffmpeg(.exe)` et `bin/ffprobe(.exe)` de l'archive.
pub fn extract_ffmpeg(archive: &Path, tools_dir: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| EngineError::Other(format!("Archive FFmpeg illisible : {e}")))?;
    let wanted = [exe("ffmpeg"), exe("ffprobe")];
    let mut found = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| EngineError::Other(e.to_string()))?;
        if !entry.is_file() {
            continue;
        }
        // Le nom de destination est fixé par nous : aucun chemin de l'archive n'est utilisé.
        let name = entry.name().map_err(|e| EngineError::Other(e.to_string()))?.replace('\\', "/");
        let Some(base) = name.rsplit('/').next() else { continue };
        if !name.contains("/bin/") || !wanted.iter().any(|w| w == base) {
            continue;
        }
        let tmp = tools_dir.join(format!("{base}.download"));
        {
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(&mut entry, &mut out)?;
            out.sync_all()?;
        }
        replace_file(&tmp, &tools_dir.join(base))?;
        found += 1;
    }
    if found < wanted.len() {
        return Err(EngineError::Other("L'archive FFmpeg ne contient pas les fichiers attendus.".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checksum_files() {
        let sums = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d  yt-dlp.exe\n\
                    767ba9dce7052aa196f8b12652f433d76de128ba168966fa84b6394d5e6c6795  yt-dlp_linux\n";
        assert_eq!(
            expected_hash(sums, "yt-dlp.exe").as_deref(),
            Some("f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d")
        );
        assert_eq!(expected_hash(sums, "yt-dlp"), None);
        assert_eq!(expected_hash("abc  yt-dlp.exe", "yt-dlp.exe"), None, "empreinte de longueur invalide");
    }

    #[test]
    fn extracts_only_wanted_binaries() {
        let tmp = tempfile::tempdir().unwrap();
        let zpath = tmp.path().join("f.zip");
        {
            let f = std::fs::File::create(&zpath).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in [
                (format!("ffmpeg-x/bin/{}", exe("ffmpeg")), "FF"),
                (format!("ffmpeg-x/bin/{}", exe("ffprobe")), "FP"),
                (format!("ffmpeg-x/bin/{}", exe("ffplay")), "NO"),
                (format!("../../{}", exe("ffmpeg")), "EVIL"),
                ("ffmpeg-x/doc/readme.txt".to_string(), "NO"),
            ] {
                w.start_file(name, opts).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let out = tmp.path().join("tools");
        std::fs::create_dir_all(&out).unwrap();
        extract_ffmpeg(&zpath, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join(exe("ffmpeg"))).unwrap(), "FF");
        assert_eq!(std::fs::read_to_string(out.join(exe("ffprobe"))).unwrap(), "FP");
        assert!(!out.join(exe("ffplay")).exists());
        assert!(!tmp.path().join(exe("ffmpeg")).exists(), "pas d'écriture hors du dossier");
    }
}

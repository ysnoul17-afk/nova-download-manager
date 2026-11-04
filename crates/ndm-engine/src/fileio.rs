//! Écritures positionnelles thread-safe (plusieurs connexions écrivent dans le même fichier).

use std::fs::File;
use std::io;

#[cfg(unix)]
pub fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
pub fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    // `seek_write` utilise une structure OVERLAPPED avec un décalage explicite :
    // sûr en parallèle sur un même handle.
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "écriture nulle"));
            }
            Ok(n) => {
                buf = &buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Contenu du flux NTFS `Zone.Identifier` (« Mark of the Web ») : zone 3 = Internet.
/// Windows s'en sert pour SmartScreen, le mode protégé d'Office (macros bloquées),
/// et l'avertissement à l'ouverture des programmes, comme pour un fichier téléchargé par le navigateur.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn zone_identifier(url: &str, referrer: Option<&str>) -> String {
    // Identifiants éventuellement présents dans l'adresse (https://user:pass@…) : jamais recopiés.
    let clean = |u: &str| -> Option<String> {
        let mut p = url::Url::parse(u).ok()?;
        if !matches!(p.scheme(), "http" | "https") {
            return None;
        }
        let _ = p.set_username("");
        let _ = p.set_password(None);
        Some(p.to_string())
    };
    let mut s = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
    if let Some(r) = referrer.and_then(clean) {
        s.push_str(&format!("ReferrerUrl={r}\r\n"));
    }
    if let Some(h) = clean(url) {
        s.push_str(&format!("HostUrl={h}\r\n"));
    }
    s
}

/// Marque le fichier comme provenant d'Internet (Windows ; sans effet ailleurs).
/// Un échec (disque FAT/exFAT sans flux alternatifs) n'empêche pas le téléchargement.
pub fn mark_from_internet(path: &std::path::Path, url: &str, referrer: Option<&str>) {
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        if let Err(e) = std::fs::write(std::path::PathBuf::from(ads), zone_identifier(url, referrer)) {
            tracing::warn!("marque de provenance Internet impossible pour {} : {e}", path.display());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (path, url, referrer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_identifier_content() {
        let z = zone_identifier("https://user:pw@cdn.ex.fr/setup.exe?x=1", Some("https://ex.fr/page"));
        assert_eq!(z, "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://ex.fr/page\r\nHostUrl=https://cdn.ex.fr/setup.exe?x=1\r\n");
        assert!(!z.contains("pw"));
        assert_eq!(zone_identifier("pas une url", None), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
}

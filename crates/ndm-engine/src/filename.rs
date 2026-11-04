//! Déduction, nettoyage et classement des noms de fichiers.

use crate::types::Category;
use percent_encoding::percent_decode_str;
use std::path::{Path, PathBuf};

/// Extrait le nom depuis un en-tête `Content-Disposition`.
/// Gère `filename*=UTF-8''...` (RFC 5987) en priorité, puis `filename="..."`.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain: Option<String> = None;
    for part in split_params(value) {
        let part = part.trim();
        let Some((key, val)) = part.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let val = val.trim();
        if key == "filename*" {
            // charset'lang'valeur-encodée
            let mut it = val.splitn(3, '\'');
            let charset = it.next().unwrap_or("").to_ascii_lowercase();
            let _lang = it.next();
            if let Some(enc) = it.next() {
                let bytes: Vec<u8> = percent_decode_str(enc.trim_matches('"')).collect();
                let s = if charset == "utf-8" || charset.is_empty() {
                    String::from_utf8_lossy(&bytes).into_owned()
                } else {
                    // ISO-8859-1 : chaque octet est un point de code.
                    bytes.iter().map(|&b| b as char).collect()
                };
                if !s.trim().is_empty() {
                    return Some(s);
                }
            }
        } else if key == "filename" && plain.is_none() {
            let unq = unquote(val);
            // Certains serveurs percent-encodent aussi ce champ.
            let decoded = percent_decode_str(&unq).decode_utf8_lossy().into_owned();
            if !decoded.trim().is_empty() {
                plain = Some(decoded);
            }
        }
    }
    plain
}

/// Découpe sur `;` en respectant les guillemets.
fn split_params(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
            continue;
        }
        match c {
            '\\' if in_q => {
                cur.push(c);
                esc = true;
            }
            '"' => {
                in_q = !in_q;
                cur.push(c);
            }
            ';' if !in_q => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut esc = false;
        for c in inner.chars() {
            if esc {
                out.push(c);
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else {
                out.push(c);
            }
        }
        out
    } else {
        s.to_string()
    }
}

/// Dernier segment non vide du chemin de l'URL, décodé.
pub fn from_url(url: &url::Url) -> Option<String> {
    let seg = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let decoded = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    (!decoded.trim().is_empty()).then_some(decoded)
}

/// Extension par défaut selon le type MIME, si le nom n'en a pas.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let m = mime.split(';').next()?.trim().to_ascii_lowercase();
    Some(match m.as_str() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "video/mp2t" => "ts",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/flac" => "flac",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/gzip" | "application/x-gzip" => "gz",
        "application/x-msdownload" | "application/x-msdos-program" => "exe",
        "application/x-msi" => "msi",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/html" => "html",
        _ => return None,
    })
}

/// Rend un nom utilisable sous Windows : caractères interdits, noms réservés,
/// points/espaces finaux, longueur.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        // Caractères de contrôle de direction et invisibles : ils permettent d'afficher
        // « facture[RLO]fdp.exe » comme « factureexe.pdf ». Supprimés.
        .filter(|c| !is_invisible_format(*c))
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 32 => '_',
            c => c,
        })
        .collect();
    // Pas de point ni d'espace en fin de nom sous Windows.
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    let s_trim = s.trim_start().to_string();
    let mut s = if s_trim.is_empty() { "telechargement".to_string() } else { s_trim };

    let stem_upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem_upper.as_str()) {
        s.insert(0, '_');
    }

    // Limite à 200 octets en préservant l'extension.
    const MAX: usize = 200;
    if s.len() > MAX {
        let (stem, ext) = split_ext(&s);
        let ext = ext.map(|e| format!(".{e}")).unwrap_or_default();
        let mut budget = MAX.saturating_sub(ext.len());
        while !stem.is_char_boundary(budget) {
            budget -= 1;
        }
        s = format!("{}{}", &stem[..budget], ext);
    }
    s
}

fn is_invisible_format(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'   // espaces de largeur nulle, marques LRM/RLM
        | '\u{202A}'..='\u{202E}' // incorporations et forçages de direction (dont RLO)
        | '\u{2060}'..='\u{2064}' // jointures invisibles
        | '\u{2066}'..='\u{2069}' // isolats de direction
        | '\u{061C}'               // marque de lettre arabe
        | '\u{FEFF}'               // indicateur d'ordre des octets
    )
}

pub fn split_ext(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

/// `nom.ext` → `nom (1).ext`, `nom (2).ext`... jusqu'à trouver un nom libre
/// (ni fichier final, ni fichier partiel en cours).
pub fn unique_path(dir: &Path, name: &str, is_taken: impl Fn(&Path) -> bool) -> PathBuf {
    let taken = |p: &Path| p.exists() || crate::download::part_path(p).exists() || is_taken(p);
    let first = dir.join(name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = split_ext(name);
    for i in 1..10_000 {
        let candidate = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let p = dir.join(candidate);
        if !taken(&p) {
            return p;
        }
    }
    dir.join(format!("{}-{}", uuid::Uuid::new_v4(), name))
}

/// Types de fichiers qui exécutent du code (ou en contiennent) quand on les ouvre sous Windows.
const EXECUTABLE_EXTS: &[&str] = &[
    "exe", "msi", "msix", "msixbundle", "appx", "appxbundle", "appinstaller", "application", "appref-ms",
    "bat", "cmd", "com", "scr", "pif", "cpl", "msc", "msp", "gadget", "inf", "reg", "scf", "lnk", "url",
    "js", "jse", "vbs", "vbe", "wsf", "wsh", "hta", "ps1", "psm1", "psd1", "jar", "dll", "sys",
    // Images disque : leur contenu échappait à la protection « provenance Internet » de Windows.
    "iso", "img", "vhd", "vhdx",
];

/// Vrai si ouvrir ce fichier peut exécuter un programme.
pub fn is_executable(name: &str) -> bool {
    split_ext(name).1.map(|e| EXECUTABLE_EXTS.contains(&e.to_ascii_lowercase().as_str())).unwrap_or(false)
}

pub fn categorize(name: &str) -> Category {
    let ext = split_ext(name).1.map(|e| e.to_ascii_lowercase());
    let Some(ext) = ext else { return Category::Other };
    match ext.as_str() {
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpg" | "mpeg"
        | "3gp" | "ts" | "m2ts" | "ogv" => Category::Video,
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "opus" | "wma" | "aiff" | "alac" => {
            Category::Music
        }
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
        | "txt" | "rtf" | "csv" | "epub" | "md" => Category::Document,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "heic"
        | "avif" | "ico" | "raw" => Category::Image,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" | "cab" => {
            Category::Archive
        }
        "exe" | "msi" | "msix" | "appx" | "iso" | "img" | "dmg" | "apk" | "deb" | "rpm"
        | "bat" | "cmd" | "ps1" => Category::Software,
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_variants() {
        assert_eq!(
            from_content_disposition(r#"attachment; filename="rapport final.pdf""#).as_deref(),
            Some("rapport final.pdf")
        );
        assert_eq!(
            from_content_disposition(
                r#"attachment; filename="fallback.pdf"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf"#
            )
            .as_deref(),
            Some("résumé.pdf")
        );
        assert_eq!(
            from_content_disposition(r#"inline; filename="a;b.txt""#).as_deref(),
            Some("a;b.txt")
        );
        assert_eq!(from_content_disposition("attachment").as_deref(), None);
    }

    #[test]
    fn sanitize_windows() {
        assert_eq!(sanitize("a<b>c:d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize("CON.txt"), "_CON.txt");
        assert_eq!(sanitize("fichier. . "), "fichier");
        assert_eq!(sanitize(""), "telechargement");
        // « facture[RLO]fdp.exe » s'afficherait « factureexe.pdf » : l'inversion est supprimée.
        assert_eq!(sanitize("facture\u{202E}fdp.exe"), "facturefdp.exe");
        assert_eq!(categorize(&sanitize("facture\u{202E}fdp.exe")), Category::Software);
        assert_eq!(sanitize("a\u{200B}b\u{2066}.zip"), "ab.zip");
        let long = format!("{}.mp4", "é".repeat(300));
        let s = sanitize(&long);
        assert!(s.len() <= 200 && s.ends_with(".mp4"));
    }

    #[test]
    fn executables() {
        for n in ["setup.exe", "facture.pdf.js", "x.VBS", "a.hta", "raccourci.lnk", "image.iso", "s.scr"] {
            assert!(is_executable(n), "{n}");
        }
        for n in ["film.mp4", "doc.pdf", "archive.zip", "sans_extension"] {
            assert!(!is_executable(n), "{n}");
        }
    }

    #[test]
    fn categories() {
        assert_eq!(categorize("film.MKV"), Category::Video);
        assert_eq!(categorize("setup.exe"), Category::Software);
        assert_eq!(categorize("photo.jpeg"), Category::Image);
        assert_eq!(categorize("sans_extension"), Category::Other);
    }

    #[test]
    fn url_names() {
        let u = url::Url::parse("https://ex.com/dir/mon%20fichier.zip?x=1").unwrap();
        assert_eq!(from_url(&u).as_deref(), Some("mon fichier.zip"));
        let u = url::Url::parse("https://ex.com/").unwrap();
        assert_eq!(from_url(&u), None);
    }
}

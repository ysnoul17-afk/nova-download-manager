//! Erreurs du moteur, avec des messages compréhensibles par l'utilisateur.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Adresse invalide : {0}")]
    InvalidUrl(String),
    #[error("{}", http_message(*status))]
    HttpStatus { status: u16, url: String },
    #[error("{}", network_message(.0))]
    Network(#[from] reqwest::Error),
    #[error("{}", io_message(.0))]
    Io(#[from] std::io::Error),
    #[error("Le fichier a été modifié sur le serveur depuis le début du téléchargement.")]
    RemoteChanged,
    #[error("Le serveur a renvoyé une portion de fichier inattendue.")]
    BadRange,
    #[error("La connexion ne reçoit plus de données (délai dépassé).")]
    Stalled,
    #[error("La connexion s'est fermée avant la fin du transfert.")]
    Truncated,
    #[error("Fichier incomplet : {actual} octets reçus sur {expected} attendus.")]
    Integrity { expected: u64, actual: u64 },
    #[error("Téléchargement introuvable.")]
    NotFound,
    #[error("Opération annulée.")]
    Cancelled,
    #[error("Base de données : {0}")]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

impl EngineError {
    /// Erreurs passagères qui justifient une nouvelle tentative.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Stalled | Self::Truncated => true,
            Self::HttpStatus { status, .. } => {
                matches!(*status, 408 | 425 | 429 | 500 | 502 | 503 | 504)
            }
            _ => false,
        }
    }
}

fn http_message(status: u16) -> String {
    match status {
        401 => "Authentification requise : le serveur refuse l'accès (401).".into(),
        403 => "Accès refusé par le serveur (403). Le lien a peut-être expiré ou nécessite d'être connecté.".into(),
        404 => "Fichier introuvable sur le serveur (404).".into(),
        410 => "Ce lien a expiré ou le fichier a été supprimé (410).".into(),
        416 => "Le serveur refuse la reprise à cette position (416).".into(),
        429 => "Le serveur limite le nombre de requêtes (429). Réessayez plus tard.".into(),
        500..=599 => format!("Le serveur rencontre une erreur ({status}). Réessayez plus tard."),
        _ => format!("Le serveur a répondu avec le code {status}."),
    }
}

fn source_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(inner) = cur {
        s.push_str(" | ");
        s.push_str(&inner.to_string());
        cur = inner.source();
    }
    s.to_ascii_lowercase()
}

fn is_dns_error(e: &reqwest::Error) -> bool {
    let chain = source_chain(e);
    chain.contains("dns error")
        || chain.contains("failed to lookup address")
        || chain.contains("no such host")
        || chain.contains("name or service not known")
        || chain.contains("nodename nor servname")
}

fn network_message(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "Délai d'attente dépassé : le serveur ne répond pas.".into();
    }
    if is_dns_error(e) {
        return "Adresse du serveur introuvable (erreur DNS). Vérifiez l'URL et votre connexion.".into();
    }
    let chain = source_chain(e);
    if chain.contains("certificate") || chain.contains("tls") || chain.contains("ssl") {
        return "Connexion sécurisée impossible : certificat du serveur invalide ou non reconnu.".into();
    }
    if e.is_connect() {
        return "Impossible de se connecter au serveur. Vérifiez votre connexion Internet.".into();
    }
    if e.is_redirect() {
        return "Trop de redirections : le lien semble mal configuré.".into();
    }
    if e.is_body() || e.is_decode() || chain.contains("connection reset") || chain.contains("closed") {
        return "La connexion a été interrompue pendant le transfert.".into();
    }
    format!("Erreur réseau : {e}")
}

fn io_message(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    // Codes Windows : ERROR_HANDLE_DISK_FULL (39), ERROR_DISK_FULL (112), ENOSPC (28) sous Unix.
    let disk_full = matches!(e.raw_os_error(), Some(39) | Some(112))
        || (cfg!(unix) && e.raw_os_error() == Some(28))
        || e.kind() == StorageFull
        || e.kind() == QuotaExceeded;
    if disk_full {
        return "Espace disque insuffisant dans le dossier de destination.".into();
    }
    match e.kind() {
        PermissionDenied => "Permission refusée : impossible d'écrire dans le dossier de destination.".into(),
        ReadOnlyFilesystem => "Le dossier de destination est en lecture seule.".into(),
        NotFound => "Dossier de destination introuvable.".into(),
        InvalidFilename => "Nom de fichier ou chemin invalide.".into(),
        _ => match e.raw_os_error() {
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            Some(32) | Some(33) if cfg!(windows) => {
                "Le fichier est utilisé par un autre programme.".into()
            }
            _ => format!("Erreur disque : {e}"),
        },
    }
}

pub type Result<T, E = EngineError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_messages() {
        let e = EngineError::HttpStatus { status: 404, url: "x".into() };
        assert!(e.to_string().contains("introuvable"));
        let e = EngineError::Io(std::io::Error::from(std::io::ErrorKind::StorageFull));
        assert!(e.to_string().contains("Espace disque"));
        let e = EngineError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(e.to_string().contains("Permission"));
    }
}

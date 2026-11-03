//! # ndm-engine
//!
//! Moteur de téléchargement de Nova Download Manager.
//!
//! - Téléchargement multisegment avec segmentation dynamique (vol de travail).
//! - Reprise après interruption, avec vérification ETag / Last-Modified (If-Range).
//! - File d'attente avec priorités et nombre de téléchargements simultanés.
//! - Limitation de vitesse globale et par fichier.
//! - Persistance SQLite (historique, recherche, réglages), reprise automatique au redémarrage.
//! - Classement automatique par catégorie.

pub mod download;
pub mod error;
mod fileio;
pub mod filename;
pub mod limiter;
pub mod manager;
pub mod store;
pub mod types;

pub use error::{EngineError, Result};
pub use manager::Engine;
pub use store::{HistoryQuery, SortKey, Store};
pub use types::*;

//! Types publics partagés entre le moteur, l'application et l'hôte natif.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

pub type DownloadId = Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    /// En file d'attente, démarrera dès qu'un créneau est libre.
    Queued,
    /// Analyse de l'URL (taille, nom, support des plages).
    Probing,
    Downloading,
    Paused,
    /// Assemblage / renommage final.
    Finalizing,
    Completed,
    Failed,
    /// Arrêté par l'utilisateur (le fichier partiel est conservé).
    Stopped,
}

impl DownloadStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Probing | Self::Downloading | Self::Finalizing)
    }
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low = 0,
    #[default]
    Normal = 1,
    High = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Video,
    Music,
    Document,
    Image,
    Archive,
    Software,
    Other,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Self::Video,
        Self::Music,
        Self::Document,
        Self::Image,
        Self::Archive,
        Self::Software,
        Self::Other,
    ];

    /// Nom du sous-dossier par défaut (en français, l'interface étant française).
    pub fn default_folder_name(self) -> &'static str {
        match self {
            Self::Video => "Vidéos",
            Self::Music => "Musique",
            Self::Document => "Documents",
            Self::Image => "Images",
            Self::Archive => "Archives",
            Self::Software => "Logiciels",
            Self::Other => "Autres",
        }
    }
}

/// Un segment d'octets `[start, end]` (bornes incluses) et sa progression.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Segment {
    pub start: u64,
    /// Borne incluse. `None` = taille inconnue (téléchargement mono-flux).
    pub end: Option<u64>,
    /// Octets déjà écrits à partir de `start`.
    pub downloaded: u64,
    /// Segment actuellement pris en charge par une connexion (non persisté).
    #[serde(skip, default)]
    pub active: bool,
}

impl Segment {
    pub fn len(&self) -> Option<u64> {
        self.end.map(|e| e + 1 - self.start)
    }
    pub fn remaining(&self) -> Option<u64> {
        self.len().map(|l| l.saturating_sub(self.downloaded))
    }
    pub fn is_done(&self) -> bool {
        matches!(self.remaining(), Some(0))
    }
    /// Prochain octet à télécharger.
    pub fn cursor(&self) -> u64 {
        self.start + self.downloaded
    }
}

/// Demande d'ajout d'un téléchargement (depuis l'UI ou l'extension).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AddRequest {
    pub url: String,
    /// Nom de fichier imposé (sinon déduit de la réponse serveur / de l'URL).
    pub filename: Option<String>,
    /// Dossier imposé (sinon dossier de la catégorie).
    pub directory: Option<PathBuf>,
    /// En-têtes à rejouer (Referer, User-Agent, Cookie...) transmis par le navigateur.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub priority: Priority,
    /// Ajouter sans démarrer.
    #[serde(default)]
    pub start_paused: bool,
    /// Nombre de connexions max pour ce fichier (sinon valeur globale).
    pub connections: Option<u8>,
    /// Taille connue à l'avance (ex. fournie par le navigateur), purement indicative.
    pub size_hint: Option<u64>,
    /// Téléchargement vidéo via yt-dlp (sinon téléchargement HTTP direct).
    #[serde(default)]
    pub media: Option<crate::media::MediaSpec>,
}

/// Instantané d'un téléchargement, sérialisable pour l'UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub id: DownloadId,
    pub url: String,
    /// URL finale après redirections.
    pub final_url: Option<String>,
    pub filename: String,
    pub directory: PathBuf,
    pub category: Category,
    pub status: DownloadStatus,
    pub priority: Priority,
    pub total_size: Option<u64>,
    pub downloaded: u64,
    /// Octets/s (moyenne glissante, pendant le téléchargement).
    pub speed: u64,
    /// Vitesse moyenne constatée une fois terminé.
    pub avg_speed: Option<u64>,
    /// Secondes restantes estimées.
    pub eta_secs: Option<u64>,
    pub active_connections: u32,
    pub max_connections: u8,
    pub resumable: bool,
    /// Limite propre au fichier (octets/s, 0 = aucune).
    pub speed_limit: u64,
    pub error: Option<String>,
    /// Horodatages Unix en millisecondes.
    pub added_at: u64,
    pub completed_at: Option<u64>,
    /// Ordre d'ajout (strictement croissant).
    pub seq: u64,
    /// Qualité choisie pour une vidéo (« 1080p MP4 »), sinon absent.
    pub media_label: Option<String>,
    /// Étape en cours pour une vidéo (« Piste audio », « Fusion audio et vidéo »…).
    pub stage: Option<String>,
}

impl DownloadInfo {
    pub fn full_path(&self) -> PathBuf {
        self.directory.join(&self.filename)
    }
    pub fn progress(&self) -> Option<f64> {
        self.total_size
            .filter(|t| *t > 0)
            .map(|t| self.downloaded as f64 / t as f64)
    }
}

/// Événements diffusés à l'UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EngineEvent {
    Added { info: DownloadInfo },
    /// Mise à jour périodique (~2 fois/s) des téléchargements dont l'état a changé.
    Progress { items: Vec<DownloadInfo> },
    StatusChanged { info: DownloadInfo },
    Removed { id: DownloadId },
    /// Statistiques globales.
    Stats { total_speed: u64, active: u32, queued: u32 },
}

/// Configuration du moteur, modifiable à chaud.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// Dossier racine (par défaut `%USERPROFILE%\Downloads\NDM`).
    pub base_directory: PathBuf,
    /// Dossier par catégorie (sinon `base_directory/<nom par défaut>`).
    pub category_directories: BTreeMap<Category, PathBuf>,
    pub max_concurrent_downloads: usize,
    /// Connexions par fichier (IDM utilise 8 par défaut).
    pub connections_per_download: u8,
    /// Taille minimale d'un segment avant de le scinder (octets).
    pub min_segment_size: u64,
    /// Limite globale en octets/s (0 = illimité).
    pub global_speed_limit: u64,
    pub max_retries: u32,
    pub user_agent: String,
    pub connect_timeout_secs: u64,
    /// Délai sans recevoir d'octet avant de considérer une connexion bloquée.
    pub read_timeout_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            base_directory: default_download_dir(),
            category_directories: BTreeMap::new(),
            max_concurrent_downloads: 4,
            connections_per_download: 8,
            min_segment_size: 1024 * 1024,
            global_speed_limit: 0,
            max_retries: 8,
            user_agent: format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) NovaDownloadManager/{}",
                env!("CARGO_PKG_VERSION")
            ),
            connect_timeout_secs: 15,
            read_timeout_secs: 30,
        }
    }
}

impl EngineConfig {
    /// Ramène les valeurs dans des bornes sûres.
    pub fn normalized(mut self) -> Self {
        self.max_concurrent_downloads = self.max_concurrent_downloads.clamp(1, 16);
        self.connections_per_download = self.connections_per_download.clamp(1, 32);
        self.min_segment_size = self.min_segment_size.clamp(64 * 1024, 64 * 1024 * 1024);
        self.max_retries = self.max_retries.min(50);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(3, 120);
        self.read_timeout_secs = self.read_timeout_secs.clamp(5, 300);
        if self.user_agent.trim().is_empty() {
            self.user_agent = Self::default().user_agent;
        }
        self.category_directories.retain(|_, p| p.is_absolute());
        self
    }

    pub fn directory_for(&self, cat: Category) -> PathBuf {
        self.category_directories
            .get(&cat)
            .cloned()
            .unwrap_or_else(|| self.base_directory.join(cat.default_folder_name()))
    }
}

fn default_download_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Downloads").join("NDM")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

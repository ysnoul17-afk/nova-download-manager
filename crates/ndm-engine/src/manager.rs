//! Gestionnaire : file d'attente, priorités, persistance SQLite et diffusion d'événements.

use crate::download::{self, DownloadRecord, DownloadShared, EngineCtx, Validator};
use crate::error::{EngineError, Result};
use crate::filename;
use crate::store::{HistoryQuery, Store};
use crate::types::*;
use parking_lot::{Mutex, RwLock};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const TICK: Duration = Duration::from_millis(500);
const SAVE_EVERY: Duration = Duration::from_secs(2);
const CONFIG_KEY: &str = "engine_config";

struct Runtime {
    cancel: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    speed: f64,
    last_received: u64,
    last_sample: Instant,
    run_started: Option<Instant>,
}

struct Entry {
    shared: Arc<DownloadShared>,
    rt: Mutex<Runtime>,
}

impl Entry {
    fn new(record: DownloadRecord) -> Self {
        Self {
            shared: Arc::new(DownloadShared::new(record)),
            rt: Mutex::new(Runtime {
                cancel: None,
                task: None,
                speed: 0.0,
                last_received: 0,
                last_sample: Instant::now(),
                run_started: None,
            }),
        }
    }

    fn id(&self) -> DownloadId {
        self.shared.record.lock().id
    }

    fn is_running(&self) -> bool {
        self.rt.lock().task.as_ref().map(|t| !t.is_finished()).unwrap_or(false)
    }

    fn info(&self, default_conns: u8) -> DownloadInfo {
        let r = self.shared.record.lock();
        let rt = self.rt.lock();
        let downloaded = r.downloaded();
        let speed = if r.status == DownloadStatus::Downloading { rt.speed.round() as u64 } else { 0 };
        let eta_secs = match (r.total_size, speed) {
            (Some(t), s) if s > 0 => Some(t.saturating_sub(downloaded) / s),
            _ => None,
        };
        DownloadInfo {
            id: r.id,
            url: r.url.clone(),
            final_url: r.final_url.clone(),
            filename: r.filename.clone(),
            directory: r.directory.clone(),
            category: r.category,
            status: r.status,
            priority: r.priority,
            total_size: r.total_size.or(r.size_hint),
            downloaded,
            speed,
            avg_speed: r.avg_speed,
            eta_secs,
            active_connections: self.shared.active_connections.load(Ordering::Relaxed),
            max_connections: r.max_connections.unwrap_or(default_conns),
            resumable: r.resumable,
            speed_limit: r.speed_limit,
            error: r.error.clone(),
            added_at: r.added_at,
            completed_at: r.completed_at,
            seq: r.seq,
            media_label: r.media.as_ref().map(|m| m.label.clone()),
            stage: r.stage.map(|s| s.label().to_string()),
        }
    }
}

struct Inner {
    ctx: Arc<EngineCtx>,
    entries: RwLock<BTreeMap<DownloadId, Arc<Entry>>>,
    events: broadcast::Sender<EngineEvent>,
    store: Arc<Store>,
    seq: AtomicU64,
    wake: Notify,
    dirty: Mutex<HashSet<DownloadId>>,
    save_lock: tokio::sync::Mutex<()>,
    shutdown: CancellationToken,
}

/// Point d'entrée du moteur. Clonable à faible coût.
#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    /// Démarre le moteur avec sa base `ndm.sqlite3` dans `data_dir`.
    /// La configuration enregistrée remplace `defaults` si elle existe.
    /// Doit être appelé dans un runtime Tokio.
    pub fn start(data_dir: impl AsRef<Path>, defaults: EngineConfig) -> Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)?;
        let store = Arc::new(Store::open(&data_dir.join("ndm.sqlite3"))?);
        Self::start_with_store(store, defaults)
    }

    pub fn start_with_store(store: Arc<Store>, defaults: EngineConfig) -> Result<Self> {
        let config = store
            .get_setting(CONFIG_KEY)?
            .and_then(|s| serde_json::from_str::<EngineConfig>(&s).ok())
            .unwrap_or(defaults)
            .normalized();

        let mut map = BTreeMap::new();
        let mut max_seq = 0;
        for mut r in store.load_all()? {
            // Les téléchargements interrompus par la fermeture reprennent automatiquement.
            if r.status.is_active() {
                r.status = DownloadStatus::Queued;
            }
            for s in &mut r.segments {
                s.active = false;
            }
            max_seq = max_seq.max(r.seq);
            map.insert(r.id, Arc::new(Entry::new(r)));
        }

        let (events, _) = broadcast::channel(1024);
        let inner = Arc::new(Inner {
            ctx: Arc::new(EngineCtx::new(config)?),
            entries: RwLock::new(map),
            events,
            store,
            seq: AtomicU64::new(max_seq + 1),
            wake: Notify::new(),
            dirty: Mutex::new(HashSet::new()),
            save_lock: tokio::sync::Mutex::new(()),
            shutdown: CancellationToken::new(),
        });
        tokio::spawn(background(Arc::downgrade(&inner)));
        inner.wake.notify_one();
        Ok(Self { inner })
    }

    /// Indique au moteur où trouver yt-dlp et FFmpeg.
    pub fn set_media_tools(&self, tools: crate::media::MediaTools) {
        *self.inner.ctx.media_tools.write() = tools;
    }

    pub fn media_tools(&self) -> crate::media::MediaTools {
        self.inner.ctx.media_tools.read().clone()
    }

    /// Client HTTP du moteur (réutilisé pour l'installation des outils).
    pub fn http_client(&self) -> reqwest::Client {
        self.inner.ctx.client.clone()
    }

    /// Analyse rapide d'une adresse : fichier téléchargeable ou page web ?
    pub async fn inspect(&self, url: &str) -> Result<download::ProbeResult> {
        download::probe(&self.inner.ctx, url, &Default::default()).await
    }

    /// Comme `inspect`, avec les en-têtes du navigateur (Referer, User-Agent, Cookie…).
    pub async fn inspect_with(&self, url: &str, headers: &BTreeMap<String, String>) -> Result<download::ProbeResult> {
        download::probe(&self.inner.ctx, url, headers).await
    }

    /// Analyse une page vidéo avec yt-dlp (qualités disponibles, titre, durée…).
    pub async fn analyze_media(&self, url: &str) -> Result<crate::media::MediaInfo> {
        let tools = self.media_tools();
        crate::media::analyze(&tools, url, &CancellationToken::new()).await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.inner.store
    }

    pub fn config(&self) -> EngineConfig {
        self.inner.ctx.config.read().clone()
    }

    /// Applique et enregistre une nouvelle configuration.
    pub fn set_config(&self, cfg: EngineConfig) -> Result<EngineConfig> {
        let cfg = cfg.normalized();
        self.inner.ctx.global_limiter.set_rate(cfg.global_speed_limit);
        *self.inner.ctx.config.write() = cfg.clone();
        let json = serde_json::to_string(&cfg).map_err(|e| EngineError::Other(e.to_string()))?;
        self.inner.store.set_setting(CONFIG_KEY, &json)?;
        self.inner.wake.notify_one();
        Ok(cfg)
    }

    pub fn set_global_speed_limit(&self, bytes_per_sec: u64) -> Result<()> {
        let mut cfg = self.config();
        cfg.global_speed_limit = bytes_per_sec;
        self.set_config(cfg).map(|_| ())
    }

    /// Tous les téléchargements, du plus récent au plus ancien.
    pub fn list(&self) -> Vec<DownloadInfo> {
        let conns = self.inner.ctx.config.read().connections_per_download;
        let mut v: Vec<_> = self.inner.entries.read().values().map(|e| e.info(conns)).collect();
        v.sort_by_key(|i| std::cmp::Reverse(i.seq));
        v
    }

    /// Carte des segments (pour l'affichage de la progression par connexion).
    pub fn segments(&self, id: DownloadId) -> Option<Vec<Segment>> {
        self.inner.entries.read().get(&id).map(|e| e.shared.record.lock().segments.clone())
    }

    /// Téléchargement déjà terminé pour la même adresse (et la même qualité vidéo),
    /// dont le fichier existe encore sur le disque. Le plus récent en premier.
    pub fn find_completed(&self, url: &str, media_label: Option<&str>) -> Option<DownloadInfo> {
        let norm = url::Url::parse(url.trim()).map(|u| u.to_string()).unwrap_or_else(|_| url.trim().to_string());
        self.list().into_iter().find(|i| {
            i.status == DownloadStatus::Completed
                && (i.url == norm || i.final_url.as_deref() == Some(norm.as_str()))
                && i.media_label.as_deref() == media_label
                && i.full_path().is_file()
        })
    }

    pub fn get(&self, id: DownloadId) -> Option<DownloadInfo> {
        let conns = self.inner.ctx.config.read().connections_per_download;
        self.inner.entries.read().get(&id).map(|e| e.info(conns))
    }

    /// Recherche dans l'historique (SQLite). Les données en cours sont d'abord enregistrées.
    pub async fn query(&self, q: HistoryQuery) -> Result<Vec<DownloadInfo>> {
        self.inner.save().await;
        let store = self.inner.store.clone();
        let ids = tokio::task::spawn_blocking(move || store.query(&q))
            .await
            .map_err(|e| EngineError::Other(e.to_string()))??;
        let conns = self.inner.ctx.config.read().connections_per_download;
        let entries = self.inner.entries.read();
        Ok(ids.iter().filter_map(|id| entries.get(id)).map(|e| e.info(conns)).collect())
    }

    pub fn add(&self, req: AddRequest) -> Result<DownloadInfo> {
        let url = url::Url::parse(req.url.trim()).map_err(|_| EngineError::InvalidUrl(req.url.clone()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(EngineError::InvalidUrl(format!("protocole non pris en charge ({})", url.scheme())));
        }
        if url.host_str().unwrap_or("").is_empty() {
            return Err(EngineError::InvalidUrl(req.url.clone()));
        }
        // Pour une vidéo, le nom est fixé plus tard à partir du titre.
        let filename = if req.media.is_some() {
            String::new()
        } else {
            req.filename.as_deref().map(filename::sanitize).unwrap_or_default()
        };
        let category = match &req.media {
            Some(m) if m.format.is_audio() => Category::Music,
            Some(_) => Category::Video,
            None if filename.is_empty() => Category::Other,
            None => filename::categorize(&filename),
        };
        let directory = match &req.directory {
            Some(d) => validate_directory(d)?,
            None => self.inner.ctx.config.read().directory_for(category),
        };

        let record = DownloadRecord {
            id: uuid::Uuid::new_v4(),
            url: url.to_string(),
            final_url: None,
            headers: req.headers,
            filename_locked: !filename.is_empty(),
            filename,
            directory_locked: req.directory.is_some(),
            directory,
            category,
            status: if req.start_paused { DownloadStatus::Paused } else { DownloadStatus::Queued },
            priority: req.priority,
            total_size: None,
            size_hint: req.size_hint,
            resumable: false,
            segments: vec![],
            validator: Validator::default(),
            mime: None,
            max_connections: req.connections.map(|c| c.clamp(1, 32)),
            speed_limit: 0,
            error: None,
            added_at: now_ms(),
            completed_at: None,
            seq: self.inner.seq.fetch_add(1, Ordering::Relaxed),
            active_ms: 0,
            avg_speed: None,
            media: req.media,
            stage: None,
        };
        let id = record.id;
        let entry = Arc::new(Entry::new(record));
        self.inner.entries.write().insert(id, entry.clone());
        let info = entry.info(self.config().connections_per_download);
        let _ = self.inner.events.send(EngineEvent::Added { info: info.clone() });
        self.inner.mark_dirty(id);
        self.inner.wake.notify_one();
        Ok(info)
    }

    pub fn pause(&self, id: DownloadId) -> Result<()> {
        self.inner.halt(id, DownloadStatus::Paused)
    }

    /// Arrête le téléchargement ; le fichier partiel est conservé pour une reprise ultérieure.
    pub fn stop(&self, id: DownloadId) -> Result<()> {
        self.inner.halt(id, DownloadStatus::Stopped)
    }

    pub fn resume(&self, id: DownloadId) -> Result<()> {
        let e = self.inner.entry(id)?;
        {
            let mut r = e.shared.record.lock();
            if !matches!(r.status, DownloadStatus::Paused | DownloadStatus::Stopped | DownloadStatus::Failed) {
                return Ok(());
            }
            r.status = DownloadStatus::Queued;
            r.error = None;
        }
        self.inner.status_changed(&e);
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Recommence depuis zéro (fichier partiel supprimé).
    pub async fn restart(&self, id: DownloadId) -> Result<()> {
        let e = self.inner.entry(id)?;
        self.inner.halt(id, DownloadStatus::Paused)?;
        wait_task(&e).await;
        {
            let mut r = e.shared.record.lock();
            if r.status != DownloadStatus::Completed {
                r.remove_partials();
            }
            r.reset_progress();
            r.completed_at = None;
            r.avg_speed = None;
            r.active_ms = 0;
            r.error = None;
            if !r.filename_locked {
                r.filename.clear();
            }
            r.status = DownloadStatus::Queued;
        }
        e.shared.received.store(0, Ordering::Relaxed);
        self.inner.status_changed(&e);
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Retire de la liste et de l'historique. `delete_file` supprime aussi le fichier terminé.
    pub async fn remove(&self, id: DownloadId, delete_file: bool) -> Result<()> {
        let e = self.inner.entries.write().remove(&id).ok_or(EngineError::NotFound)?;
        if let Some(c) = e.rt.lock().cancel.take() {
            c.cancel();
        }
        wait_task(&e).await;
        {
            let r = e.shared.record.lock();
            // Les fichiers partiels n'ont d'utilité qu'avec l'entrée : on les supprime toujours.
            if r.status != DownloadStatus::Completed {
                r.remove_partials();
            }
            if let Some(p) = r.final_path() {
                if delete_file && r.status == DownloadStatus::Completed {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
        self.inner.dirty.lock().remove(&id);
        let store = self.inner.store.clone();
        let _ = tokio::task::spawn_blocking(move || store.delete(&[id])).await;
        let _ = self.inner.events.send(EngineEvent::Removed { id });
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn set_priority(&self, id: DownloadId, p: Priority) -> Result<()> {
        let e = self.inner.entry(id)?;
        e.shared.record.lock().priority = p;
        self.inner.status_changed(&e);
        self.inner.wake.notify_one();
        Ok(())
    }

    /// Limite propre à un fichier (0 = aucune).
    pub fn set_speed_limit(&self, id: DownloadId, bytes_per_sec: u64) -> Result<()> {
        let e = self.inner.entry(id)?;
        e.shared.record.lock().speed_limit = bytes_per_sec;
        e.shared.limiter.set_rate(bytes_per_sec);
        self.inner.status_changed(&e);
        Ok(())
    }

    pub fn pause_all(&self) {
        for id in self.ids() {
            let _ = self.pause(id);
        }
    }

    pub fn resume_all(&self) {
        for id in self.ids() {
            let _ = self.resume(id);
        }
    }

    fn ids(&self) -> Vec<DownloadId> {
        self.inner.entries.read().keys().copied().collect()
    }

    /// Interrompt proprement et enregistre l'état (à appeler à la fermeture de l'application).
    /// Les téléchargements en cours reprendront au prochain démarrage.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let entries: Vec<_> = self.inner.entries.read().values().cloned().collect();
        for e in &entries {
            if let Some(c) = e.rt.lock().cancel.take() {
                c.cancel();
            }
        }
        for e in &entries {
            wait_task(e).await;
        }
        self.inner.save().await;
    }

    /// Force une sauvegarde immédiate.
    pub async fn save(&self) {
        self.inner.save().await;
    }
}

/// Refuse les chemins relatifs ou contenant `..` (traversée de répertoires).
fn validate_directory(d: &Path) -> Result<PathBuf> {
    if !d.is_absolute() || d.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(EngineError::Other(format!("Dossier de destination invalide : {}", d.display())));
    }
    Ok(d.to_path_buf())
}

async fn wait_task(e: &Entry) {
    let task = e.rt.lock().task.take();
    if let Some(t) = task {
        let _ = t.await;
    }
}

impl Inner {
    fn entry(&self, id: DownloadId) -> Result<Arc<Entry>> {
        self.entries.read().get(&id).cloned().ok_or(EngineError::NotFound)
    }

    fn mark_dirty(&self, id: DownloadId) {
        self.dirty.lock().insert(id);
    }

    fn status_changed(&self, e: &Entry) {
        let conns = self.ctx.config.read().connections_per_download;
        let info = e.info(conns);
        self.mark_dirty(info.id);
        let _ = self.events.send(EngineEvent::StatusChanged { info });
    }

    fn halt(&self, id: DownloadId, to: DownloadStatus) -> Result<()> {
        let e = self.entry(id)?;
        {
            let mut r = e.shared.record.lock();
            if r.status == DownloadStatus::Completed || r.status == to {
                return Ok(());
            }
            // Mettre en pause un échec n'a pas de sens ; on garde l'erreur visible.
            if r.status == DownloadStatus::Failed && to == DownloadStatus::Paused {
                return Ok(());
            }
            r.status = to;
        }
        if let Some(c) = e.rt.lock().cancel.take() {
            c.cancel();
        }
        self.status_changed(&e);
        self.wake.notify_one();
        Ok(())
    }

    /// Démarre autant de téléchargements en attente que le permet la limite.
    fn schedule(self: &Arc<Self>) {
        if self.shutdown.is_cancelled() {
            return;
        }
        let max = self.ctx.config.read().max_concurrent_downloads;
        let entries: Vec<_> = self.entries.read().values().cloned().collect();
        let running = entries.iter().filter(|e| e.is_running()).count();
        if running >= max {
            return;
        }
        let mut queued: Vec<_> = entries
            .into_iter()
            .filter(|e| !e.is_running() && e.shared.record.lock().status == DownloadStatus::Queued)
            .collect();
        queued.sort_by_key(|e| {
            let r = e.shared.record.lock();
            (std::cmp::Reverse(r.priority), r.seq)
        });
        for e in queued.into_iter().take(max - running) {
            self.launch(e);
        }
    }

    fn launch(self: &Arc<Self>, e: Arc<Entry>) {
        let cancel = CancellationToken::new();
        e.shared.record.lock().status = DownloadStatus::Probing;
        self.status_changed(&e);

        let inner = self.clone();
        let entry = e.clone();
        let token = cancel.clone();
        let task = tokio::spawn(async move {
            let id = entry.id();
            // Chemins déjà réservés par d'autres téléchargements non terminés.
            let reserved: HashSet<PathBuf> = inner
                .entries
                .read()
                .values()
                .filter(|o| !Arc::ptr_eq(o, &entry))
                .filter_map(|o| {
                    let r = o.shared.record.lock();
                    (r.status != DownloadStatus::Completed).then(|| r.final_path()).flatten()
                })
                .collect();
            let res = download::run(entry.shared.clone(), inner.ctx.clone(), token, |p| reserved.contains(p)).await;

            let elapsed = entry.rt.lock().run_started.take().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
            {
                let mut r = entry.shared.record.lock();
                r.active_ms += elapsed;
                match res {
                    Ok(()) => {
                        r.status = DownloadStatus::Completed;
                        r.completed_at = Some(now_ms());
                        r.error = None;
                        r.segments.iter_mut().for_each(|s| s.active = false);
                        if r.active_ms > 0 {
                            r.avg_speed = r.total_size.map(|t| t * 1000 / r.active_ms);
                        }
                        tracing::info!("terminé : {}", r.filename);
                    }
                    // Le statut (pause, arrêt) a déjà été fixé par l'appelant.
                    Err(EngineError::Cancelled) => {
                        if r.status.is_active() {
                            // Fermeture de l'application : reprise automatique au prochain lancement.
                            r.status = if inner.shutdown.is_cancelled() {
                                DownloadStatus::Queued
                            } else {
                                DownloadStatus::Paused
                            };
                        }
                    }
                    Err(err) => {
                        tracing::warn!("échec {id} : {err}");
                        r.status = DownloadStatus::Failed;
                        r.error = Some(err.to_string());
                    }
                }
            }
            entry.rt.lock().cancel = None;
            inner.status_changed(&entry);
            // Sauvegarde immédiate à chaque fin, puis on relance la file.
            inner.save().await;
            inner.wake.notify_one();
        });
        let mut rt = e.rt.lock();
        rt.cancel = Some(cancel);
        rt.task = Some(task);
        rt.last_sample = Instant::now();
        rt.run_started = Some(Instant::now());
        rt.last_received = e.shared.received.load(Ordering::Relaxed);
        rt.speed = 0.0;
    }

    /// Calcule les vitesses et diffuse la progression.
    fn tick(&self) {
        let conns = self.ctx.config.read().connections_per_download;
        let entries: Vec<_> = self.entries.read().values().cloned().collect();
        let mut items = Vec::new();
        let (mut total_speed, mut active, mut queued) = (0u64, 0u32, 0u32);
        for e in &entries {
            let status = e.shared.record.lock().status;
            match status {
                DownloadStatus::Queued => queued += 1,
                s if s.is_active() => active += 1,
                _ => {}
            }
            if !e.is_running() {
                continue;
            }
            {
                let mut rt = e.rt.lock();
                let now = Instant::now();
                let dt = now.duration_since(rt.last_sample).as_secs_f64().max(0.001);
                let recv = e.shared.received.load(Ordering::Relaxed);
                let inst = recv.saturating_sub(rt.last_received) as f64 / dt;
                // Moyenne exponentielle : réactive mais sans à-coups.
                rt.speed = if inst == 0.0 {
                    // Plus rien ne passe (reconnexion) : la vitesse affichée chute vite.
                    if rt.speed < 1024.0 { 0.0 } else { rt.speed * 0.4 }
                } else if rt.speed == 0.0 {
                    inst
                } else {
                    rt.speed * 0.7 + inst * 0.3
                };
                rt.last_received = recv;
                rt.last_sample = now;
            }
            let info = e.info(conns);
            total_speed += info.speed;
            self.mark_dirty(info.id);
            items.push(info);
        }
        if !items.is_empty() {
            let _ = self.events.send(EngineEvent::Progress { items });
        }
        let _ = self.events.send(EngineEvent::Stats { total_speed, active, queued });
    }

    async fn save(&self) {
        let _guard = self.save_lock.lock().await;
        let ids: Vec<DownloadId> = self.dirty.lock().drain().collect();
        if ids.is_empty() {
            return;
        }
        let records: Vec<DownloadRecord> = {
            let entries = self.entries.read();
            ids.iter()
                .filter_map(|id| entries.get(id))
                .map(|e| e.shared.record.lock().clone())
                .collect()
        };
        let store = self.store.clone();
        let res = tokio::task::spawn_blocking(move || store.upsert_many(&records)).await;
        if !matches!(res, Ok(Ok(()))) {
            tracing::error!("sauvegarde impossible : {res:?}");
            self.dirty.lock().extend(ids);
        }
    }
}

async fn background(weak: Weak<Inner>) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_save = Instant::now();
    loop {
        let Some(inner) = weak.upgrade() else { return };
        if inner.shutdown.is_cancelled() {
            return;
        }
        tokio::select! {
            _ = tick.tick() => {
                inner.tick();
                if last_save.elapsed() >= SAVE_EVERY {
                    inner.save().await;
                    last_save = Instant::now();
                }
            }
            _ = inner.wake.notified() => {}
            _ = inner.shutdown.cancelled() => return,
        }
        inner.schedule();
    }
}

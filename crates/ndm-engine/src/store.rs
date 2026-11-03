//! Persistance SQLite : téléchargements (historique compris) et réglages.
//!
//! Les colonnes indexées servent à la recherche, aux filtres et aux tris ;
//! l'enregistrement complet (segments compris) est conservé en JSON.

use crate::download::DownloadRecord;
use crate::types::*;
use parking_lot::Mutex;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

const SCHEMA_VERSION: i64 = 1;

pub struct Store {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    AddedAt,
    CompletedAt,
    Size,
    Speed,
    /// Type de fichier (catégorie puis nom).
    Type,
    Name,
}

/// Recherche dans l'historique.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryQuery {
    /// Texte recherché dans le nom de fichier et l'URL.
    pub search: Option<String>,
    pub statuses: Vec<DownloadStatus>,
    pub categories: Vec<Category>,
    /// Bornes sur la date d'ajout (ms Unix, incluses).
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub sort: SortKey,
    pub descending: bool,
    pub limit: Option<u32>,
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let store = Self { conn: Mutex::new(Connection::open_in_memory()?) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let c = self.conn.lock();
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v < 1 {
            c.execute_batch(
                "BEGIN;
                CREATE TABLE IF NOT EXISTS downloads (
                    id            TEXT PRIMARY KEY,
                    seq           INTEGER NOT NULL,
                    url           TEXT NOT NULL,
                    filename      TEXT NOT NULL,
                    directory     TEXT NOT NULL,
                    category      TEXT NOT NULL,
                    status        TEXT NOT NULL,
                    priority      INTEGER NOT NULL,
                    total_size    INTEGER,
                    avg_speed     INTEGER,
                    added_at      INTEGER NOT NULL,
                    completed_at  INTEGER,
                    record        TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_dl_status   ON downloads(status);
                CREATE INDEX IF NOT EXISTS idx_dl_added    ON downloads(added_at);
                CREATE INDEX IF NOT EXISTS idx_dl_category ON downloads(category);
                CREATE TABLE IF NOT EXISTS settings (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                PRAGMA user_version = 1;
                COMMIT;",
            )?;
        }
        debug_assert!(SCHEMA_VERSION == 1);
        Ok(())
    }

    pub fn load_all(&self) -> rusqlite::Result<Vec<DownloadRecord>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT record FROM downloads ORDER BY seq")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for json in rows {
            match serde_json::from_str::<DownloadRecord>(&json?) {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!("enregistrement illisible ignoré : {e}"),
            }
        }
        Ok(out)
    }

    pub fn upsert_many(&self, records: &[DownloadRecord]) -> rusqlite::Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "INSERT INTO downloads (id, seq, url, filename, directory, category, status, priority,
                                        total_size, avg_speed, added_at, completed_at, record)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(id) DO UPDATE SET
                    url=excluded.url, filename=excluded.filename, directory=excluded.directory,
                    category=excluded.category, status=excluded.status, priority=excluded.priority,
                    total_size=excluded.total_size, avg_speed=excluded.avg_speed,
                    completed_at=excluded.completed_at, record=excluded.record",
            )?;
            for r in records {
                let json = serde_json::to_string(r).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                st.execute(params![
                    r.id.to_string(),
                    r.seq as i64,
                    r.url,
                    r.filename,
                    r.directory.to_string_lossy(),
                    enum_str(&r.category),
                    enum_str(&r.status),
                    r.priority as i64,
                    r.total_size.map(|v| v as i64),
                    r.avg_speed.map(|v| v as i64),
                    r.added_at as i64,
                    r.completed_at.map(|v| v as i64),
                    json,
                ])?;
            }
        }
        tx.commit()
    }

    pub fn delete(&self, ids: &[DownloadId]) -> rusqlite::Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached("DELETE FROM downloads WHERE id = ?1")?;
            for id in ids {
                st.execute([id.to_string()])?;
            }
        }
        tx.commit()
    }

    /// Renvoie les identifiants correspondant à la requête, dans l'ordre demandé.
    pub fn query(&self, q: &HistoryQuery) -> rusqlite::Result<Vec<DownloadId>> {
        let mut sql = String::from("SELECT id FROM downloads WHERE 1=1");
        let mut args: Vec<rusqlite::types::Value> = Vec::new();

        if let Some(s) = q.search.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let pat = format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
            sql.push_str(" AND (filename LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\')");
            args.push(pat.clone().into());
            args.push(pat.into());
        }
        if !q.statuses.is_empty() {
            sql.push_str(&format!(" AND status IN ({})", vec!["?"; q.statuses.len()].join(",")));
            args.extend(q.statuses.iter().map(|s| enum_str(s).into()));
        }
        if !q.categories.is_empty() {
            sql.push_str(&format!(" AND category IN ({})", vec!["?"; q.categories.len()].join(",")));
            args.extend(q.categories.iter().map(|c| enum_str(c).into()));
        }
        if let Some(f) = q.from_ms {
            sql.push_str(" AND added_at >= ?");
            args.push((f as i64).into());
        }
        if let Some(t) = q.to_ms {
            sql.push_str(" AND added_at <= ?");
            args.push((t as i64).into());
        }
        let dir = if q.descending { "DESC" } else { "ASC" };
        let order = match q.sort {
            SortKey::AddedAt => format!("added_at {dir}"),
            SortKey::CompletedAt => format!("completed_at IS NULL, completed_at {dir}"),
            SortKey::Size => format!("total_size IS NULL, total_size {dir}"),
            SortKey::Speed => format!("avg_speed IS NULL, avg_speed {dir}"),
            SortKey::Type => format!("category {dir}, filename COLLATE NOCASE {dir}"),
            SortKey::Name => format!("filename COLLATE NOCASE {dir}"),
        };
        sql.push_str(&format!(" ORDER BY {order}, seq DESC"));
        if let Some(l) = q.limit {
            sql.push_str(&format!(" LIMIT {l}"));
        }

        let c = self.conn.lock();
        let mut st = c.prepare(&sql)?;
        let ids = st
            .query_map(params_from_iter(args), |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok().and_then(|s| uuid::Uuid::parse_str(&s).ok()))
            .collect();
        Ok(ids)
    }

    pub fn get_setting(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.lock().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

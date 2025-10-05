use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unable to determine data directory")]
    DataDir,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimRecord {
    pub name: String,
    pub pubkey: String,
    pub ip: String,
    pub relays: Vec<String>,
    pub created_at: i64,
    pub fetched_at: i64,
    pub event_id: String,
    pub location: Option<String>,
    pub endpoints: Option<String>,
    pub service_kind: Option<String>,
    pub tls_pubkey: Option<String>,
    pub tls_alg: Option<String>,
    pub blossom_root: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionRecord {
    pub name: String,
    pub pubkey: String,
    pub chosen_at: i64,
}

pub struct Storage {
    pool: Pool<SqliteConnectionManager>,
}

#[derive(Debug)]
struct SqliteCustomizer;

impl r2d2::CustomizeConnection<Connection, rusqlite::Error> for SqliteCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.busy_timeout(Duration::from_secs(1))
    }
}

impl Storage {
    pub fn new() -> Result<Self, StorageError> {
        let path = database_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|_| StorageError::DataDir)?;
        }

        let manager = SqliteConnectionManager::file(&path);
        let pool = Pool::builder()
            .max_size(4)
            .connection_customizer(Box::new(SqliteCustomizer))
            .build(manager)?;

        let conn = pool.get()?;
        initialise_schema(&conn)?;

        Ok(Self { pool })
    }

    /// Create storage with custom path (primarily for testing)
    #[allow(dead_code)]
    pub fn new_with_path(path: &std::path::Path) -> Result<Self, StorageError> {
        let db_path = path.join("frontier.db");
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).map_err(|_| StorageError::DataDir)?;
        }

        let manager = SqliteConnectionManager::file(&db_path);
        let pool = Pool::builder()
            .max_size(4)
            .connection_customizer(Box::new(SqliteCustomizer))
            .build(manager)?;

        let conn = pool.get()?;
        initialise_schema(&conn)?;

        Ok(Self { pool })
    }

    pub fn save_claim(&self, claim: &ClaimRecord) -> Result<(), StorageError> {
        let conn = self.pool.get()?;
        let relays_json = serde_json::to_value(&claim.relays)?;
        conn.execute(
            "INSERT OR REPLACE INTO claims (
                name,
                pubkey,
                ip,
                relays,
                created_at,
                fetched_at,
                event_id,
                location,
                endpoints,
                service_kind,
                tls_pubkey,
                tls_alg,
                blossom_root,
                note
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                claim.name,
                claim.pubkey,
                claim.ip,
                relays_json.to_string(),
                claim.created_at,
                claim.fetched_at,
                claim.event_id,
                claim.location.as_deref(),
                claim.endpoints.as_deref(),
                claim.service_kind.as_deref(),
                claim.tls_pubkey.as_deref(),
                claim.tls_alg.as_deref(),
                claim.blossom_root.as_deref(),
                claim.note.as_deref(),
            ],
        )?;
        Ok(())
    }

    pub fn cached_claims(&self, name: &str) -> Result<Vec<ClaimRecord>, StorageError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT
                name,
                pubkey,
                ip,
                relays,
                created_at,
                fetched_at,
                event_id,
                location,
                endpoints,
                service_kind,
                tls_pubkey,
                tls_alg,
                blossom_root,
                note
            FROM claims WHERE name = ?1",
        )?;
        let mut rows = stmt.query([name])?;
        let mut claims = Vec::new();
        while let Some(row) = rows.next()? {
            let relays_raw: String = row.get(3)?;
            let relays: Vec<String> = if relays_raw.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&relays_raw)?
            };
            claims.push(ClaimRecord {
                name: row.get(0)?,
                pubkey: row.get(1)?,
                ip: row.get(2)?,
                relays,
                created_at: row.get(4)?,
                fetched_at: row.get(5)?,
                event_id: row.get(6)?,
                location: row.get::<_, Option<String>>(7)?,
                endpoints: row.get::<_, Option<String>>(8)?,
                service_kind: row.get::<_, Option<String>>(9)?,
                tls_pubkey: row.get::<_, Option<String>>(10)?,
                tls_alg: row.get::<_, Option<String>>(11)?,
                blossom_root: row.get::<_, Option<String>>(12)?,
                note: row.get::<_, Option<String>>(13)?,
            });
        }
        Ok(claims)
    }

    pub fn record_selection(&self, selection: &SelectionRecord) -> Result<(), StorageError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR REPLACE INTO selections (name, pubkey, chosen_at) VALUES (?1, ?2, ?3)",
            params![selection.name, selection.pubkey, selection.chosen_at],
        )?;
        Ok(())
    }

    pub fn selection(&self, name: &str) -> Result<Option<SelectionRecord>, StorageError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT name, pubkey, chosen_at FROM selections WHERE name = ?1",
            params![name],
            |row| {
                Ok(SelectionRecord {
                    name: row.get(0)?,
                    pubkey: row.get(1)?,
                    chosen_at: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
}

fn database_path() -> Result<PathBuf, StorageError> {
    if let Ok(dir) = std::env::var("FRONTIER_DATA_DIR") {
        let mut path = PathBuf::from(dir);
        path.push("cache.sqlite3");
        return Ok(path);
    }

    if let Some(dirs) = ProjectDirs::from("org", "Frontier", "FrontierBrowser") {
        let mut data_dir = dirs.data_dir().to_path_buf();
        data_dir.push("nns");
        data_dir.push("cache.sqlite3");
        Ok(data_dir)
    } else {
        Err(StorageError::DataDir)
    }
}

fn initialise_schema(conn: &Connection) -> Result<(), StorageError> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS claims (
            name TEXT NOT NULL,
            pubkey TEXT NOT NULL,
            ip TEXT NOT NULL,
            relays TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            fetched_at INTEGER NOT NULL,
            event_id TEXT NOT NULL,
            location TEXT,
            endpoints TEXT,
            service_kind TEXT,
            tls_pubkey TEXT,
            tls_alg TEXT,
            blossom_root TEXT,
            note TEXT,
            PRIMARY KEY (name, pubkey)
        );
        CREATE TABLE IF NOT EXISTS selections (
            name TEXT PRIMARY KEY,
            pubkey TEXT NOT NULL,
            chosen_at INTEGER NOT NULL
        );
        "#,
    )?;
    ensure_claim_column(conn, "location", "TEXT")?;
    ensure_claim_column(conn, "endpoints", "TEXT")?;
    ensure_claim_column(conn, "service_kind", "TEXT")?;
    ensure_claim_column(conn, "tls_pubkey", "TEXT")?;
    ensure_claim_column(conn, "tls_alg", "TEXT")?;
    ensure_claim_column(conn, "blossom_root", "TEXT")?;
    ensure_claim_column(conn, "note", "TEXT")?;
    Ok(())
}

fn ensure_claim_column(conn: &Connection, column: &str, ty: &str) -> Result<(), StorageError> {
    let mut stmt = conn.prepare("PRAGMA table_info(claims)")?;
    let mut rows = stmt.query([])?;
    let mut has_column = false;
    while let Some(row) = rows.next()? {
        let column_name: String = row.get(1)?;
        if column_name == column {
            has_column = true;
            break;
        }
    }

    if !has_column {
        let sql = format!("ALTER TABLE claims ADD COLUMN {column} {ty}");
        conn.execute(sql.as_str(), [])?;
    }

    Ok(())
}

pub fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use r2d2::Pool;
    use r2d2_sqlite::SqliteConnectionManager;
    use tempfile::TempDir;

    fn temp_storage() -> Storage {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cache.sqlite3");
        let manager = SqliteConnectionManager::file(&path);
        let pool = Pool::builder()
            .max_size(2)
            .connection_customizer(Box::new(SqliteCustomizer))
            .build(manager)
            .unwrap();
        initialise_schema(&pool.get().unwrap()).unwrap();
        Storage { pool }
    }

    #[test]
    fn round_trip_claim() {
        let storage = temp_storage();
        let claim = ClaimRecord {
            name: "test".into(),
            pubkey: "pub".into(),
            ip: "127.0.0.1:8080".into(),
            relays: vec!["wss://example".into()],
            created_at: 1,
            fetched_at: 2,
            event_id: "1".into(),
            location: None,
            endpoints: None,
            service_kind: None,
            tls_pubkey: None,
            tls_alg: None,
            blossom_root: None,
            note: None,
        };
        storage.save_claim(&claim).unwrap();
        let claims = storage.cached_claims("test").unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].pubkey, "pub");
    }

    #[test]
    fn round_trip_selection() {
        let storage = temp_storage();
        let selection = SelectionRecord {
            name: "test".into(),
            pubkey: "pub".into(),
            chosen_at: 5,
        };
        storage.record_selection(&selection).unwrap();
        let stored = storage.selection("test").unwrap();
        assert_eq!(stored.unwrap().pubkey, "pub");
    }
}

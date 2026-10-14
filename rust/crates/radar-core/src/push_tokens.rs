//! Push-token store (spec-shared §9): one SQLite file at `<STATE_DIR>/push_tokens.sqlite`.
//!
//! Every operation opens a new connection, as the Python version does. The schema is idempotent
//! (`CREATE TABLE IF NOT EXISTS`) and byte-identical to the Python one, so both can share the file.
//!
//! Policy (defect D14, decided here): `token` remains the **global** primary key. Registering a token for
//! another user reassigns it (`ON CONFLICT(token) DO UPDATE`). Changing the key would change the on-disk
//! schema that the Python workers still read during cutover. The Python docstring is wrong, not the schema.

use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

/// File name inside the state directory.
pub const DB_FILE: &str = "push_tokens.sqlite";
/// SQLite busy timeout (spec §15.1: 5 s).
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The idempotent schema (spec §9.2). The text matches the Python string byte for byte, including the
/// indentation that SQLite keeps in `sqlite_master.sql`.
pub const SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS push_tokens (
            user_id    TEXT NOT NULL,
            token      TEXT NOT NULL PRIMARY KEY,
            platform   TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user ON push_tokens(user_id);
        ";

/// Errors from the push-token store.
#[derive(Debug, Error)]
pub enum PushTokenError {
    /// The state directory could not be created.
    #[error("creating state directory: {0}")]
    Io(#[from] std::io::Error),
    /// A SQLite operation failed.
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
}

/// One registered device token.
#[derive(Debug, Clone, PartialEq)]
pub struct PushToken {
    /// Owner.
    pub user_id: String,
    /// Device token (globally unique).
    pub token: String,
    /// `"ios"` or `"android"` (not validated).
    pub platform: String,
    /// Unix seconds of the first registration.
    pub created_at: f64,
}

/// Handle to the push-token database. Holds only the path, so it is cheap to clone.
#[derive(Debug, Clone)]
pub struct PushTokenStore {
    db_path: PathBuf,
    state_dir: PathBuf,
}

impl PushTokenStore {
    /// Store rooted at `state_dir` (`<state_dir>/push_tokens.sqlite`).
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        let state_dir = state_dir.into();
        Self {
            db_path: state_dir.join(DB_FILE),
            state_dir,
        }
    }

    /// The database file.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn connect(&self) -> Result<Connection, PushTokenError> {
        std::fs::create_dir_all(&self.state_dir)?;
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.execute_batch(SCHEMA)?;
        Ok(conn)
    }

    /// Register `token` for `user_id` on `platform`. Re-registering updates the owner and platform and keeps
    /// the original `created_at`.
    pub fn upsert(&self, user_id: &str, token: &str, platform: &str) -> Result<(), PushTokenError> {
        self.upsert_at(user_id, token, platform, now_epoch())
    }

    /// [`PushTokenStore::upsert`] with an explicit `created_at` (tests and imports).
    pub fn upsert_at(
        &self,
        user_id: &str,
        token: &str,
        platform: &str,
        created_at: f64,
    ) -> Result<(), PushTokenError> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO push_tokens(user_id, token, platform, created_at) VALUES(?1, ?2, ?3, ?4) \
             ON CONFLICT(token) DO UPDATE SET user_id=excluded.user_id, platform=excluded.platform",
            params![user_id, token, platform, created_at],
        )?;
        Ok(())
    }

    /// Delete every row with this token. Returns the number of rows removed.
    pub fn delete_by_token(&self, token: &str) -> Result<usize, PushTokenError> {
        let conn = self.connect()?;
        Ok(conn.execute("DELETE FROM push_tokens WHERE token = ?1", params![token])?)
    }

    /// Delete the row for this user and token. Returns the number of rows removed.
    pub fn delete_for_user(&self, user_id: &str, token: &str) -> Result<usize, PushTokenError> {
        let conn = self.connect()?;
        Ok(conn.execute(
            "DELETE FROM push_tokens WHERE user_id = ?1 AND token = ?2",
            params![user_id, token],
        )?)
    }

    /// All tokens for a user. No order is guaranteed (spec §9.4).
    pub fn list_for_user(&self, user_id: &str) -> Result<Vec<PushToken>, PushTokenError> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT user_id, token, platform, created_at FROM push_tokens WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map(params![user_id], |row| {
            Ok(PushToken {
                user_id: row.get(0)?,
                token: row.get(1)?,
                platform: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_reassigns_global_token_and_keeps_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let store = PushTokenStore::new(dir.path().join("state"));
        store.upsert_at("alice", "tok-1", "ios", 100.0).unwrap();
        store.upsert_at("alice", "tok-1", "android", 200.0).unwrap();
        assert_eq!(store.list_for_user("alice").unwrap()[0].created_at, 100.0);
        assert_eq!(store.list_for_user("alice").unwrap()[0].platform, "android");
        // D14: the same token registered by another user moves to that user.
        store.upsert_at("bob", "tok-1", "ios", 300.0).unwrap();
        assert!(store.list_for_user("alice").unwrap().is_empty());
        let bobs = store.list_for_user("bob").unwrap();
        assert_eq!(bobs.len(), 1);
        assert_eq!(bobs[0].created_at, 100.0);
    }

    #[test]
    fn deletes_report_rowcounts() {
        let dir = tempfile::tempdir().unwrap();
        let store = PushTokenStore::new(dir.path());
        store.upsert_at("u", "t1", "ios", 1.0).unwrap();
        store.upsert_at("u", "t2", "ios", 1.0).unwrap();
        assert_eq!(store.delete_for_user("other", "t1").unwrap(), 0);
        assert_eq!(store.delete_for_user("u", "t1").unwrap(), 1);
        assert_eq!(store.delete_by_token("t2").unwrap(), 1);
        assert_eq!(store.delete_by_token("t2").unwrap(), 0);
    }

    #[test]
    fn schema_matches_python_sqlite_master_golden() {
        let dir = tempfile::tempdir().unwrap();
        let store = PushTokenStore::new(dir.path());
        store.upsert_at("u", "t", "ios", 1.0).unwrap();
        let conn = Connection::open(store.db_path()).unwrap();
        let mut stmt = conn
            .prepare("SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name")
            .unwrap();
        let rows: Vec<serde_json::Value> = stmt
            .query_map([], |r| {
                let name: String = r.get(0)?;
                let sql: String = r.get(1)?;
                Ok(serde_json::json!({"name": name, "sql": sql}))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/push_tokens/schema.json")).unwrap();
        assert_eq!(serde_json::Value::Array(rows), golden);
    }

    #[test]
    fn schema_is_idempotent_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let store = PushTokenStore::new(dir.path());
        store.upsert_at("u", "t", "ios", 1.0).unwrap();
        assert_eq!(store.list_for_user("u").unwrap().len(), 1);
    }
}

//! Postgres-backed state store for RFC #389 Phase 1 (`STATE_DIR` -> Postgres).
//!
//! Opt-in via environment variables:
//! - `RADAR_STATE_DATABASE_URL`: Postgres connection string. When unset, all helpers are no-ops
//!   and the backend uses `STATE_DIR` on disk as before.
//! - `RADAR_STATE_SCHEMA`: schema name (default `"radar"`).
//! - `RADAR_STATE_READ_PG`: `"1"` / `"true"` to serve reads from Postgres instead of `STATE_DIR`
//!   (default `false`, i.e. dual-write shadow mode for parity verification and instant rollback).

use std::io;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};

use crate::push_tokens::PushToken;

/// Re-exported error type for callers outside `radar-core` that implement `From<PgError>`.
pub type PgError = anyhow::Error;

/// Default Postgres schema name for core radar state (shares `level2-postgres` with a separate role).
pub const DEFAULT_SCHEMA: &str = "radar";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
/// Advisory lock namespace for `state_docs` key-level read-modify-write transactions.
const STATE_DOC_LOCK_NS: i32 = 291;

/// Configuration parsed from `RADAR_STATE_*` environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgStateConfig {
    /// Postgres connection URL (`RADAR_STATE_DATABASE_URL`).
    pub database_url: String,
    /// Validated schema name (`RADAR_STATE_SCHEMA`, default `"radar"`).
    pub schema: String,
    /// When `true`, readers serve from Postgres (`RADAR_STATE_READ_PG=1`); when `false`, Postgres
    /// receives shadow dual-writes while `STATE_DIR` remains the read source.
    pub read_pg: bool,
}

impl PgStateConfig {
    /// Read from process environment. Returns `Ok(None)` when `RADAR_STATE_DATABASE_URL` is unset.
    pub fn from_env() -> Result<Option<Self>> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Read from a custom lookup function (used in tests).
    pub fn from_lookup<F>(mut get: F) -> Result<Option<Self>>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let Some(url) = get("RADAR_STATE_DATABASE_URL")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        let schema = get("RADAR_STATE_SCHEMA")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_SCHEMA.to_string());
        validate_schema(&schema)?;
        let read_pg = get("RADAR_STATE_READ_PG").is_some_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
        Ok(Some(Self {
            database_url: url,
            schema,
            read_pg,
        }))
    }
}

/// Validate a Postgres schema identifier (`[a-z][a-z0-9_]{0,62}`).
pub fn validate_schema(schema: &str) -> Result<()> {
    let mut bytes = schema.bytes();
    let Some(first) = bytes.next() else {
        bail!("RADAR_STATE_SCHEMA must not be empty");
    };
    ensure!(
        first.is_ascii_lowercase()
            && schema.len() <= 63
            && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "invalid RADAR_STATE_SCHEMA {schema:?}: must match [a-z][a-z0-9_]{{0,62}}"
    );
    Ok(())
}

/// Validate a relative state document key (e.g. `"manifest.json"` or `"star-seeing/20261126T18Z.bin"`).
pub fn validate_key(key: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.len() <= 255
            && !key.starts_with('/')
            && !key.contains('\0')
            && !key.contains('\\')
            && !key
                .split('/')
                .any(|seg| seg.is_empty() || seg == "." || seg == ".."),
        "invalid state document key {key:?}"
    );
    Ok(())
}

/// Idempotent DDL for `schema`.
pub fn schema_ddl(schema: &str) -> Result<String> {
    validate_schema(schema)?;
    Ok(format!(
        "CREATE SCHEMA IF NOT EXISTS {schema};
         CREATE TABLE IF NOT EXISTS {schema}.state_docs (
             key        text PRIMARY KEY,
             body       bytea NOT NULL,
             revision   bigint NOT NULL DEFAULT 1,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE IF NOT EXISTS {schema}.push_tokens (
             token      text PRIMARY KEY,
             user_id    text NOT NULL,
             platform   text NOT NULL,
             created_at double precision NOT NULL,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE INDEX IF NOT EXISTS idx_push_tokens_user_id ON {schema}.push_tokens(user_id);"
    ))
}

async fn open_client(url: &str) -> Result<Client> {
    let mut config: tokio_postgres::Config =
        url.parse().context("invalid RADAR_STATE_DATABASE_URL")?;
    if config.get_connect_timeout().is_none() {
        config.connect_timeout(CONNECT_TIMEOUT);
    }
    config.keepalives(true).keepalives_idle(KEEPALIVE_IDLE);
    let (client, connection) = tokio::time::timeout(CONNECT_TIMEOUT, config.connect(NoTls))
        .await
        .context("state Postgres connect timed out")?
        .context("state Postgres connect failed")?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            tracing::warn!(err = %err, "state_pg_connection_closed");
        }
    });
    client
        .batch_execute("SET statement_timeout = '15s'; SET lock_timeout = '10s'")
        .await
        .context("setting state Postgres timeouts")?;
    Ok(client)
}

struct Inner {
    url: String,
    schema: String,
    client: Option<Client>,
    initialized: bool,
}

impl Inner {
    async fn client(&mut self) -> Result<&mut Client> {
        let needs_open = self.client.as_ref().is_none_or(Client::is_closed);
        if needs_open {
            let client = open_client(&self.url).await?;
            self.client = Some(client);
        }
        let client = self
            .client
            .as_mut()
            .context("state Postgres client missing")?;
        if !self.initialized {
            let ddl = schema_ddl(&self.schema)?;
            client
                .batch_execute(&ddl)
                .await
                .context("initializing radar state Postgres schema")?;
            self.initialized = true;
        }
        Ok(client)
    }
}

/// Postgres-backed state store usable from both async tasks and synchronous blocking threads.
#[derive(Clone)]
pub struct PgStateStore {
    inner: Arc<Mutex<Inner>>,
    schema: String,
    read_pg: bool,
    rt: Arc<tokio::runtime::Runtime>,
}

impl PgStateStore {
    /// Create a lazy-connecting store from `cfg` (connects and initializes schema on first use).
    pub fn new(cfg: PgStateConfig) -> Result<Self> {
        validate_schema(&cfg.schema)?;
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .thread_name("radar-state-pg")
            .build()
            .context("building state_pg runtime")?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                url: cfg.database_url,
                schema: cfg.schema.clone(),
                client: None,
                initialized: false,
            })),
            schema: cfg.schema,
            read_pg: cfg.read_pg,
            rt: Arc::new(rt),
        })
    }

    /// True when `RADAR_STATE_READ_PG` is enabled for this store.
    #[must_use]
    pub fn read_pg(&self) -> bool {
        self.read_pg
    }

    /// Upsert `key` with `body`, bumping `revision` atomically. Returns the new revision.
    pub async fn put(&self, key: &str, body: &[u8]) -> Result<i64> {
        validate_key(key)?;
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "INSERT INTO {}.state_docs (key, body, revision, updated_at)
             VALUES ($1, $2, 1, now())
             ON CONFLICT (key) DO UPDATE
             SET body = excluded.body,
                 revision = {}.state_docs.revision + 1,
                 updated_at = now()
             RETURNING revision",
            self.schema, self.schema
        );
        let row = client.query_one(&sql, &[&key, &body]).await?;
        Ok(row.get(0))
    }

    /// Fetch `(body, revision)` for `key`, or `None` when absent.
    pub async fn get(&self, key: &str) -> Result<Option<(Vec<u8>, i64)>> {
        validate_key(key)?;
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "SELECT body, revision FROM {}.state_docs WHERE key = $1",
            self.schema
        );
        let row = client.query_opt(&sql, &[&key]).await?;
        Ok(row.map(|r| (r.get(0), r.get(1))))
    }

    /// Delete `key`. Returns `true` if a row was removed.
    pub async fn delete(&self, key: &str) -> Result<bool> {
        validate_key(key)?;
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!("DELETE FROM {}.state_docs WHERE key = $1", self.schema);
        Ok(client.execute(&sql, &[&key]).await? > 0)
    }

    /// Fetch only the current `revision` for `key`, or `None` when absent.
    pub async fn get_revision(&self, key: &str) -> Result<Option<i64>> {
        validate_key(key)?;
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "SELECT revision FROM {}.state_docs WHERE key = $1",
            self.schema
        );
        let row = client.query_opt(&sql, &[&key]).await?;
        Ok(row.map(|r| r.get(0)))
    }

    /// List all keys starting with `prefix` (ordered ascending).
    pub async fn list_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        ensure!(
            !prefix.is_empty() && !prefix.contains('\0') && !prefix.contains('\\'),
            "invalid state_docs prefix {prefix:?}"
        );
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let escaped = prefix.replace('%', "\\%").replace('_', "\\_");
        let pattern = format!("{escaped}%");
        let sql = format!(
            "SELECT key FROM {}.state_docs WHERE key LIKE $1 ESCAPE '\\' ORDER BY key",
            self.schema
        );
        let rows = client.query(&sql, &[&pattern]).await?;
        Ok(rows.into_iter().map(|r| r.get(0)).collect())
    }

    /// Run a key-level advisory-locked read-modify-write transaction for `key`.
    pub async fn update_doc<E, F>(
        &self,
        key: &str,
        mutate: F,
    ) -> std::result::Result<(Vec<u8>, i64), E>
    where
        F: FnOnce(Option<Vec<u8>>) -> std::result::Result<Vec<u8>, E>,
        E: From<anyhow::Error>,
    {
        match self
            .update_doc_opt(key, |cur| mutate(cur).map(Some))
            .await?
        {
            Some(pair) => Ok(pair),
            None => Err(E::from(anyhow::anyhow!(
                "update_doc unexpectedly returned None"
            ))),
        }
    }

    /// Run a key-level advisory-locked read-modify-write transaction that may skip writing by
    /// returning `Ok(None)`.
    pub async fn update_doc_opt<E, F>(
        &self,
        key: &str,
        mutate: F,
    ) -> std::result::Result<Option<(Vec<u8>, i64)>, E>
    where
        F: FnOnce(Option<Vec<u8>>) -> std::result::Result<Option<Vec<u8>>, E>,
        E: From<anyhow::Error>,
    {
        validate_key(key)?;
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let tx = client
            .transaction()
            .await
            .context("begin state_docs transaction")?;
        tx.query_one(
            "SELECT pg_advisory_xact_lock($1, hashtext($2))",
            &[&STATE_DOC_LOCK_NS, &key],
        )
        .await
        .context("acquire state_docs advisory lock")?;
        let sel = format!("SELECT body FROM {}.state_docs WHERE key = $1", self.schema);
        let current: Option<Vec<u8>> = tx
            .query_opt(&sel, &[&key])
            .await
            .context("read state_docs in transaction")?
            .map(|r| r.get(0));
        let Some(next) = mutate(current)? else {
            tx.rollback()
                .await
                .context("rollback skipped state_docs transaction")?;
            return Ok(None);
        };
        let upsert = format!(
            "INSERT INTO {}.state_docs (key, body, revision, updated_at)
             VALUES ($1, $2, 1, now())
             ON CONFLICT (key) DO UPDATE
             SET body = excluded.body,
                 revision = {}.state_docs.revision + 1,
                 updated_at = now()
             RETURNING revision",
            self.schema, self.schema
        );
        let rev: i64 = tx
            .query_one(&upsert, &[&key, &next])
            .await
            .context("upsert state_docs in transaction")?
            .get(0);
        tx.commit().await.context("commit state_docs transaction")?;
        Ok(Some((next, rev)))
    }

    /// Upsert a push token, reassigning `user_id`/`platform` on conflict while preserving `created_at`.
    pub async fn push_token_upsert(
        &self,
        user_id: &str,
        token: &str,
        platform: &str,
        created_at: f64,
    ) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "INSERT INTO {}.push_tokens (token, user_id, platform, created_at, updated_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (token) DO UPDATE
             SET user_id = excluded.user_id,
                 platform = excluded.platform,
                 updated_at = now()",
            self.schema
        );
        client
            .execute(&sql, &[&token, &user_id, &platform, &created_at])
            .await?;
        Ok(())
    }

    /// Delete every row with `token`. Returns the number of rows removed.
    pub async fn push_token_delete_by_token(&self, token: &str) -> Result<usize> {
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!("DELETE FROM {}.push_tokens WHERE token = $1", self.schema);
        Ok(client.execute(&sql, &[&token]).await? as usize)
    }

    /// Delete the row matching `(user_id, token)`. Returns the number of rows removed.
    pub async fn push_token_delete_for_user(&self, user_id: &str, token: &str) -> Result<usize> {
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "DELETE FROM {}.push_tokens WHERE user_id = $1 AND token = $2",
            self.schema
        );
        Ok(client.execute(&sql, &[&user_id, &token]).await? as usize)
    }

    /// List all tokens belonging to `user_id`.
    pub async fn push_token_list_for_user(&self, user_id: &str) -> Result<Vec<PushToken>> {
        let mut guard = self.inner.lock().await;
        let client = guard.client().await?;
        let sql = format!(
            "SELECT user_id, token, platform, created_at FROM {}.push_tokens WHERE user_id = $1",
            self.schema
        );
        let rows = client.query(&sql, &[&user_id]).await?;
        Ok(rows
            .into_iter()
            .map(|r| PushToken {
                user_id: r.get(0),
                token: r.get(1),
                platform: r.get(2),
                created_at: r.get(3),
            })
            .collect())
    }

    /// Blocking wrapper around [`Self::put`] safe to call from `spawn_blocking` or plain threads.
    pub fn put_blocking(&self, key: &str, body: &[u8]) -> Result<i64> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.put(key, body)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg put thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::get`] safe to call from `spawn_blocking` or plain threads.
    pub fn get_blocking(&self, key: &str) -> Result<Option<(Vec<u8>, i64)>> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.get(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg get thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::get_revision`].
    pub fn get_revision_blocking(&self, key: &str) -> Result<Option<i64>> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.get_revision(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg get_revision thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::delete`].
    pub fn delete_blocking(&self, key: &str) -> Result<bool> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.delete(key)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg delete thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::list_prefix`].
    pub fn list_prefix_blocking(&self, prefix: &str) -> Result<Vec<String>> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.list_prefix(prefix)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg list_prefix thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::update_doc`] safe to call from `spawn_blocking` or plain threads.
    pub fn update_doc_blocking<E, F>(
        &self,
        key: &str,
        mutate: F,
    ) -> std::result::Result<(Vec<u8>, i64), E>
    where
        F: FnOnce(Option<Vec<u8>>) -> std::result::Result<Vec<u8>, E> + Send,
        E: From<anyhow::Error> + Send,
    {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.update_doc(key, mutate)))
                .join()
                .map_err(|_| E::from(anyhow::anyhow!("state_pg update_doc thread panicked")))?
        })
    }

    /// Blocking wrapper around [`Self::update_doc_opt`].
    pub fn update_doc_opt_blocking<E, F>(
        &self,
        key: &str,
        mutate: F,
    ) -> std::result::Result<Option<(Vec<u8>, i64)>, E>
    where
        F: FnOnce(Option<Vec<u8>>) -> std::result::Result<Option<Vec<u8>>, E> + Send,
        E: From<anyhow::Error> + Send,
    {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.update_doc_opt(key, mutate)))
                .join()
                .map_err(|_| E::from(anyhow::anyhow!("state_pg update_doc_opt thread panicked")))?
        })
    }

    /// Blocking wrapper around [`Self::push_token_upsert`].
    pub fn push_token_upsert_blocking(
        &self,
        user_id: &str,
        token: &str,
        platform: &str,
        created_at: f64,
    ) -> Result<()> {
        std::thread::scope(|s| {
            s.spawn(|| {
                self.rt
                    .block_on(self.push_token_upsert(user_id, token, platform, created_at))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("state_pg push_token_upsert thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::push_token_delete_by_token`].
    pub fn push_token_delete_by_token_blocking(&self, token: &str) -> Result<usize> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.push_token_delete_by_token(token)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg push_token_delete thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::push_token_delete_for_user`].
    pub fn push_token_delete_for_user_blocking(&self, user_id: &str, token: &str) -> Result<usize> {
        std::thread::scope(|s| {
            s.spawn(|| {
                self.rt
                    .block_on(self.push_token_delete_for_user(user_id, token))
            })
            .join()
            .map_err(|_| anyhow::anyhow!("state_pg push_token_delete_for_user thread panicked"))?
        })
    }

    /// Blocking wrapper around [`Self::push_token_list_for_user`].
    pub fn push_token_list_for_user_blocking(&self, user_id: &str) -> Result<Vec<PushToken>> {
        std::thread::scope(|s| {
            s.spawn(|| self.rt.block_on(self.push_token_list_for_user(user_id)))
                .join()
                .map_err(|_| anyhow::anyhow!("state_pg push_token_list thread panicked"))?
        })
    }
}

static GLOBAL: OnceLock<Option<PgStateStore>> = OnceLock::new();

/// Process-wide Postgres state store, initialized once from `RADAR_STATE_*` env vars.
/// Returns `None` when `RADAR_STATE_DATABASE_URL` is unset.
pub fn global() -> Option<&'static PgStateStore> {
    GLOBAL
        .get_or_init(|| match PgStateConfig::from_env() {
            Ok(Some(cfg)) => match PgStateStore::new(cfg) {
                Ok(store) => Some(store),
                Err(err) => {
                    tracing::error!(err = %err, "state_pg_init_failed");
                    None
                }
            },
            Ok(None) => None,
            Err(err) => {
                tracing::error!(err = %err, "state_pg_config_invalid");
                None
            }
        })
        .as_ref()
}

/// True when Postgres is configured AND `RADAR_STATE_READ_PG=1`.
#[must_use]
pub fn read_enabled() -> bool {
    global().is_some_and(PgStateStore::read_pg)
}

/// Dual-write `bytes` for `key` to Postgres when configured.
///
/// In shadow mode (`RADAR_STATE_READ_PG=0`), a Postgres write failure is logged as a warning so a
/// transient database issue never breaks disk-backed serving.
pub fn mirror_write(key: &str, bytes: &[u8]) {
    let Some(store) = global() else {
        return;
    };
    if let Err(err) = store.put_blocking(key, bytes) {
        tracing::warn!(key, err = %format!("{err:#}"), "state_pg_mirror_write_failed");
    }
}

/// Read `key` from Postgres when `RADAR_STATE_READ_PG=1`, otherwise read `path` from disk.
///
/// Returns `Ok(None)` when the document/file does not exist.
pub fn read_doc_or_disk(path: &Path, key: Option<&str>) -> io::Result<Option<Vec<u8>>> {
    if let (Some(pg), Some(key)) = (global().filter(|s| s.read_pg()), key) {
        return pg
            .get_blocking(key)
            .map(|opt| opt.map(|(body, _)| body))
            .map_err(io::Error::other);
    }
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Write `bytes` to Postgres when `RADAR_STATE_READ_PG=1`, otherwise run `disk_write` and
/// best-effort mirror `bytes` to Postgres if configured.
pub fn write_or_mirror(
    key: Option<&str>,
    bytes: &[u8],
    disk_write: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if let (Some(pg), Some(key)) = (global().filter(|s| s.read_pg()), key) {
        pg.put_blocking(key, bytes).map_err(io::Error::other)?;
        return Ok(());
    }
    disk_write()?;
    if let Some(key) = key {
        mirror_write(key, bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn schema_validation_accepts_safe_identifiers_and_rejects_injection() {
        for good in ["radar", "radar_v1", "a", "state_2026"] {
            assert!(validate_schema(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "Radar",
            "1radar",
            "radar-ng",
            "radar; DROP TABLE x",
            "a.b",
        ] {
            assert!(validate_schema(bad).is_err(), "{bad}");
        }
        let too_long = "a".repeat(64);
        assert!(validate_schema(&too_long).is_err());
    }

    #[test]
    fn key_validation_rejects_traversal_and_absolute_paths() {
        for good in [
            "manifest.json",
            "storms.json",
            "star-seeing/20261126T18Z.bin",
        ] {
            assert!(validate_key(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../manifest.json",
            "a//b",
            "a/./b",
            "a\\b",
            "a\0b",
        ] {
            assert!(validate_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn config_from_lookup_defaults_and_read_flag() {
        let empty: HashMap<&str, &str> = HashMap::new();
        assert_eq!(
            PgStateConfig::from_lookup(|k| empty.get(k).map(|v| (*v).to_string())).unwrap(),
            None
        );

        let shadow = HashMap::from([("RADAR_STATE_DATABASE_URL", "postgres://u:p@host/db")]);
        let cfg = PgStateConfig::from_lookup(|k| shadow.get(k).map(|v| (*v).to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.schema, "radar");
        assert!(!cfg.read_pg);

        let active = HashMap::from([
            ("RADAR_STATE_DATABASE_URL", "postgres://u:p@host/db"),
            ("RADAR_STATE_SCHEMA", "radar_prod"),
            ("RADAR_STATE_READ_PG", "1"),
        ]);
        let cfg = PgStateConfig::from_lookup(|k| active.get(k).map(|v| (*v).to_string()))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.schema, "radar_prod");
        assert!(cfg.read_pg);
    }

    #[test]
    fn ddl_uses_validated_schema() {
        let ddl = schema_ddl("radar").unwrap();
        assert!(ddl.contains("CREATE SCHEMA IF NOT EXISTS radar;"));
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS radar.state_docs"));
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS radar.push_tokens"));
        assert!(ddl.contains("user_id    text NOT NULL"));
        assert!(ddl.contains("created_at double precision NOT NULL"));
        assert!(ddl.contains("idx_push_tokens_user_id ON radar.push_tokens(user_id)"));
    }

    #[test]
    fn disk_fallback_helpers_round_trip_when_pg_unset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.json");
        assert!(
            read_doc_or_disk(&path, Some("sample.json"))
                .unwrap()
                .is_none()
        );
        write_or_mirror(Some("sample.json"), b"{\"ok\":true}", || {
            std::fs::write(&path, b"{\"ok\":true}")
        })
        .unwrap();
        assert_eq!(
            read_doc_or_disk(&path, Some("sample.json")).unwrap(),
            Some(b"{\"ok\":true}".to_vec())
        );
    }
}

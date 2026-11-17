//! Saved connections: their non-secret configuration and where it is stored.
//!
//! Passwords and passphrases are never part of [`ConnectionConfig`]; they live
//! in the OS keychain (see `secrets.rs`), keyed by the connection's id.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::db::DbKind;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    None,
    Dev,
    Staging,
    Prod,
}

/// PostgreSQL's `sslmode` levels. For ClickHouse only `Disable` (HTTP) and
/// `VerifyFull` (HTTPS) are meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tls {
    pub mode: TlsMode,
    /// PEM file with the CA certificate(s) to trust; empty uses the system store.
    pub ca_cert_path: String,
    /// PEM client certificate and key, for mutual TLS (PostgreSQL).
    pub client_cert_path: String,
    pub client_key_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    #[default]
    Agent,
    Password,
    Key,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Ssh {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// Private key file, for `SshAuth::Key`.
    pub key_path: String,
}

impl Default for Ssh {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: SshAuth::default(),
            key_path: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    pub folder: String,
    /// CSS color for the connection's accent, or empty.
    pub color: String,
    pub environment: Environment,
    pub read_only: bool,

    // PostgreSQL and ClickHouse
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    /// PostgreSQL: look the password up in `~/.pgpass` when none is saved.
    pub use_pgpass: bool,
    pub tls: Tls,
    pub ssh: Ssh,

    // SQLite
    pub path: String,
    /// Create the file if it does not exist.
    pub create: bool,

    /// Off = nothing about this connection is ever sent to an AI provider.
    pub ai_enabled: bool,
    /// Let the schema index carry a few example values of low-cardinality columns.
    pub ai_sample_values: bool,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: DbKind::Postgres,
            folder: String::new(),
            color: String::new(),
            environment: Environment::default(),
            read_only: false,
            host: "localhost".into(),
            port: 5432,
            database: String::new(),
            user: String::new(),
            use_pgpass: true,
            tls: Tls::default(),
            ssh: Ssh::default(),
            path: String::new(),
            create: false,
            ai_enabled: true,
            ai_sample_values: false,
        }
    }
}

impl ConnectionConfig {
    /// Trims text fields and rejects configurations that cannot work.
    pub fn validate(mut self) -> Result<Self> {
        for field in [
            &mut self.name,
            &mut self.folder,
            &mut self.host,
            &mut self.database,
            &mut self.user,
            &mut self.path,
        ] {
            *field = field.trim().to_string();
        }
        if self.name.is_empty() {
            return Err(Error::Other("Give the connection a name.".into()));
        }
        match self.kind {
            DbKind::Sqlite => {
                if self.path.is_empty() {
                    return Err(Error::Other("Choose a SQLite database file.".into()));
                }
                self.ssh.enabled = false;
            }
            _ => {
                if self.host.is_empty() {
                    return Err(Error::Other("Enter a host.".into()));
                }
                if self.port == 0 {
                    return Err(Error::Other("Enter a port.".into()));
                }
                if self.ssh.enabled
                    && (self.ssh.host.trim().is_empty() || self.ssh.user.trim().is_empty())
                {
                    return Err(Error::Other(
                        "The SSH tunnel needs a host and a user.".into(),
                    ));
                }
            }
        }
        if !self.color.is_empty()
            && !(self.color.starts_with('#')
                && matches!(self.color.len(), 4 | 7)
                && self.color[1..].chars().all(|c| c.is_ascii_hexdigit()))
        {
            self.color.clear();
        }
        Ok(self)
    }

    pub fn is_prod(&self) -> bool {
        self.environment == Environment::Prod
    }
}

/// The app's own SQLite file (`rowboat.db`). Holds connections now; history
/// and the schema index join it in later milestones.
pub struct AppDb {
    path: PathBuf,
    conn: Mutex<Connection>,
}

impl AppDb {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(&path).map_err(db_err)?;
        Self::init(conn, path)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(
            Connection::open_in_memory().map_err(db_err)?,
            PathBuf::new(),
        )
    }

    fn init(conn: Connection, path: PathBuf) -> Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS connections (
                 id       TEXT PRIMARY KEY,
                 position INTEGER NOT NULL,
                 config   TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS history (
                 id            INTEGER PRIMARY KEY,
                 connection_id TEXT NOT NULL,
                 sql           TEXT NOT NULL,
                 started_at    INTEGER NOT NULL,
                 elapsed_ms    INTEGER NOT NULL,
                 rows          INTEGER,
                 error         TEXT
             );
             CREATE INDEX IF NOT EXISTS history_by_connection ON history (connection_id, id DESC);
             CREATE TABLE IF NOT EXISTS session (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .map_err(db_err)?;
        Ok(Self {
            path,
            conn: Mutex::new(conn),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Result<Vec<ConnectionConfig>> {
        let conn = self.lock();
        let mut stmt = conn
            .prepare("SELECT config FROM connections ORDER BY position, rowid")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for json in rows {
            // A row written by a newer version may not parse; skip it rather
            // than lose access to every other connection.
            if let Ok(config) = serde_json::from_str(&json.map_err(db_err)?) {
                out.push(config);
            }
        }
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<ConnectionConfig> {
        self.list()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::Other("That connection no longer exists.".into()))
    }

    /// Inserts or replaces a connection. A config without an id gets one.
    pub fn save(&self, config: ConnectionConfig) -> Result<ConnectionConfig> {
        let mut config = config.validate()?;
        if config.id.is_empty() {
            config.id = uuid::Uuid::new_v4().to_string();
        }
        let json = serde_json::to_string(&config).map_err(|e| Error::Other(e.to_string()))?;
        self.lock()
            .execute(
                "INSERT INTO connections (id, position, config)
                 VALUES (?1, (SELECT COALESCE(MAX(position), 0) + 1 FROM connections), ?2)
                 ON CONFLICT(id) DO UPDATE SET config = excluded.config",
                params![config.id, json],
            )
            .map_err(db_err)?;
        Ok(config)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.lock()
            .execute("DELETE FROM connections WHERE id = ?1", params![id])
            .map_err(db_err)?;
        Ok(())
    }
}

/// One statement that was run, for the History panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub connection_id: String,
    pub sql: String,
    /// Unix time in milliseconds.
    pub started_at: i64,
    pub elapsed_ms: i64,
    pub rows: Option<i64>,
    pub error: Option<String>,
}

impl AppDb {
    pub fn history_add(
        &self,
        connection_id: &str,
        sql: &str,
        started_at: i64,
        elapsed_ms: i64,
        rows: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.lock()
            .execute(
                "INSERT INTO history (connection_id, sql, started_at, elapsed_ms, rows, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![connection_id, sql, started_at, elapsed_ms, rows, error],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Newest first. `connection_id` and `search` narrow the list when given;
    /// every word of `search` must appear in the statement.
    pub fn history_search(
        &self,
        connection_id: Option<&str>,
        search: &str,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>> {
        let mut sql = String::from(
            "SELECT id, connection_id, sql, started_at, elapsed_ms, rows, error FROM history WHERE 1 = 1",
        );
        let mut values: Vec<String> = Vec::new();
        if let Some(id) = connection_id {
            values.push(id.to_string());
            sql.push_str(&format!(" AND connection_id = ?{}", values.len()));
        }
        for word in search.split_whitespace() {
            // LIKE with its wildcards escaped: the user is searching for text.
            let escaped = word
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            values.push(format!("%{escaped}%"));
            sql.push_str(&format!(" AND sql LIKE ?{} ESCAPE '\\'", values.len()));
        }
        sql.push_str(&format!(" ORDER BY id DESC LIMIT {}", limit.min(1000)));
        let conn = self.lock();
        let mut stmt = conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(values.iter()), |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    connection_id: row.get(1)?,
                    sql: row.get(2)?,
                    started_at: row.get(3)?,
                    elapsed_ms: row.get(4)?,
                    rows: row.get(5)?,
                    error: row.get(6)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db_err)
    }

    pub fn history_clear(&self) -> Result<()> {
        self.lock()
            .execute("DELETE FROM history", [])
            .map_err(db_err)?;
        Ok(())
    }

    /// Deletes entries older than the retention period.
    pub fn history_prune(&self, now_ms: i64, retention_days: u32) -> Result<()> {
        let cutoff = now_ms - i64::from(retention_days) * 86_400_000;
        self.lock()
            .execute("DELETE FROM history WHERE started_at < ?1", params![cutoff])
            .map_err(db_err)?;
        Ok(())
    }

    /// Stores a named piece of session state (open tabs and their text).
    pub fn session_save(&self, key: &str, value: &str) -> Result<()> {
        self.lock()
            .execute(
                "INSERT INTO session (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn session_load(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.lock()
            .query_row(
                "SELECT value FROM session WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)
    }
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::Other(format!("Rowboat's own database failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pg(name: &str) -> ConnectionConfig {
        ConnectionConfig {
            name: name.into(),
            user: "app".into(),
            ..Default::default()
        }
    }

    #[test]
    fn save_assigns_an_id_and_lists_in_creation_order() {
        let db = AppDb::in_memory().unwrap();
        let a = db.save(pg("a")).unwrap();
        let b = db.save(pg("b")).unwrap();
        assert!(!a.id.is_empty() && a.id != b.id);
        let names: Vec<_> = db.list().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn saving_again_updates_in_place() {
        let db = AppDb::in_memory().unwrap();
        let mut a = db.save(pg("a")).unwrap();
        db.save(pg("b")).unwrap();
        a.name = "renamed".into();
        a.environment = Environment::Prod;
        db.save(a.clone()).unwrap();
        let all = db.list().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], a, "position is kept");
        assert!(db.get(&a.id).unwrap().is_prod());
    }

    #[test]
    fn delete_removes_only_that_connection() {
        let db = AppDb::in_memory().unwrap();
        let a = db.save(pg("a")).unwrap();
        db.save(pg("b")).unwrap();
        db.delete(&a.id).unwrap();
        assert_eq!(db.list().unwrap().len(), 1);
        assert!(db.get(&a.id).is_err());
    }

    #[test]
    fn invalid_configs_are_rejected_with_a_reason() {
        let db = AppDb::in_memory().unwrap();
        assert!(db.save(pg("  ")).is_err());
        let mut no_host = pg("x");
        no_host.host = " ".into();
        assert!(db.save(no_host).is_err());
        let sqlite = ConnectionConfig {
            name: "s".into(),
            kind: DbKind::Sqlite,
            ..Default::default()
        };
        assert!(db.save(sqlite).is_err(), "SQLite needs a path");
    }

    #[test]
    fn stored_config_has_no_secret_fields() {
        let json = serde_json::to_string(&ConnectionConfig::default())
            .unwrap()
            .to_lowercase();
        for needle in ["password", "passphrase", "secret"] {
            assert!(!json.contains(needle), "config must not contain `{needle}`");
        }
    }

    #[test]
    fn history_is_newest_first_and_searchable() {
        let db = AppDb::in_memory().unwrap();
        db.history_add("a", "select * from orders", 1000, 12, Some(50), None)
            .unwrap();
        db.history_add("b", "select * from customers", 2000, 5, Some(3), None)
            .unwrap();
        db.history_add(
            "a",
            "delete from orders where id = 100%_x",
            3000,
            7,
            None,
            Some("boom"),
        )
        .unwrap();

        let all = db.history_search(None, "", 100).unwrap();
        assert_eq!(
            all.iter().map(|h| h.started_at).collect::<Vec<_>>(),
            [3000, 2000, 1000]
        );
        assert_eq!(all[0].error.as_deref(), Some("boom"));

        let only_a = db.history_search(Some("a"), "", 100).unwrap();
        assert_eq!(only_a.len(), 2);
        // Every word must match; case does not matter.
        assert_eq!(
            db.history_search(None, "ORDERS select", 100).unwrap().len(),
            1
        );
        // % and _ are searched for literally, not as wildcards.
        assert_eq!(db.history_search(None, "100%_x", 100).unwrap().len(), 1);
        assert_eq!(db.history_search(None, "%", 100).unwrap().len(), 1);
        assert_eq!(db.history_search(None, "", 2).unwrap().len(), 2);
    }

    #[test]
    fn history_prunes_by_age_and_clears() {
        let db = AppDb::in_memory().unwrap();
        let day = 86_400_000;
        db.history_add("a", "old", 0, 1, None, None).unwrap();
        db.history_add("a", "recent", 9 * day, 1, None, None)
            .unwrap();
        db.history_prune(10 * day, 5).unwrap();
        let left = db.history_search(None, "", 10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].sql, "recent");
        db.history_clear().unwrap();
        assert!(db.history_search(None, "", 10).unwrap().is_empty());
    }

    #[test]
    fn session_state_is_replaced_not_appended() {
        let db = AppDb::in_memory().unwrap();
        assert_eq!(db.session_load("tabs").unwrap(), None);
        db.session_save("tabs", "[1]").unwrap();
        db.session_save("tabs", "[1,2]").unwrap();
        assert_eq!(db.session_load("tabs").unwrap().as_deref(), Some("[1,2]"));
    }

    #[test]
    fn connections_survive_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rowboat.db");
        let saved = AppDb::open(path.clone()).unwrap().save(pg("a")).unwrap();
        assert_eq!(AppDb::open(path).unwrap().list().unwrap(), vec![saved]);
    }
}

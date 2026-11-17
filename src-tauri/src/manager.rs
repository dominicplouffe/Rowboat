//! Ties saved connections, secrets, tunnels and open sessions together. This
//! is the layer the IPC commands call.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::connections::{AppDb, ConnectionConfig, HistoryEntry, SshAuth};
use crate::db::{
    self, ConnectParams, DbError, DbKind, DbResult, EditTarget, ErrorKind, ExecOptions,
    QueryResult, SchemaDetails, SchemaNode, SchemaSnapshot, Session,
};
use crate::edits::{update_statements, RowEdit};
use crate::export::{self, Format};
use crate::secrets::{SecretKind, Secrets};
use crate::settings::{ConfirmDestructive, Settings};
use crate::ssh::{default_known_hosts_path, Tunnel, TunnelParams};
use crate::statements::{self, destructive_reason};

/// Secrets as typed by the user. `None` leaves the stored secret alone, an
/// empty string deletes it. This type only ever travels *into* the core.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecretInput {
    pub password: Option<String>,
    pub ssh_password: Option<String>,
    pub ssh_passphrase: Option<String>,
}

impl SecretInput {
    fn get(&self, kind: SecretKind) -> &Option<String> {
        match kind {
            SecretKind::Password => &self.password,
            SecretKind::SshPassword => &self.ssh_password,
            SecretKind::SshPassphrase => &self.ssh_passphrase,
        }
    }
}

/// What the UI sees of a connection: its configuration and *whether* secrets
/// exist, never the secrets themselves.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    #[serde(flatten)]
    pub config: ConnectionConfig,
    pub has_password: bool,
    pub has_ssh_password: bool,
    pub has_ssh_passphrase: bool,
    pub connected: bool,
    pub server_version: Option<String>,
    pub supports_transactions: bool,
    pub in_transaction: bool,
    /// This connection's mode; starts from the "Auto-commit by default" setting.
    pub auto_commit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    #[serde(flatten)]
    pub result: QueryResult,
    pub elapsed_ms: u64,
    /// False for a statement that changes data: it is never run a second
    /// time to fetch more rows, count them or export them.
    pub repeatable: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RunOptions {
    /// Rows to fetch; 0 means no limit.
    pub row_limit: u32,
    /// The user has confirmed a statement the Prod guard flagged.
    pub confirmed: bool,
    /// Fetch past the result memory cap.
    pub uncapped: bool,
    /// This run repeats a statement to fetch more of its rows.
    pub refetch: bool,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

struct Open {
    session: Box<dyn Session>,
    /// Kept alive for as long as the session uses it.
    _tunnel: Option<Tunnel>,
    config: ConnectionConfig,
    /// Statements on one connection run one at a time. Waiting here, rather
    /// than inside the driver, is what lets a queued statement be cancelled
    /// without touching the one that is running.
    turn: tokio::sync::Mutex<()>,
    auto_commit: AtomicBool,
}

/// A statement that has been submitted and not yet finished.
struct Pending {
    connection_id: String,
    /// True once it holds the connection's turn and is with the database.
    executing: bool,
    cancelled: Arc<AtomicBool>,
    wake: Arc<tokio::sync::Notify>,
}

pub struct Manager {
    db: AppDb,
    secrets: Secrets,
    sessions: tokio::sync::Mutex<HashMap<String, Arc<Open>>>,
    /// Submitted statements by query id, for cancel.
    running: Mutex<HashMap<String, Pending>>,
}

impl Manager {
    pub fn new(db: AppDb, secrets: Secrets) -> Self {
        Self {
            db,
            secrets,
            sessions: Default::default(),
            running: Default::default(),
        }
    }

    pub fn secrets_are_persistent(&self) -> bool {
        self.secrets.is_persistent()
    }

    async fn view(&self, config: ConnectionConfig) -> ConnectionView {
        let open = self.sessions.lock().await.get(&config.id).cloned();
        ConnectionView {
            has_password: self.secrets.has(&config.id, SecretKind::Password),
            has_ssh_password: self.secrets.has(&config.id, SecretKind::SshPassword),
            has_ssh_passphrase: self.secrets.has(&config.id, SecretKind::SshPassphrase),
            connected: open.is_some(),
            server_version: open.as_ref().map(|o| o.session.server_version()),
            supports_transactions: open
                .as_ref()
                .map_or(config.kind != DbKind::Clickhouse, |o| {
                    o.session.supports_transactions()
                }),
            in_transaction: open.as_ref().is_some_and(|o| o.session.in_transaction()),
            auto_commit: open
                .as_ref()
                .is_none_or(|o| o.auto_commit.load(Ordering::SeqCst)),
            config,
        }
    }

    pub async fn list(&self) -> DbResult<Vec<ConnectionView>> {
        let mut out = Vec::new();
        for config in self.db.list().map_err(internal)? {
            out.push(self.view(config).await);
        }
        Ok(out)
    }

    pub async fn save(
        &self,
        config: ConnectionConfig,
        secrets: SecretInput,
    ) -> DbResult<ConnectionView> {
        let existed = !config.id.is_empty();
        let config = self.db.save(config).map_err(internal)?;
        for kind in SecretKind::ALL {
            if let Some(value) = secrets.get(kind) {
                self.secrets.set(&config.id, kind, value).map_err(|e| {
                    DbError::other(format!(
                        "Could not store the secret in the system keychain: {e}"
                    ))
                })?;
            }
        }
        // An edited connection must be reopened to pick the changes up.
        if existed {
            self.close(&config.id).await;
        }
        Ok(self.view(config).await)
    }

    pub async fn delete(&self, id: &str) -> DbResult<()> {
        self.close(id).await;
        self.secrets.forget(id);
        self.db.delete(id).map_err(internal)
    }

    /// The secret to use: the one just typed, else the stored one.
    fn secret(&self, id: &str, kind: SecretKind, typed: &SecretInput) -> DbResult<Option<String>> {
        match typed.get(kind) {
            Some(value) => Ok(Some(value.clone()).filter(|v| !v.is_empty())),
            None if id.is_empty() => Ok(None),
            None => self.secrets.get(id, kind).map_err(|e| {
                DbError::other(format!("Could not read from the system keychain: {e}"))
            }),
        }
    }

    async fn connect(
        &self,
        config: &ConnectionConfig,
        typed: &SecretInput,
        settings: &Settings,
        trusted_fingerprint: Option<String>,
    ) -> DbResult<Open> {
        let connect_timeout = Duration::from_secs(settings.connections.connect_timeout_secs as u64);
        let keepalive = Duration::from_secs(settings.connections.keepalive_secs as u64);

        let tunnel = if config.ssh.enabled && config.kind != DbKind::Sqlite {
            Some(
                Tunnel::open(TunnelParams {
                    ssh: config.ssh.clone(),
                    password: match config.ssh.auth {
                        SshAuth::Password => {
                            self.secret(&config.id, SecretKind::SshPassword, typed)?
                        }
                        _ => None,
                    },
                    passphrase: match config.ssh.auth {
                        SshAuth::Key => {
                            self.secret(&config.id, SecretKind::SshPassphrase, typed)?
                        }
                        _ => None,
                    },
                    target_host: config.host.clone(),
                    target_port: config.port,
                    policy: settings.connections.ssh_known_hosts,
                    trusted_fingerprint,
                    connect_timeout,
                    keepalive,
                    known_hosts_path: default_known_hosts_path(),
                })
                .await?,
            )
        } else {
            None
        };

        let session = db::connect(ConnectParams {
            config: config.clone(),
            password: self.secret(&config.id, SecretKind::Password, typed)?,
            client_id: settings.general.client_id.clone(),
            connect_timeout,
            keepalive,
            auto_commit: settings.execution.auto_commit,
            auto_reconnect: settings.connections.auto_reconnect,
            via: tunnel.as_ref().map(|t| t.local),
        })
        .await?;
        Ok(Open {
            session,
            _tunnel: tunnel,
            config: config.clone(),
            turn: tokio::sync::Mutex::new(()),
            auto_commit: AtomicBool::new(settings.execution.auto_commit),
        })
    }

    /// Tries a configuration (saved or not) and reports the server version.
    pub async fn test(
        &self,
        config: ConnectionConfig,
        typed: SecretInput,
        settings: &Settings,
        trusted_fingerprint: Option<String>,
    ) -> DbResult<String> {
        let mut config = config.validate().map_err(internal)?;
        // Testing must never create the file it is testing for.
        config.create = false;
        let open = self
            .connect(&config, &typed, settings, trusted_fingerprint)
            .await?;
        let version = open.session.server_version();
        open.session.close().await;
        Ok(version)
    }

    pub async fn open(
        &self,
        id: &str,
        settings: &Settings,
        trusted_fingerprint: Option<String>,
    ) -> DbResult<ConnectionView> {
        let config = self.db.get(id).map_err(internal)?;
        if self.sessions.lock().await.contains_key(id) {
            return Ok(self.view(config).await);
        }
        let open = self
            .connect(
                &config,
                &SecretInput::default(),
                settings,
                trusted_fingerprint,
            )
            .await?;
        // A file created by connecting exists from now on.
        if config.create {
            let _ = self.db.save(ConnectionConfig {
                create: false,
                ..config.clone()
            });
        }
        self.sessions
            .lock()
            .await
            .insert(id.to_string(), Arc::new(open));
        Ok(self.view(self.db.get(id).map_err(internal)?).await)
    }

    pub async fn close(&self, id: &str) {
        if let Some(open) = self.sessions.lock().await.remove(id) {
            open.session.cancel().await;
            open.session.close().await;
        }
    }

    async fn session(&self, id: &str) -> DbResult<Arc<Open>> {
        self.sessions
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| DbError::connection("This connection is not open."))
    }

    /// Runs `work` when it is this statement's turn on its connection.
    ///
    /// A cancel while waiting ends the statement before anything reaches the
    /// database; a cancel or timeout while running stops it there. The timeout
    /// (0 = none) counts from when the work starts, not while it queued.
    async fn with_turn<'a, T>(
        &self,
        open: &'a Arc<Open>,
        query_id: &str,
        timeout_secs: u32,
        work: impl FnOnce(&'a dyn Session) -> futures_util::future::BoxFuture<'a, DbResult<T>>,
    ) -> DbResult<T> {
        let (cancelled, wake) = {
            let entry = Pending {
                connection_id: open.config.id.clone(),
                executing: false,
                cancelled: Arc::new(AtomicBool::new(false)),
                wake: Arc::new(tokio::sync::Notify::new()),
            };
            let handles = (entry.cancelled.clone(), entry.wake.clone());
            self.pending().insert(query_id.to_string(), entry);
            handles
        };
        // Whatever happens below, the statement stops being pending.
        let _done = Finished(self, query_id);

        let _turn = tokio::select! {
            turn = open.turn.lock() => turn,
            () = wake.notified() => return Err(DbError::cancelled()),
        };
        if cancelled.load(Ordering::SeqCst) {
            return Err(DbError::cancelled());
        }
        if let Some(entry) = self.pending().get_mut(query_id) {
            entry.executing = true;
        }
        open.session
            .set_auto_commit(open.auto_commit.load(Ordering::SeqCst));

        let running = work(open.session.as_ref());
        if timeout_secs == 0 {
            return running.await;
        }
        match tokio::time::timeout(Duration::from_secs(timeout_secs as u64), running).await {
            Ok(result) => result,
            Err(_) => {
                open.session.cancel().await;
                Err(DbError::new(
                    ErrorKind::Timeout,
                    format!("The query was cancelled after the {timeout_secs} s timeout."),
                )
                .with_hint("Change the query timeout in Settings → Execution."))
            }
        }
    }

    /// Runs one statement with the default options. See [`Manager::run_with`].
    pub async fn run(
        &self,
        connection_id: &str,
        query_id: &str,
        sql: &str,
        row_limit: u32,
        confirmed: bool,
        settings: &Settings,
    ) -> DbResult<RunResult> {
        let options = RunOptions {
            row_limit,
            confirmed,
            uncapped: false,
            refetch: false,
        };
        self.run_with(connection_id, query_id, sql, options, settings)
            .await
    }

    /// Runs one statement.
    ///
    /// `confirmed` says the user has already agreed to run a statement the
    /// Prod guard flagged; without it such a statement is refused with
    /// [`ErrorKind::NeedsConfirmation`] and nothing is sent to the database.
    pub async fn run_with(
        &self,
        connection_id: &str,
        query_id: &str,
        sql: &str,
        options: RunOptions,
        settings: &Settings,
    ) -> DbResult<RunResult> {
        let open = self.session(connection_id).await?;
        let repeatable = statements::repeatable(open.config.kind, sql);
        if options.refetch && !repeatable {
            return Err(not_repeatable());
        }

        let guarded = match settings.execution.confirm_destructive {
            ConfirmDestructive::Always => true,
            ConfirmDestructive::ProdOnly => open.config.is_prod(),
            ConfirmDestructive::Never => false,
        };
        if guarded && !options.confirmed {
            if let Some(reason) = destructive_reason(open.config.kind, sql) {
                return Err(DbError::new(ErrorKind::NeedsConfirmation, reason));
            }
        }

        let exec = ExecOptions {
            row_limit: options.row_limit,
            // The result memory cap protects the app; "Fetch all anyway"
            // lifts it for one run.
            max_bytes: if options.uncapped {
                0
            } else {
                u64::from(settings.execution.result_memory_cap_mb) * 1024 * 1024
            },
        };
        let started_at = now_ms();
        let started = Instant::now();
        let result = self
            .with_turn(
                &open,
                query_id,
                settings.execution.query_timeout_secs,
                |session| Box::pin(session.execute(sql, exec)),
            )
            .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        if settings.privacy.keep_history {
            let (rows, error) = match &result {
                Ok(r) if r.columns.is_empty() => (r.rows_affected.map(|n| n as i64), None),
                Ok(r) => (Some(r.rows.len() as i64), None),
                Err(e) => (None, Some(e.message.as_str())),
            };
            // History is a convenience; failing to record it must not fail the query.
            let _ = self.db.history_add(
                connection_id,
                sql,
                started_at,
                elapsed_ms as i64,
                rows,
                error,
            );
        }
        Ok(RunResult {
            result: result?,
            elapsed_ms,
            repeatable,
        })
    }

    /// Counts the rows a query returns, without fetching them.
    pub async fn count(
        &self,
        connection_id: &str,
        query_id: &str,
        sql: &str,
        settings: &Settings,
    ) -> DbResult<u64> {
        let open = self.session(connection_id).await?;
        if !statements::repeatable(open.config.kind, sql) {
            return Err(not_repeatable());
        }
        // A separate statement that wraps the user's query; their own SQL is
        // not changed and this is never what Run sends.
        let body = sql.trim().trim_end_matches(';').trim_end();
        let counting = format!("SELECT count(*) FROM (\n{body}\n) AS rowboat_count");
        let result = self
            .with_turn(
                &open,
                query_id,
                settings.execution.query_timeout_secs,
                |session| Box::pin(session.execute(&counting, ExecOptions::limit(1))),
            )
            .await
            .map_err(|mut error| {
                if matches!(error.kind, ErrorKind::Syntax) {
                    error.hint =
                        Some("Only queries that return rows (SELECT) can be counted.".into());
                    error.position = None;
                }
                error
            })?;
        result
            .rows
            .first()
            .and_then(|row| row.first().cloned().flatten())
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| DbError::other("The database did not return a count."))
    }

    /// Everything autocomplete needs to know about a connection's schema.
    pub async fn snapshot(&self, connection_id: &str) -> DbResult<SchemaSnapshot> {
        self.session(connection_id).await?.session.snapshot().await
    }

    /// Runs `sql` and streams the whole result into `path`, never holding it
    /// in memory. Returns the number of rows written.
    pub async fn export_query(
        &self,
        connection_id: &str,
        query_id: &str,
        sql: &str,
        path: &std::path::Path,
        format: Format,
        table: &str,
    ) -> DbResult<u64> {
        let open = self.session(connection_id).await?;
        if !statements::repeatable(open.config.kind, sql) {
            return Err(not_repeatable());
        }
        // Written next to the target and renamed when complete, so a failed
        // or cancelled export never destroys a file it was going to replace.
        let mut partial = path.as_os_str().to_owned();
        partial.push(".partial");
        let partial = std::path::PathBuf::from(partial);
        let file = std::fs::File::create(&partial)
            .map_err(|e| DbError::other(format!("Could not create {}: {e}", path.display())))?;
        let writer = export::Writer::new(
            std::io::BufWriter::with_capacity(1 << 16, file),
            export::Options {
                format,
                kind: open.config.kind,
                table: table.to_string(),
            },
        );
        let result = self
            .with_turn(&open, query_id, 0, |session| {
                Box::pin(session.export(sql, Box::new(writer)))
            })
            .await;
        let result = result.and_then(|rows| {
            std::fs::rename(&partial, path)
                .map(|()| rows)
                .map_err(|e| DbError::other(format!("Could not write {}: {e}", path.display())))
        });
        if result.is_err() {
            // Do not leave half a file behind.
            let _ = std::fs::remove_file(&partial);
        }
        result
    }

    /// The exact statements Apply will run for these edits.
    pub async fn edits_preview(
        &self,
        connection_id: &str,
        target: &EditTarget,
        edits: &[RowEdit],
    ) -> DbResult<Vec<String>> {
        let open = self.session(connection_id).await?;
        update_statements(open.config.kind, target, edits)
    }

    /// Applies staged edits in one transaction: either every row is updated
    /// or none is. Each statement must change exactly one row.
    pub async fn edits_apply(
        &self,
        connection_id: &str,
        query_id: &str,
        target: &EditTarget,
        edits: &[RowEdit],
    ) -> DbResult<u64> {
        let open = self.session(connection_id).await?;
        if open.config.read_only {
            return Err(DbError::new(
                ErrorKind::Permission,
                "This connection is read-only.",
            ));
        }
        let statements = update_statements(open.config.kind, target, edits)?;
        let statements = &statements;
        self.with_turn(&open, query_id, 0, |session| {
            Box::pin(async move {
                let all = ExecOptions::limit(0);
                // Inside the user's own transaction use a savepoint, so a failure
                // undoes these edits only; otherwise use a transaction of our own.
                let nested = session.in_transaction();
                let (begin, commit, abort) = if nested {
                    (
                        "SAVEPOINT rowboat_edit",
                        "RELEASE SAVEPOINT rowboat_edit",
                        "ROLLBACK TO SAVEPOINT rowboat_edit",
                    )
                } else {
                    ("BEGIN", "COMMIT", "ROLLBACK")
                };
                session.execute(begin, all).await?;
                for statement in statements {
                    let outcome = match session.execute(statement, all).await {
                        Ok(result) if result.rows_affected == Some(1) => Ok(()),
                        Ok(result) => Err(DbError::other(format!(
                        "An edit would have changed {} rows instead of 1, so nothing was saved. \
                         The row may have been changed or deleted since it was loaded.",
                        result.rows_affected.unwrap_or(0)
                    ))
                        .with_detail(statement.clone())),
                        Err(error) => Err(error.with_detail(statement.clone())),
                    };
                    if let Err(error) = outcome {
                        let _ = session.execute(abort, all).await;
                        return Err(error);
                    }
                }
                if let Err(error) = session.execute(commit, all).await {
                    // A commit that fails (a deferred constraint, say) must
                    // not leave the session inside a half-finished transaction.
                    let _ = session.execute(abort, all).await;
                    return Err(error);
                }
                Ok(statements.len() as u64)
            })
        })
        .await
    }

    pub fn history(
        &self,
        connection_id: Option<&str>,
        search: &str,
        limit: u32,
    ) -> DbResult<Vec<HistoryEntry>> {
        self.db
            .history_search(connection_id, search, limit)
            .map_err(internal)
    }

    // ------------------------------------------------------------------ AI --

    pub fn ai_key(&self, provider_id: &str) -> Result<Option<String>, String> {
        self.secrets.ai_key(provider_id)
    }

    pub fn set_ai_key(&self, provider_id: &str, key: &str) -> Result<(), String> {
        self.secrets.set_ai_key(provider_id, key)
    }

    pub fn config(&self, connection_id: &str) -> DbResult<ConnectionConfig> {
        self.db.get(connection_id).map_err(internal)
    }

    pub async fn is_open(&self, connection_id: &str) -> bool {
        self.sessions.lock().await.contains_key(connection_id)
    }

    /// The catalog facts the AI schema index adds to the snapshot.
    pub async fn details(&self, connection_id: &str, samples: bool) -> DbResult<SchemaDetails> {
        self.session(connection_id)
            .await?
            .session
            .details(samples)
            .await
    }

    pub fn history_clear(&self) -> DbResult<()> {
        self.db.history_clear().map_err(internal)
    }

    /// Drops history older than the retention setting.
    pub fn history_prune(&self, settings: &Settings) {
        let _ = self
            .db
            .history_prune(now_ms(), settings.privacy.history_retention_days);
    }

    pub fn session_save(&self, key: &str, value: &str) -> DbResult<()> {
        self.db.session_save(key, value).map_err(internal)
    }

    pub fn session_load(&self, key: &str) -> DbResult<Option<String>> {
        self.db.session_load(key).map_err(internal)
    }

    pub fn session_delete(&self, key: &str) -> DbResult<()> {
        self.db.session_delete(key).map_err(internal)
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<String, Pending>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Cancels the statement that was started with `query_id`, and only that
    /// one: a queued statement is dropped before it runs; a running one is
    /// cancelled in the database.
    pub async fn cancel(&self, query_id: &str) {
        let target = {
            let pending = self.pending();
            let Some(entry) = pending.get(query_id) else {
                return;
            };
            if !entry.executing {
                entry.cancelled.store(true, Ordering::SeqCst);
                entry.wake.notify_one();
                return;
            }
            entry.connection_id.clone()
        };
        if let Ok(open) = self.session(&target).await {
            open.session.cancel().await;
        }
    }

    /// Switches one open connection between auto-commit and manual commit.
    pub async fn set_auto_commit(&self, connection_id: &str, auto_commit: bool) -> DbResult<()> {
        let open = self.session(connection_id).await?;
        open.auto_commit.store(auto_commit, Ordering::SeqCst);
        Ok(())
    }

    pub async fn end_transaction(&self, connection_id: &str, commit: bool) -> DbResult<()> {
        let open = self.session(connection_id).await?;
        if commit {
            open.session.commit().await
        } else {
            open.session.rollback().await
        }
    }

    pub async fn children(
        &self,
        connection_id: &str,
        path: &[String],
    ) -> DbResult<Vec<SchemaNode>> {
        self.session(connection_id)
            .await?
            .session
            .children(path)
            .await
    }

    pub async fn ddl(&self, connection_id: &str, path: &[String]) -> DbResult<String> {
        self.session(connection_id).await?.session.ddl(path).await
    }
}

/// Removes a statement from the pending map when `run` returns, on any path.
struct Finished<'a>(&'a Manager, &'a str);

impl Drop for Finished<'_> {
    fn drop(&mut self) {
        self.0.pending().remove(self.1);
    }
}

fn not_repeatable() -> DbError {
    DbError::other(
        "This statement changes data, so Rowboat will not run it a second time to fetch, count or export its rows.",
    )
    .with_hint("Run a SELECT to read the rows.")
}

fn internal(error: crate::error::Error) -> DbError {
    DbError::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::Environment;

    fn manager() -> Manager {
        Manager::new(AppDb::in_memory().unwrap(), Secrets::in_memory())
    }

    fn sqlite(dir: &tempfile::TempDir, name: &str) -> ConnectionConfig {
        ConnectionConfig {
            name: name.into(),
            kind: DbKind::Sqlite,
            path: dir.path().join(format!("{name}.db")).display().to_string(),
            create: true,
            ..Default::default()
        }
    }

    async fn run(
        m: &Manager,
        id: &str,
        sql: &str,
        confirmed: bool,
        settings: &Settings,
    ) -> DbResult<RunResult> {
        m.run(
            id,
            &uuid::Uuid::new_v4().to_string(),
            sql,
            1000,
            confirmed,
            settings,
        )
        .await
    }

    #[tokio::test]
    async fn the_ui_learns_that_a_secret_exists_but_never_its_value() {
        let m = manager();
        let saved = m
            .save(
                ConnectionConfig {
                    name: "pg".into(),
                    user: "app".into(),
                    ..Default::default()
                },
                SecretInput {
                    password: Some("hunter2".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(saved.has_password && !saved.has_ssh_password);
        let json = serde_json::to_string(&m.list().await.unwrap()).unwrap();
        assert!(!json.contains("hunter2"), "{json}");

        // Saving again without typing a password keeps it; an empty one removes it.
        let kept = m
            .save(saved.config.clone(), SecretInput::default())
            .await
            .unwrap();
        assert!(kept.has_password);
        let cleared = m
            .save(
                saved.config.clone(),
                SecretInput {
                    password: Some(String::new()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!cleared.has_password);
    }

    #[tokio::test]
    async fn deleting_a_connection_forgets_its_secrets() {
        let m = manager();
        let saved = m
            .save(
                ConnectionConfig {
                    name: "pg".into(),
                    ..Default::default()
                },
                SecretInput {
                    password: Some("pw".into()),
                    ssh_passphrase: Some("phrase".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        m.delete(&saved.config.id).await.unwrap();
        assert!(m.list().await.unwrap().is_empty());
        assert!(!m.secrets.has(&saved.config.id, SecretKind::Password));
        assert!(!m.secrets.has(&saved.config.id, SecretKind::SshPassphrase));
    }

    #[tokio::test]
    async fn open_run_and_close_a_connection() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = m
            .save(sqlite(&dir, "a"), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;

        let closed = run(&m, &id, "SELECT 1", false, &settings)
            .await
            .unwrap_err();
        assert_eq!(closed.kind, ErrorKind::Connection);

        let view = m.open(&id, &settings, None).await.unwrap();
        assert!(view.connected);
        assert!(view.server_version.unwrap().starts_with("SQLite"));
        assert!(!view.config.create, "the file exists now");

        let result = run(&m, &id, "SELECT 40 + 2 AS answer", false, &settings)
            .await
            .unwrap();
        assert_eq!(result.result.rows[0][0].as_deref(), Some("42"));

        m.close(&id).await;
        assert!(!m.list().await.unwrap()[0].connected);
    }

    #[tokio::test]
    async fn testing_a_connection_does_not_create_the_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let config = sqlite(&dir, "ghost");
        let error = m
            .test(
                config.clone(),
                SecretInput::default(),
                &Settings::default(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Connection);
        assert!(!std::path::Path::new(&config.path).exists());
    }

    #[tokio::test]
    async fn prod_guard_refuses_destructive_statements_until_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default(); // confirm on Prod only
        let mut prod = sqlite(&dir, "prod");
        prod.environment = Environment::Prod;
        let prod = m
            .save(prod, SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        let dev = m
            .save(sqlite(&dir, "dev"), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        for id in [&prod, &dev] {
            m.open(id, &settings, None).await.unwrap();
            run(&m, id, "CREATE TABLE t (id INTEGER)", false, &settings)
                .await
                .unwrap();
            run(&m, id, "INSERT INTO t VALUES (1), (2)", false, &settings)
                .await
                .unwrap();
        }
        let count = |r: RunResult| r.result.rows[0][0].clone().unwrap();

        // On Prod: refused, and nothing was deleted.
        let refused = run(&m, &prod, "DELETE FROM t", false, &settings)
            .await
            .unwrap_err();
        assert_eq!(refused.kind, ErrorKind::NeedsConfirmation);
        assert!(refused.message.contains("every row"));
        assert_eq!(
            count(
                run(&m, &prod, "SELECT count(*) FROM t", false, &settings)
                    .await
                    .unwrap()
            ),
            "2"
        );

        // A WHERE clause makes it an ordinary statement.
        run(&m, &prod, "DELETE FROM t WHERE id = 1", false, &settings)
            .await
            .unwrap();

        // Confirmed: runs.
        run(&m, &prod, "DELETE FROM t", true, &settings)
            .await
            .unwrap();
        assert_eq!(
            count(
                run(&m, &prod, "SELECT count(*) FROM t", false, &settings)
                    .await
                    .unwrap()
            ),
            "0"
        );

        // On Dev the default setting does not ask.
        run(&m, &dev, "DELETE FROM t", false, &settings)
            .await
            .unwrap();

        // "Always" asks everywhere; "Never" asks nowhere.
        let mut always = Settings::default();
        always.execution.confirm_destructive = ConfirmDestructive::Always;
        assert_eq!(
            run(&m, &dev, "DROP TABLE t", false, &always)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::NeedsConfirmation
        );
        let mut never = Settings::default();
        never.execution.confirm_destructive = ConfirmDestructive::Never;
        run(&m, &prod, "DROP TABLE t", false, &never).await.unwrap();
    }

    #[tokio::test]
    async fn the_query_timeout_cancels_the_statement() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let mut settings = Settings::default();
        settings.execution.query_timeout_secs = 1;
        let id = m
            .save(sqlite(&dir, "slow"), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        m.open(&id, &settings, None).await.unwrap();

        let started = Instant::now();
        let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
        let error = run(&m, &id, endless, false, &settings).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Timeout);
        assert!(started.elapsed() < Duration::from_secs(3));
        // The session is free again.
        run(&m, &id, "SELECT 1", false, &settings).await.unwrap();
    }

    #[tokio::test]
    async fn cancel_by_query_id_stops_the_right_statement() {
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(manager());
        let settings = Settings::default();
        let id = m
            .save(sqlite(&dir, "c"), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        m.open(&id, &settings, None).await.unwrap();

        let (runner, connection) = (m.clone(), id.clone());
        let task = tokio::spawn(async move {
            let endless = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";
            runner
                .run(
                    &connection,
                    "query-1",
                    endless,
                    1000,
                    false,
                    &Settings::default(),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        m.cancel("some-other-query").await; // unknown id: nothing happens
        assert!(!task.is_finished());
        m.cancel("query-1").await;
        assert_eq!(task.await.unwrap().unwrap_err().kind, ErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn count_rows_counts_without_fetching() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = opened(&m, &dir, "count", &settings).await;
        let big = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 250000) SELECT i FROM n;";
        assert_eq!(m.count(&id, "c1", big, &settings).await.unwrap(), 250_000);
        // Something that is not a query cannot be counted, and says why.
        let error = m
            .count(&id, "c2", "CREATE TABLE t (a)", &settings)
            .await
            .unwrap_err();
        assert!(error.hint.unwrap().contains("SELECT"));
    }

    #[tokio::test]
    async fn the_memory_cap_stops_fetch_all_unless_lifted() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let mut settings = Settings::default();
        settings.execution.result_memory_cap_mb = 16; // the smallest allowed
        let id = opened(&m, &dir, "cap", &settings).await;
        // ~1.2 million rows of ~40 bytes: well over 16 MB.
        let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1200000) SELECT i, 'padding padding' FROM n";
        let capped = RunOptions {
            row_limit: 0,
            ..Default::default()
        };
        let r = m
            .run_with(&id, "q1", sql, capped, &settings)
            .await
            .unwrap()
            .result;
        assert!(r.stopped_at_memory_cap && r.more_available);
        assert!(r.rows.len() < 1_200_000);

        let lifted = RunOptions {
            row_limit: 0,
            uncapped: true,
            ..Default::default()
        };
        let r = m
            .run_with(&id, "q2", sql, lifted, &settings)
            .await
            .unwrap()
            .result;
        assert_eq!(r.rows.len(), 1_200_000);
        assert!(!r.stopped_at_memory_cap);
    }

    #[tokio::test]
    async fn edits_apply_together_or_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = opened(&m, &dir, "edit", &settings).await;
        run(
            &m,
            &id,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, qty INTEGER CHECK (qty >= 0))",
            false,
            &settings,
        )
        .await
        .unwrap();
        run(
            &m,
            &id,
            "INSERT INTO t VALUES (1, 'a', 1), (2, 'b', 2)",
            false,
            &settings,
        )
        .await
        .unwrap();
        let target = run(&m, &id, "SELECT * FROM t", false, &settings)
            .await
            .unwrap()
            .result
            .editable
            .unwrap();
        let edit = |key: &str, column: &str, value: Option<&str>| RowEdit {
            key: vec![Some(key.into())],
            changes: vec![(column.into(), value.map(str::to_string))],
        };
        let names = |r: RunResult| -> Vec<Option<String>> {
            r.result
                .rows
                .into_iter()
                .map(|row| row[1].clone())
                .collect()
        };

        // The preview is exactly what will run.
        let edits = [edit("1", "name", Some("it's")), edit("2", "name", None)];
        let preview = m.edits_preview(&id, &target, &edits).await.unwrap();
        assert_eq!(
            preview[0],
            "UPDATE \"t\" SET \"name\" = 'it''s' WHERE \"id\" = '1'"
        );

        assert_eq!(m.edits_apply(&id, "e1", &target, &edits).await.unwrap(), 2);
        assert_eq!(
            names(
                run(&m, &id, "SELECT * FROM t ORDER BY id", false, &settings)
                    .await
                    .unwrap()
            ),
            [Some("it's".to_string()), None]
        );

        // The second edit violates a constraint: the first must be undone too.
        let failing = [
            edit("1", "name", Some("changed")),
            edit("2", "qty", Some("-5")),
        ];
        let error = m
            .edits_apply(&id, "e2", &target, &failing)
            .await
            .unwrap_err();
        assert!(error.detail.unwrap().contains("\"qty\" = '-5'"));
        assert_eq!(
            names(
                run(&m, &id, "SELECT * FROM t ORDER BY id", false, &settings)
                    .await
                    .unwrap()
            )[0],
            Some("it's".to_string())
        );

        // A row that no longer exists: nothing is saved.
        let gone = [edit("1", "name", Some("x")), edit("99", "name", Some("y"))];
        let error = m.edits_apply(&id, "e3", &target, &gone).await.unwrap_err();
        assert!(error.message.contains("0 rows instead of 1"), "{error:?}");
        assert_eq!(
            names(
                run(&m, &id, "SELECT * FROM t ORDER BY id", false, &settings)
                    .await
                    .unwrap()
            )[0],
            Some("it's".to_string())
        );
    }

    #[tokio::test]
    async fn edits_inside_an_open_transaction_keep_it_open() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = opened(&m, &dir, "nested", &settings).await;
        run(
            &m,
            &id,
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)",
            false,
            &settings,
        )
        .await
        .unwrap();
        run(&m, &id, "INSERT INTO t VALUES (1, 'a')", false, &settings)
            .await
            .unwrap();
        let target = run(&m, &id, "SELECT * FROM t", false, &settings)
            .await
            .unwrap()
            .result
            .editable
            .unwrap();

        m.set_auto_commit(&id, false).await.unwrap();
        run(
            &m,
            &id,
            "INSERT INTO t VALUES (2, 'mine')",
            false,
            &settings,
        )
        .await
        .unwrap();
        let bad = [RowEdit {
            key: vec![Some("99".into())],
            changes: vec![("name".into(), Some("x".into()))],
        }];
        m.edits_apply(&id, "e", &target, &bad).await.unwrap_err();
        // The user's own uncommitted insert survived the failed edit.
        assert!(m.list().await.unwrap()[0].in_transaction);
        let rows = run(&m, &id, "SELECT count(*) FROM t", false, &settings)
            .await
            .unwrap();
        assert_eq!(rows.result.rows[0][0].as_deref(), Some("2"));
    }

    #[tokio::test]
    async fn export_streams_the_full_result_to_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let id = opened(&m, &dir, "export", &Settings::default()).await;
        let path = dir.path().join("out.ndjson");
        let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000) SELECT i AS n FROM n";
        let rows = m
            .export_query(&id, "x1", sql, &path, Format::Ndjson, "t")
            .await
            .unwrap();
        assert_eq!(rows, 100_000);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 100_000);
        assert_eq!(text.lines().last(), Some("{\"n\":100000}"));

        // A failed export leaves no partial file.
        let bad = dir.path().join("bad.csv");
        m.export_query(&id, "x2", "SELECT * FROM missing", &bad, Format::Csv, "t")
            .await
            .unwrap_err();
        assert!(!bad.exists());

        // …and never destroys the file it was going to replace.
        m.export_query(&id, "x3", "SELECT * FROM missing", &path, Format::Csv, "t")
            .await
            .unwrap_err();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no .partial left"
        );
    }

    #[tokio::test]
    async fn a_statement_that_changes_data_is_never_run_again_for_more_rows() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = opened(&m, &dir, "rerun", &settings).await;
        run(&m, &id, "CREATE TABLE t (n INTEGER)", true, &settings)
            .await
            .unwrap();
        run(
            &m,
            &id,
            "INSERT INTO t VALUES (1), (2), (3)",
            true,
            &settings,
        )
        .await
        .unwrap();

        let bump = "UPDATE t SET n = n + 10 RETURNING n";
        let first = run(&m, &id, bump, true, &settings).await.unwrap();
        assert!(!first.repeatable);
        let again = RunOptions {
            row_limit: 0,
            confirmed: true,
            uncapped: false,
            refetch: true,
        };
        m.run_with(&id, "p1", bump, again, &settings)
            .await
            .unwrap_err();
        m.count(&id, "c1", bump, &settings).await.unwrap_err();
        m.export_query(&id, "e1", bump, &dir.path().join("x.csv"), Format::Csv, "t")
            .await
            .unwrap_err();
        let sum = run(&m, &id, "SELECT sum(n) FROM t", true, &settings)
            .await
            .unwrap();
        assert_eq!(
            sum.result.rows[0][0].as_deref(),
            Some("36"),
            "the UPDATE ran exactly once"
        );

        // A plain read can be repeated.
        let read = run(&m, &id, "SELECT n FROM t", true, &settings)
            .await
            .unwrap();
        assert!(read.repeatable);
        m.run_with(&id, "p2", "SELECT n FROM t", again, &settings)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn history_records_what_ran_unless_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let mut settings = Settings::default();
        let id = opened(&m, &dir, "hist", &settings).await;
        run(&m, &id, "SELECT 1", false, &settings).await.unwrap();
        run(&m, &id, "SELECT * FROM nope", false, &settings)
            .await
            .unwrap_err();

        let entries = m.history(Some(&id), "", 10).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sql, "SELECT * FROM nope");
        assert!(entries[0].error.as_ref().unwrap().contains("nope"));
        assert_eq!((entries[1].rows, entries[1].error.clone()), (Some(1), None));

        settings.privacy.keep_history = false;
        run(&m, &id, "SELECT 2", false, &settings).await.unwrap();
        assert_eq!(m.history(None, "", 10).unwrap().len(), 2);
    }

    const ENDLESS: &str =
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n";

    async fn opened(
        m: &Manager,
        dir: &tempfile::TempDir,
        name: &str,
        settings: &Settings,
    ) -> String {
        let id = m
            .save(sqlite(dir, name), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        m.open(&id, settings, None).await.unwrap();
        id
    }

    #[tokio::test]
    async fn cancelling_a_queued_statement_leaves_the_running_one_alone() {
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(manager());
        let id = opened(&m, &dir, "q", &Settings::default()).await;

        // Tab A runs; tab B queues behind it on the same connection.
        let (a, connection) = (m.clone(), id.clone());
        let running = tokio::spawn(async move {
            a.run(
                &connection,
                "tab-a",
                ENDLESS,
                1000,
                false,
                &Settings::default(),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (b, connection) = (m.clone(), id.clone());
        let queued = tokio::spawn(async move {
            b.run(
                &connection,
                "tab-b",
                "SELECT 'b ran'",
                1000,
                false,
                &Settings::default(),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Cancelling B ends B at once and never reaches the database.
        m.cancel("tab-b").await;
        assert_eq!(
            queued.await.unwrap().unwrap_err().kind,
            ErrorKind::Cancelled
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!running.is_finished(), "tab A must still be running");

        m.cancel("tab-a").await;
        assert_eq!(
            running.await.unwrap().unwrap_err().kind,
            ErrorKind::Cancelled
        );
    }

    #[tokio::test]
    async fn the_timeout_counts_from_when_a_statement_starts_running() {
        let dir = tempfile::tempdir().unwrap();
        let m = Arc::new(manager());
        let mut settings = Settings::default();
        settings.execution.query_timeout_secs = 1;
        let id = opened(&m, &dir, "t", &settings).await;

        let (a, connection, slow) = (m.clone(), id.clone(), settings.clone());
        let first =
            tokio::spawn(async move { a.run(&connection, "a", ENDLESS, 1000, false, &slow).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // Queued for ~0.9 s behind the first; it must not time out for waiting.
        let second = m.run(&id, "b", "SELECT 1", 1000, false, &settings).await;
        assert_eq!(first.await.unwrap().unwrap_err().kind, ErrorKind::Timeout);
        assert_eq!(second.unwrap().result.rows[0][0].as_deref(), Some("1"));
    }

    #[tokio::test]
    async fn auto_commit_is_per_connection() {
        let dir = tempfile::tempdir().unwrap();
        let m = manager();
        let settings = Settings::default();
        let id = opened(&m, &dir, "tx", &settings).await;
        let other = opened(&m, &dir, "other", &settings).await;
        for connection in [&id, &other] {
            run(
                &m,
                connection,
                "CREATE TABLE t (id INTEGER)",
                false,
                &settings,
            )
            .await
            .unwrap();
        }

        // Manual commit on one connection…
        m.set_auto_commit(&id, false).await.unwrap();
        let r = run(&m, &id, "INSERT INTO t VALUES (1)", false, &settings)
            .await
            .unwrap();
        assert!(r.result.in_transaction);
        // …does not change the other one.
        let r = run(&m, &other, "INSERT INTO t VALUES (1)", false, &settings)
            .await
            .unwrap();
        assert!(!r.result.in_transaction);

        let views = m.list().await.unwrap();
        assert!(views[0].in_transaction && !views[0].auto_commit);
        assert!(!views[1].in_transaction && views[1].auto_commit);

        m.end_transaction(&id, false).await.unwrap();
        assert!(!m.list().await.unwrap()[0].in_transaction);

        // New connections start from the "Auto-commit by default" setting.
        let mut manual_default = Settings::default();
        manual_default.execution.auto_commit = false;
        let third = m
            .save(sqlite(&dir, "third"), SecretInput::default())
            .await
            .unwrap()
            .config
            .id;
        assert!(
            !m.open(&third, &manual_default, None)
                .await
                .unwrap()
                .auto_commit
        );
    }
}

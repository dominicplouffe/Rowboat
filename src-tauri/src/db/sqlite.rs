//! SQLite driver (`rusqlite`, bundled SQLite).
//!
//! The connection lives on its own thread and is driven by messages, because
//! SQLite calls block. The row limit is applied by simply not stepping the
//! statement any further. Cancelling uses SQLite's interrupt handle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use async_trait::async_trait;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use tokio::sync::oneshot;

use super::{
    hex, quote_ident, quote_literal, Column, ColumnKind, ConnectParams, DbError, DbKind, DbResult,
    ErrorKind, ExecOptions, QueryResult, SchemaNode, Session,
};
use crate::statements::{tx_control, TxControl};

type Reply<T> = oneshot::Sender<DbResult<T>>;

enum Command {
    Execute(String, ExecOptions, Reply<QueryResult>),
    EndTransaction { commit: bool, reply: Reply<()> },
    Close,
}

pub struct SqliteSession {
    commands: mpsc::Sender<Command>,
    interrupt: rusqlite::InterruptHandle,
    version: String,
    shared: Arc<Shared>,
}

struct Shared {
    auto_commit: AtomicBool,
    in_transaction: AtomicBool,
}

impl SqliteSession {
    pub async fn connect(params: ConnectParams) -> DbResult<Self> {
        let config = params.config.clone();
        let shared = Arc::new(Shared {
            auto_commit: AtomicBool::new(params.auto_commit),
            in_transaction: AtomicBool::new(false),
        });
        let (ready_tx, ready_rx) = oneshot::channel();
        let (commands, inbox) = mpsc::channel::<Command>();
        let thread_shared = shared.clone();

        std::thread::Builder::new()
            .name("rowboat-sqlite".into())
            .spawn(move || {
                let flags = if config.read_only {
                    // The spec's `mode=ro`.
                    OpenFlags::SQLITE_OPEN_READ_ONLY
                } else if config.create {
                    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
                } else {
                    OpenFlags::SQLITE_OPEN_READ_WRITE
                };
                let opened = Connection::open_with_flags(&config.path, flags)
                    .map_err(|e| {
                        let mut error = map_error(e, None);
                        error.kind = ErrorKind::Connection;
                        if !config.create && !std::path::Path::new(&config.path).exists() {
                            error.message = format!("No such file: {}", config.path);
                            error.hint =
                                Some("Turn on “Create new file” to create a database here.".into());
                        }
                        error
                    })
                    .and_then(|conn| {
                        // Opening is lazy; reading the schema proves this is a database.
                        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
                            .map_err(|e| {
                                let mut error = map_error(e, None);
                                error.kind = ErrorKind::Connection;
                                error
                            })?;
                        Ok(conn)
                    });
                let conn = match opened {
                    Ok(conn) => conn,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let version = rusqlite::version().to_string();
                let _ = ready_tx.send(Ok((conn.get_interrupt_handle(), version)));
                serve(conn, inbox, thread_shared);
            })
            .map_err(|e| DbError::other(format!("Could not start the SQLite thread: {e}")))?;

        let (interrupt, version) = ready_rx
            .await
            .map_err(|_| DbError::connection("The SQLite thread stopped unexpectedly."))??;
        Ok(Self {
            commands,
            interrupt,
            version,
            shared,
        })
    }

    async fn request<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> DbResult<T> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(make(tx))
            .map_err(|_| DbError::connection("The connection is closed."))?;
        rx.await
            .map_err(|_| DbError::connection("The connection is closed."))?
    }

    async fn query(&self, sql: String) -> DbResult<QueryResult> {
        self.request(|reply| Command::Execute(sql, ExecOptions { row_limit: 0 }, reply))
            .await
    }
}

fn serve(conn: Connection, inbox: mpsc::Receiver<Command>, shared: Arc<Shared>) {
    while let Ok(command) = inbox.recv() {
        match command {
            Command::Execute(sql, options, reply) => {
                let result = execute(&conn, &sql, options, &shared);
                shared
                    .in_transaction
                    .store(!conn.is_autocommit(), Ordering::SeqCst);
                let _ = reply.send(result.map(|mut r| {
                    r.in_transaction = !conn.is_autocommit();
                    r
                }));
            }
            Command::EndTransaction { commit, reply } => {
                let result = if conn.is_autocommit() {
                    Ok(())
                } else {
                    conn.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" })
                        .map_err(|e| map_error(e, None))
                };
                shared
                    .in_transaction
                    .store(!conn.is_autocommit(), Ordering::SeqCst);
                let _ = reply.send(result);
            }
            Command::Close => break,
        }
    }
}

fn execute(
    conn: &Connection,
    sql: &str,
    options: ExecOptions,
    shared: &Shared,
) -> DbResult<QueryResult> {
    // Manual-commit mode: open a transaction before the first statement, the
    // way the user would have with BEGIN.
    let manual = !shared.auto_commit.load(Ordering::SeqCst);
    if manual && conn.is_autocommit() && tx_control(DbKind::Sqlite, sql) == TxControl::None {
        conn.execute_batch("BEGIN").map_err(|e| map_error(e, None))?;
    }

    let mut stmt = conn.prepare(sql).map_err(|e| map_error(e, Some(sql)))?;
    if stmt.column_count() == 0 {
        let changed = stmt.raw_execute().map_err(|e| map_error(e, Some(sql)))?;
        return Ok(QueryResult {
            // SQLite reports the previous statement's count for DDL; only
            // statements that can change rows have a meaningful number.
            rows_affected: (!stmt.readonly()).then_some(changed as u64),
            ..Default::default()
        });
    }

    let declared: Vec<(String, String)> = stmt
        .columns()
        .map(|c| (c.name().to_string(), c.decl_type().unwrap_or("").to_string()))
        .collect();
    let width = declared.len();
    let limit = options.row_limit as usize;
    let mut rows_out = Vec::new();
    let mut more = false;
    // Whether every non-NULL value seen in a column was a number.
    let mut numeric = vec![true; width];
    let mut seen = vec![false; width];

    let mut rows = stmt.raw_query();
    loop {
        let Some(row) = rows.next().map_err(|e| map_error(e, Some(sql)))? else {
            break;
        };
        if limit > 0 && rows_out.len() == limit {
            // One row past the limit exists; stop stepping here.
            more = true;
            break;
        }
        let mut cells = Vec::with_capacity(width);
        for i in 0..width {
            let value = row.get_ref(i).map_err(|e| map_error(e, Some(sql)))?;
            cells.push(match value {
                ValueRef::Null => None,
                ValueRef::Integer(n) => {
                    seen[i] = true;
                    Some(n.to_string())
                }
                ValueRef::Real(f) => {
                    seen[i] = true;
                    Some(format_real(f))
                }
                ValueRef::Text(t) => {
                    seen[i] = true;
                    numeric[i] = false;
                    Some(String::from_utf8_lossy(t).into_owned())
                }
                ValueRef::Blob(b) => {
                    seen[i] = true;
                    numeric[i] = false;
                    Some(hex(b))
                }
            });
        }
        rows_out.push(cells);
    }
    drop(rows);

    let columns = declared
        .into_iter()
        .enumerate()
        .map(|(i, (name, decl))| Column {
            kind: column_kind(&decl, seen[i] && numeric[i]),
            name,
            type_name: decl,
        })
        .collect();

    Ok(QueryResult {
        columns,
        rows: rows_out,
        more_available: more,
        ..Default::default()
    })
}

/// SQLite prints REAL with up to 15 significant digits and always shows a
/// decimal point; match that so values look as they do in `sqlite3`.
fn format_real(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Inf".into() } else { "-Inf".into() };
    }
    let text = format!("{f}");
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}

/// SQLite's type affinity rules, applied to the declared type, with the
/// observed values as a fallback for expressions (which have no declared type).
fn column_kind(declared: &str, all_numbers: bool) -> ColumnKind {
    let d = declared.to_ascii_uppercase();
    if d.contains("INT")
        || d.contains("REAL")
        || d.contains("FLOA")
        || d.contains("DOUB")
        || d.contains("NUM")
        || d.contains("DEC")
    {
        ColumnKind::Number
    } else if d.contains("BOOL") {
        ColumnKind::Bool
    } else if d.contains("DATE") || d.contains("TIME") {
        ColumnKind::Temporal
    } else if d.contains("BLOB") {
        ColumnKind::Binary
    } else if d.contains("JSON") {
        ColumnKind::Json
    } else if d.is_empty() && all_numbers {
        ColumnKind::Number
    } else {
        ColumnKind::Text
    }
}

fn map_error(error: rusqlite::Error, sql: Option<&str>) -> DbError {
    use rusqlite::ffi::ErrorCode;

    let (code, message, offset) = match &error {
        rusqlite::Error::SqliteFailure(e, message) => (
            Some(e.code),
            message.clone().unwrap_or_else(|| e.to_string()),
            None,
        ),
        rusqlite::Error::SqlInputError {
            error: e,
            msg,
            offset,
            ..
        } => (Some(e.code), msg.clone(), Some(*offset)),
        rusqlite::Error::MultipleStatement => (
            None,
            "Only one statement can be run at a time.".to_string(),
            None,
        ),
        other => (None, other.to_string(), None),
    };

    let lower = message.to_lowercase();
    let kind = match code {
        Some(ErrorCode::OperationInterrupted) => ErrorKind::Cancelled,
        Some(ErrorCode::ReadOnly | ErrorCode::PermissionDenied | ErrorCode::AuthorizationForStatementDenied) => {
            ErrorKind::Permission
        }
        Some(ErrorCode::CannotOpen | ErrorCode::NotADatabase) => ErrorKind::Connection,
        _ if lower.contains("syntax error") || lower.contains("incomplete input") => {
            ErrorKind::Syntax
        }
        _ if lower.starts_with("no such ") || lower.contains("has no column") => ErrorKind::Schema,
        _ => ErrorKind::Other,
    };

    // SQLite reports a byte offset; the UI wants a 1-based character offset.
    let position = match (offset, sql) {
        (Some(offset), Some(sql)) if offset >= 0 => {
            let bytes = (offset as usize).min(sql.len());
            let safe = (0..=bytes)
                .rev()
                .find(|&i| sql.is_char_boundary(i))
                .unwrap_or(0);
            Some(sql[..safe].chars().count() as u32 + 1)
        }
        _ => None,
    };

    DbError {
        kind,
        message: if kind == ErrorKind::Cancelled {
            "Query cancelled.".into()
        } else {
            message
        },
        detail: None,
        hint: (kind == ErrorKind::Permission)
            .then(|| "This connection is read-only.".to_string()),
        code: code.map(|c| format!("{c:?}")),
        position,
    }
}

const GROUPS: [(&str, &str, &str); 4] = [
    ("Tables", "table", "table"),
    ("Views", "view", "view"),
    ("Indexes", "index", "index"),
    ("Triggers", "trigger", "trigger"),
];

#[async_trait]
impl Session for SqliteSession {
    fn kind(&self) -> DbKind {
        DbKind::Sqlite
    }

    fn server_version(&self) -> String {
        format!("SQLite {}", self.version)
    }

    async fn execute(&self, sql: &str, options: ExecOptions) -> DbResult<QueryResult> {
        let sql = sql.to_string();
        self.request(|reply| Command::Execute(sql, options, reply))
            .await
    }

    async fn cancel(&self) {
        self.interrupt.interrupt();
    }

    async fn commit(&self) -> DbResult<()> {
        self.request(|reply| Command::EndTransaction {
            commit: true,
            reply,
        })
        .await
    }

    async fn rollback(&self) -> DbResult<()> {
        self.request(|reply| Command::EndTransaction {
            commit: false,
            reply,
        })
        .await
    }

    fn in_transaction(&self) -> bool {
        self.shared.in_transaction.load(Ordering::SeqCst)
    }

    fn supports_transactions(&self) -> bool {
        true
    }

    fn set_auto_commit(&self, auto_commit: bool) {
        self.shared.auto_commit.store(auto_commit, Ordering::SeqCst);
    }

    async fn children(&self, path: &[String]) -> DbResult<Vec<SchemaNode>> {
        match path {
            [] => Ok(GROUPS
                .iter()
                .map(|(label, _, _)| SchemaNode::branch(*label, "group"))
                .collect()),
            [group] => {
                let Some((_, object_type, kind)) = GROUPS.iter().find(|(label, _, _)| label == group)
                else {
                    return Ok(vec![]);
                };
                let result = self
                    .query(format!(
                        "SELECT name, tbl_name FROM sqlite_master WHERE type = {} \
                         AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
                        quote_literal(DbKind::Sqlite, object_type)
                    ))
                    .await?;
                Ok(result
                    .rows
                    .into_iter()
                    .map(|row| {
                        let name = row[0].clone().unwrap_or_default();
                        let has_columns = matches!(*kind, "table" | "view");
                        let mut node = if has_columns {
                            SchemaNode::branch(name, kind)
                        } else {
                            SchemaNode::leaf(name, kind)
                        };
                        if !has_columns {
                            node = node.detail(format!("on {}", row[1].clone().unwrap_or_default()));
                        }
                        node
                    })
                    .collect())
            }
            [group, table] if group == "Tables" || group == "Views" => {
                let result = self
                    .query(format!(
                        "SELECT name, type, \"notnull\", pk FROM pragma_table_info({})",
                        quote_literal(DbKind::Sqlite, table)
                    ))
                    .await?;
                Ok(result
                    .rows
                    .into_iter()
                    .map(|row| {
                        let mut detail = row[1].clone().unwrap_or_default();
                        if row[3].as_deref().is_some_and(|pk| pk != "0") {
                            detail.push_str(" · primary key");
                        } else if row[2].as_deref() == Some("1") {
                            detail.push_str(" · not null");
                        }
                        SchemaNode::leaf(row[0].clone().unwrap_or_default(), "column")
                            .detail(detail.trim_start_matches(" · ").to_string())
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    async fn ddl(&self, path: &[String]) -> DbResult<String> {
        let name = path
            .get(1)
            .ok_or_else(|| DbError::other("Select a table, view, index or trigger."))?;
        let result = self
            .query(format!(
                "SELECT sql FROM sqlite_master WHERE name = {}",
                quote_literal(DbKind::Sqlite, name)
            ))
            .await?;
        match result.rows.first().and_then(|row| row[0].clone()) {
            Some(sql) => Ok(format!("{sql};")),
            None => Err(DbError::other(format!(
                "{} has no stored definition (it was created automatically).",
                quote_ident(DbKind::Sqlite, name)
            ))),
        }
    }

    async fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connections::ConnectionConfig;
    use std::time::Duration;

    fn params(path: &std::path::Path) -> ConnectParams {
        ConnectParams {
            config: ConnectionConfig {
                name: "t".into(),
                kind: DbKind::Sqlite,
                path: path.display().to_string(),
                create: true,
                ..Default::default()
            },
            password: None,
            client_id: "rowboat-test".into(),
            connect_timeout: Duration::from_secs(5),
            keepalive: Duration::ZERO,
            auto_commit: true,
            auto_reconnect: false,
            via: None,
        }
    }

    async fn open() -> (tempfile::TempDir, SqliteSession) {
        let dir = tempfile::tempdir().unwrap();
        let session = SqliteSession::connect(params(&dir.path().join("t.db")))
            .await
            .unwrap();
        (dir, session)
    }

    const ALL: ExecOptions = ExecOptions { row_limit: 0 };

    async fn seed(session: &SqliteSession, rows: u32) {
        session
            .execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, data BLOB)", ALL)
            .await
            .unwrap();
        session
            .execute(
                &format!(
                    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {rows}) \
                     INSERT INTO t (id, name, score) SELECT i, 'row ' || i, i * 1.5 FROM n"
                ),
                ALL,
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn fetches_rows_as_display_text_with_column_metadata() {
        let (_dir, s) = open().await;
        seed(&s, 3).await;
        s.execute("UPDATE t SET data = x'00ff', score = NULL WHERE id = 2", ALL)
            .await
            .unwrap();
        let r = s.execute("SELECT id, name, score, data FROM t ORDER BY id", ALL).await.unwrap();
        let names: Vec<_> = r.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "name", "score", "data"]);
        assert_eq!(r.columns[0].kind, ColumnKind::Number);
        assert_eq!(r.columns[1].kind, ColumnKind::Text);
        assert_eq!(r.columns[3].kind, ColumnKind::Binary);
        assert_eq!(
            r.rows[1],
            vec![Some("2".into()), Some("row 2".into()), None, Some("\\x00ff".into())]
        );
        assert_eq!(r.rows[0][2].as_deref(), Some("1.5"));
        assert!(!r.more_available);
    }

    #[tokio::test]
    async fn row_limit_stops_stepping_and_reports_more() {
        let (_dir, s) = open().await;
        seed(&s, 50).await;
        let r = s.execute("SELECT * FROM t", ExecOptions { row_limit: 10 }).await.unwrap();
        assert_eq!(r.rows.len(), 10);
        assert!(r.more_available);

        let exact = s.execute("SELECT * FROM t", ExecOptions { row_limit: 50 }).await.unwrap();
        assert_eq!(exact.rows.len(), 50);
        assert!(!exact.more_available, "exactly the limit is not 'more'");
    }

    #[tokio::test]
    async fn row_limit_bounds_work_on_a_huge_result() {
        let (_dir, s) = open().await;
        // 10 million rows if fully stepped; must come back at once.
        let sql = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10000000) SELECT i FROM n";
        let started = std::time::Instant::now();
        let r = s.execute(sql, ExecOptions { row_limit: 1000 }).await.unwrap();
        assert_eq!(r.rows.len(), 1000);
        assert!(r.more_available);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn reports_rows_affected_for_writes_only() {
        let (_dir, s) = open().await;
        seed(&s, 5).await;
        let update = s.execute("UPDATE t SET name = 'x' WHERE id <= 3", ALL).await.unwrap();
        assert_eq!(update.rows_affected, Some(3));
        let ddl = s.execute("CREATE TABLE u (a)", ALL).await.unwrap();
        assert!(ddl.columns.is_empty());
    }

    #[tokio::test]
    async fn errors_are_classified_and_positioned() {
        let (_dir, s) = open().await;
        let syntax = s.execute("SELEC 1", ALL).await.unwrap_err();
        assert_eq!(syntax.kind, ErrorKind::Syntax);
        assert_eq!(syntax.position, Some(1));

        let later = s.execute("SELECT 1 FROM WHERE", ALL).await.unwrap_err();
        assert_eq!(later.kind, ErrorKind::Syntax);
        assert_eq!(later.position, Some(15), "{later:?}");

        let schema = s.execute("SELECT * FROM missing", ALL).await.unwrap_err();
        assert_eq!(schema.kind, ErrorKind::Schema);
        assert!(schema.message.contains("missing"));
    }

    #[tokio::test]
    async fn cancel_interrupts_a_long_query_quickly() {
        let (_dir, s) = open().await;
        let s = Arc::new(s);
        let runner = s.clone();
        let task = tokio::spawn(async move {
            runner
                .execute(
                    "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n) SELECT count(*) FROM n",
                    ALL,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        let cancelled_at = std::time::Instant::now();
        s.cancel().await;
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert!(cancelled_at.elapsed() < Duration::from_secs(1));
        // The session is still usable.
        assert_eq!(s.execute("SELECT 1", ALL).await.unwrap().rows.len(), 1);
    }

    #[tokio::test]
    async fn manual_commit_mode_holds_changes_until_commit() {
        let (dir, s) = open().await;
        seed(&s, 1).await;
        s.set_auto_commit(false);
        let r = s.execute("INSERT INTO t (id, name) VALUES (100, 'pending')", ALL).await.unwrap();
        assert!(r.in_transaction && s.in_transaction());

        // A second connection must not see the uncommitted row.
        let mut other = params(&dir.path().join("t.db"));
        other.config.create = false;
        let peer = SqliteSession::connect(other).await.unwrap();
        let count = |r: QueryResult| r.rows[0][0].clone().unwrap();
        assert_eq!(count(peer.execute("SELECT count(*) FROM t", ALL).await.unwrap()), "1");

        s.rollback().await.unwrap();
        assert!(!s.in_transaction());
        assert_eq!(count(s.execute("SELECT count(*) FROM t", ALL).await.unwrap()), "1");
        s.rollback().await.unwrap(); // the SELECT above opened a new transaction

        s.execute("INSERT INTO t (id, name) VALUES (100, 'kept')", ALL).await.unwrap();
        s.commit().await.unwrap();
        assert_eq!(count(peer.execute("SELECT count(*) FROM t", ALL).await.unwrap()), "2");
    }

    #[tokio::test]
    async fn read_only_connections_refuse_writes() {
        let (dir, s) = open().await;
        seed(&s, 1).await;
        s.close().await;

        let mut ro = params(&dir.path().join("t.db"));
        ro.config.read_only = true;
        let s = SqliteSession::connect(ro).await.unwrap();
        assert_eq!(s.execute("SELECT count(*) FROM t", ALL).await.unwrap().rows.len(), 1);
        let error = s.execute("DELETE FROM t", ALL).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Permission);
    }

    #[tokio::test]
    async fn opening_a_missing_or_invalid_file_fails_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let mut missing = params(&dir.path().join("nope.db"));
        missing.config.create = false;
        let error = SqliteSession::connect(missing).await.err().unwrap();
        assert_eq!(error.kind, ErrorKind::Connection);
        assert!(error.message.contains("No such file"));

        let junk = dir.path().join("junk.db");
        std::fs::write(&junk, "this is not a database, just some text padding............").unwrap();
        let mut not_db = params(&junk);
        not_db.config.create = false;
        assert_eq!(
            SqliteSession::connect(not_db).await.err().unwrap().kind,
            ErrorKind::Connection
        );
    }

    #[tokio::test]
    async fn navigator_lists_objects_columns_and_ddl() {
        let (_dir, s) = open().await;
        seed(&s, 1).await;
        s.execute("CREATE VIEW v AS SELECT id FROM t", ALL).await.unwrap();
        s.execute("CREATE INDEX t_name ON t (name)", ALL).await.unwrap();

        let roots: Vec<_> = s.children(&[]).await.unwrap().into_iter().map(|n| n.name).collect();
        assert_eq!(roots, ["Tables", "Views", "Indexes", "Triggers"]);

        let tables = s.children(&["Tables".into()]).await.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!((tables[0].name.as_str(), tables[0].kind), ("t", "table"));

        let columns = s.children(&["Tables".into(), "t".into()]).await.unwrap();
        assert_eq!(columns.len(), 4);
        assert_eq!(columns[0].detail.as_deref(), Some("INTEGER · primary key"));
        assert_eq!(columns[1].detail.as_deref(), Some("TEXT · not null"));

        let indexes = s.children(&["Indexes".into()]).await.unwrap();
        assert_eq!(indexes[0].detail.as_deref(), Some("on t"));

        let ddl = s.ddl(&["Views".into(), "v".into()]).await.unwrap();
        assert_eq!(ddl, "CREATE VIEW v AS SELECT id FROM t;");
    }
}

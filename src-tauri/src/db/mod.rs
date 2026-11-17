//! Database access. One [`Session`] per open connection, behind a common
//! trait, so the rest of the core never cares which database it talks to.
//!
//! Two rules from the spec are enforced here rather than left to callers:
//! the row limit is applied while fetching (never by changing the SQL), and
//! every value leaves this module already rendered as display text.

pub mod ch_rowbinary;
pub mod clickhouse;
pub mod pg_types;
pub mod postgres;
pub mod sqlite;

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Clickhouse,
    Sqlite,
}

/// Broad category of a column, for alignment and (later) type-aware cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColumnKind {
    Text,
    Number,
    Bool,
    Temporal,
    Binary,
    Json,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// The database's own name for the type, e.g. `timestamptz`.
    pub type_name: String,
    pub kind: ColumnKind,
}

/// A cell: `None` is SQL NULL, anything else is display text.
pub type Cell = Option<String>;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    /// True when the row limit stopped the fetch and the query has more rows.
    pub more_available: bool,
    /// For statements that change rows instead of returning them.
    pub rows_affected: Option<u64>,
    /// Output the database formatted itself (ClickHouse `FORMAT …`), shown as text.
    pub raw: Option<String>,
    /// Server notices and warnings produced by the statement.
    pub notices: Vec<String>,
    /// Whether the session is inside an open transaction after this statement.
    pub in_transaction: bool,
}

/// What went wrong, in a form the UI and (later) the AI can act on. The
/// distinction matters: only syntax and schema errors are sent to the AI for
/// a suggested fix; connection and permission errors never are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorKind {
    Syntax,
    Schema,
    Permission,
    Connection,
    Cancelled,
    Timeout,
    /// The Prod guard wants a confirmation before running this statement.
    NeedsConfirmation,
    /// The SSH server's host key is not in known_hosts yet.
    UnknownHostKey,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct DbError {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// The database's error code (SQLSTATE, ClickHouse code, …).
    pub code: Option<String>,
    /// 1-based character offset into the statement, when the database gives one.
    pub position: Option<u32>,
}

impl DbError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            detail: None,
            hint: None,
            code: None,
            position: None,
        }
    }
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Other, message)
    }
    pub fn connection(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Connection, message)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Query cancelled.")
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Clone, Copy)]
pub struct ExecOptions {
    /// Maximum rows to fetch; 0 means no limit.
    pub row_limit: u32,
}

/// A node in the schema navigator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaNode {
    pub name: String,
    /// `schema`, `database`, `group`, `table`, `view`, `matview`, `function`,
    /// `sequence`, `type`, `dictionary`, `index`, `trigger`, `column`.
    pub kind: &'static str,
    /// Secondary text: a column's type, a table's engine, …
    pub detail: Option<String>,
    pub has_children: bool,
}

impl SchemaNode {
    pub fn branch(name: impl Into<String>, kind: &'static str) -> Self {
        Self {
            name: name.into(),
            kind,
            detail: None,
            has_children: true,
        }
    }
    pub fn leaf(name: impl Into<String>, kind: &'static str) -> Self {
        Self {
            name: name.into(),
            kind,
            detail: None,
            has_children: false,
        }
    }
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        if !detail.is_empty() {
            self.detail = Some(detail);
        }
        self
    }
}

/// Everything a driver needs to open a session, with secrets already
/// resolved. Never serialized, never logged.
#[derive(Clone)]
pub struct ConnectParams {
    pub config: crate::connections::ConnectionConfig,
    pub password: Option<String>,
    /// Sent as `application_name` (PostgreSQL) / client name (ClickHouse).
    pub client_id: String,
    pub connect_timeout: Duration,
    /// Zero disables keepalive.
    pub keepalive: Duration,
    pub auto_commit: bool,
    pub auto_reconnect: bool,
    /// When an SSH tunnel is in use: the local address that forwards to the
    /// database. TLS still verifies against the configured host name.
    pub via: Option<std::net::SocketAddr>,
}

#[async_trait]
pub trait Session: Send + Sync {
    fn kind(&self) -> DbKind;
    fn server_version(&self) -> String;

    /// Runs one statement and fetches at most `row_limit` rows.
    async fn execute(&self, sql: &str, options: ExecOptions) -> DbResult<QueryResult>;

    /// Cancels whatever statement is running, if any. Safe to call any time.
    async fn cancel(&self);

    async fn commit(&self) -> DbResult<()>;
    async fn rollback(&self) -> DbResult<()>;
    fn in_transaction(&self) -> bool;
    /// False for databases without transactions (ClickHouse).
    fn supports_transactions(&self) -> bool;
    fn set_auto_commit(&self, auto_commit: bool);

    /// Children of a navigator node; an empty path lists the roots.
    async fn children(&self, path: &[String]) -> DbResult<Vec<SchemaNode>>;
    /// DDL for the object at `path`.
    async fn ddl(&self, path: &[String]) -> DbResult<String>;

    async fn close(&self);
}

pub async fn connect(params: ConnectParams) -> DbResult<Box<dyn Session>> {
    match params.config.kind {
        DbKind::Postgres => Ok(Box::new(postgres::PgSession::connect(params).await?)),
        DbKind::Clickhouse => Ok(Box::new(clickhouse::ChSession::connect(params).await?)),
        DbKind::Sqlite => Ok(Box::new(sqlite::SqliteSession::connect(params).await?)),
    }
}

/// Quotes an identifier for the given database.
pub fn quote_ident(kind: DbKind, name: &str) -> String {
    match kind {
        DbKind::Clickhouse => format!("`{}`", name.replace('\\', "\\\\").replace('`', "\\`")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

/// Quotes a string literal (standard SQL quoting, plus backslashes for ClickHouse).
pub fn quote_literal(kind: DbKind, value: &str) -> String {
    match kind {
        DbKind::Clickhouse => format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'")),
        _ => format!("'{}'", value.replace('\'', "''")),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("\\x");
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

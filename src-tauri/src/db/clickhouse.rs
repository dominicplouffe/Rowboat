//! ClickHouse driver over the HTTP(S) interface (`reqwest`).
//!
//! * Results are requested as `RowBinaryWithNamesAndTypes` and decoded as they
//!   stream in; connections are kept alive and responses are compressed.
//! * The row limit is `max_result_rows` + `result_overflow_mode=break`, so the
//!   server stops early. Because `break` stops at a block boundary, the stream
//!   is also cut client-side once the limit is reached.
//! * The Client ID is sent as the HTTP client name (`User-Agent`, visible as
//!   `http_user_agent` in `system.query_log`) and as `log_comment`.
//! * Cancel sends `KILL QUERY` for the running query id and drops the stream.
//!
//! Users whose profile forbids changing settings (`readonly=1`) cannot receive
//! the settings above; the driver notices the refusal once and from then on
//! sends none, relying on the client-side cut for the row limit.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::Notify;

use super::ch_rowbinary::Decoder;
use super::{
    quote_ident, quote_literal, ConnectParams, DbError, DbKind, DbResult, ErrorKind, ExecOptions,
    QueryResult, SchemaNode, Session,
};
use super::{ColumnDetails, SchemaDetails, TableDetails};
use super::{RowSink, SchemaSnapshot, SnapshotColumn, SnapshotFunction, SnapshotTable};
use crate::connections::TlsMode;

const FORMAT: &str = "RowBinaryWithNamesAndTypes";
/// Output in a format the user chose (`FORMAT …`) is shown as text up to this size.
const RAW_LIMIT: usize = 5 * 1024 * 1024;

/// How many settings the server lets this user send.
const SETTINGS_ALL: u8 = 0;
/// Only `readonly=1` (when the connection is read-only).
const SETTINGS_READONLY_ONLY: u8 = 1;
const SETTINGS_NONE: u8 = 2;

pub struct ChSession {
    http: reqwest::Client,
    endpoint: reqwest::Url,
    user: String,
    password: String,
    database: String,
    client_id: String,
    read_only: bool,
    /// Keeps `USE`, `SET` and temporary tables alive between statements.
    session_id: Option<String>,
    version: String,
    settings_level: AtomicU8,
    /// True once a request in the session has succeeded with `database` set.
    session_has_database: AtomicBool,
    /// Serializes user statements: a ClickHouse HTTP session allows one at a time.
    gate: tokio::sync::Mutex<()>,
    running: Mutex<Option<String>>,
    cancelled: Notify,
}

fn error_text(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        let inner_text = inner.to_string();
        if !text.contains(&inner_text) {
            text.push_str(": ");
            text.push_str(&inner_text);
        }
        source = inner.source();
    }
    text
}

fn transport_error(error: reqwest::Error) -> DbError {
    let kind = if error.is_timeout() {
        ErrorKind::Timeout
    } else {
        ErrorKind::Connection
    };
    DbError::new(kind, error_text(&error))
}

/// Turns a ClickHouse exception text into a structured error.
///
/// `Code: 62. DB::Exception: Syntax error: failed at position 8 ('FROMM') …
/// (SYNTAX_ERROR) (version 24.8.1.1)`
pub fn parse_exception(body: &str, header_code: Option<&str>) -> DbError {
    let body = body.trim();
    let code = header_code.map(str::to_string).or_else(|| {
        let rest = body.split("Code: ").nth(1)?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        (!digits.is_empty()).then_some(digits)
    });
    let kind = match code.as_deref() {
        Some("62") => ErrorKind::Syntax,
        // Unknown column, function, identifier, table or database.
        Some("16" | "36" | "46" | "47" | "60" | "81" | "115" | "190") => ErrorKind::Schema,
        Some("164" | "192" | "193" | "194" | "497" | "516") => ErrorKind::Permission,
        Some("394") => ErrorKind::Cancelled,
        Some("159" | "160") => ErrorKind::Timeout,
        Some("210" | "209" | "203") => ErrorKind::Connection,
        _ => ErrorKind::Other,
    };

    let mut message = body;
    if let Some(i) = message.find("DB::Exception: ") {
        message = &message[i + "DB::Exception: ".len()..];
    }
    if let Some(i) = message.rfind(" (version ") {
        message = &message[..i];
    }
    let message = message.lines().next().unwrap_or(message).trim();

    let position = body
        .split("failed at position ")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok());

    DbError {
        kind,
        message: if kind == ErrorKind::Cancelled {
            "Query cancelled.".into()
        } else if message.is_empty() {
            "ClickHouse returned an error without a message.".into()
        } else {
            message.to_string()
        },
        detail: None,
        hint: None,
        code,
        position,
    }
}

/// Finds an exception the server appended after it had already started
/// streaming a successful response.
fn trailing_exception(bytes: &[u8]) -> Option<DbError> {
    let text = String::from_utf8_lossy(bytes);
    let start = text.find("Code: ")?;
    text[start..]
        .contains("DB::Exception")
        .then(|| parse_exception(&text[start..], None))
}

impl ChSession {
    pub async fn connect(params: ConnectParams) -> DbResult<Self> {
        crate::tls::ensure_provider();
        let c = &params.config;
        let secure = c.tls.mode != TlsMode::Disable;
        let port = params.via.map_or(c.port, |via| via.port());
        let endpoint = reqwest::Url::parse(&format!(
            "{}://{}:{port}/",
            if secure { "https" } else { "http" },
            if c.host.contains(':') {
                format!("[{}]", c.host) // IPv6 literal
            } else {
                c.host.clone()
            }
        ))
        .map_err(|e| DbError::connection(format!("{} is not a valid host: {e}", c.host)))?;

        let mut builder = reqwest::Client::builder()
            .user_agent(params.client_id.clone())
            .connect_timeout(params.connect_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            // HTTP proxies are not supported yet; never pick one up from the environment.
            .no_proxy();
        if !params.keepalive.is_zero() {
            builder = builder.tcp_keepalive(params.keepalive);
        }
        if let Some(via) = params.via {
            // Through the SSH tunnel; the URL keeps the real host for TLS.
            builder = builder.resolve(&c.host, via);
        }
        if secure {
            if !c.tls.ca_cert_path.is_empty() {
                for der in crate::tls::certificates(&c.tls.ca_cert_path, "CA certificate")? {
                    let cert = reqwest::Certificate::from_der(&der).map_err(|e| {
                        DbError::connection(format!("The CA certificate is not usable: {e}"))
                    })?;
                    builder = builder.add_root_certificate(cert);
                }
            }
            builder = match c.tls.mode {
                TlsMode::Prefer | TlsMode::Require => builder.danger_accept_invalid_certs(true),
                TlsMode::VerifyCa => builder.danger_accept_invalid_hostnames(true),
                _ => builder,
            };
        }
        let http = builder.build().map_err(|e| {
            DbError::connection(format!(
                "Could not set up the HTTP client: {}",
                error_text(&e)
            ))
        })?;

        let mut session = Self {
            http,
            endpoint,
            user: if c.user.is_empty() {
                "default".into()
            } else {
                c.user.clone()
            },
            password: params.password.clone().unwrap_or_default(),
            database: c.database.clone(),
            client_id: params.client_id.clone(),
            read_only: c.read_only,
            // A read-only session would lock its own settings after the first
            // request, so read-only connections stay stateless.
            session_id: (!c.read_only).then(|| format!("rowboat-{}", uuid::Uuid::new_v4())),
            version: String::new(),
            settings_level: AtomicU8::new(SETTINGS_ALL),
            session_has_database: AtomicBool::new(false),
            gate: tokio::sync::Mutex::new(()),
            running: Mutex::new(None),
            cancelled: Notify::new(),
        };
        let version = session
            .internal("SELECT version()")
            .await
            .map_err(|mut e| {
                if matches!(e.kind, ErrorKind::Other | ErrorKind::Timeout) {
                    e.kind = ErrorKind::Connection;
                }
                e
            })?;
        session.version = version
            .rows
            .first()
            .and_then(|row| row[0].clone())
            .unwrap_or_default();
        Ok(session)
    }

    fn request(
        &self,
        sql: &str,
        query_id: &str,
        row_limit: u32,
        level: u8,
        stateful: bool,
    ) -> reqwest::RequestBuilder {
        let mut url = self.endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("query_id", query_id);
            // In a session the database is set once; sending it every time
            // would undo the user's own `USE other_db`.
            let in_session = stateful && self.session_id.is_some();
            if !self.database.is_empty()
                && !(in_session && self.session_has_database.load(Ordering::SeqCst))
            {
                query.append_pair("database", &self.database);
            }
            if stateful {
                if let Some(session) = &self.session_id {
                    query.append_pair("session_id", session);
                    query.append_pair("session_timeout", "3600");
                }
            }
            if self.read_only && level <= SETTINGS_READONLY_ONLY {
                query.append_pair("readonly", "1");
            }
            if level == SETTINGS_ALL {
                if row_limit > 0 {
                    query.append_pair("max_result_rows", &row_limit.to_string());
                    query.append_pair("result_overflow_mode", "break");
                }
                query.append_pair("enable_http_compression", "1");
                query.append_pair("output_format_binary_write_json_as_string", "1");
                query.append_pair("log_comment", &self.client_id);
            }
        }
        self.http
            .post(url)
            // Basic auth is base64, so any UTF-8 user name or password works
            // (raw header values cannot carry non-ASCII text).
            .basic_auth(&self.user, Some(&self.password))
            // The default output format; a FORMAT clause in the SQL still wins.
            .header("X-ClickHouse-Format", FORMAT)
            .body(sql.to_string())
    }

    /// Sends a statement and decodes the response, stepping down the settings
    /// level if this user may not change settings.
    async fn run(
        &self,
        sql: &str,
        query_id: &str,
        options: ExecOptions,
        stateful: bool,
    ) -> DbResult<QueryResult> {
        let mut lock_retries = 0;
        loop {
            let level = self.settings_level.load(Ordering::SeqCst);
            match self.run_once(sql, query_id, options, level, stateful).await {
                Ok(result) => {
                    if stateful {
                        self.session_has_database.store(true, Ordering::SeqCst);
                    }
                    return Ok(result);
                }
                // SESSION_IS_LOCKED: the server is still winding down the
                // previous statement in this session (one we cut at the row
                // limit or cancelled). It frees up within moments.
                Err(error) if error.code.as_deref() == Some("373") && lock_retries < 12 => {
                    lock_retries += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(error) if error.code.as_deref() == Some("164") && level < SETTINGS_NONE => {
                    // "Cannot modify setting in readonly mode": not a failure of
                    // the user's statement, unless it fails again without them.
                    let next = if level == SETTINGS_ALL && self.read_only {
                        SETTINGS_READONLY_ONLY
                    } else {
                        SETTINGS_NONE
                    };
                    if !error.message.to_lowercase().contains("setting") {
                        return Err(error.with_hint("This connection is read-only."));
                    }
                    self.settings_level.store(next, Ordering::SeqCst);
                }
                other => return other,
            }
        }
    }

    async fn run_once(
        &self,
        sql: &str,
        query_id: &str,
        options: ExecOptions,
        level: u8,
        stateful: bool,
    ) -> DbResult<QueryResult> {
        let row_limit = options.row_limit;
        let response = self
            .request(sql, query_id, row_limit, level, stateful)
            .send()
            .await
            .map_err(transport_error)?;

        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let status = response.status();
        let exception_code = header("X-ClickHouse-Exception-Code");
        let format = header("X-ClickHouse-Format");
        let written: Option<u64> = header("X-ClickHouse-Summary")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
            .and_then(|summary| summary.get("written_rows")?.as_str()?.parse().ok());

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let mut error = parse_exception(&body, exception_code.as_deref());
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                error.kind = ErrorKind::Permission;
            }
            if error.kind == ErrorKind::Permission
                && error.code.as_deref() == Some("164")
                && error.hint.is_none()
            {
                error.hint = Some("This connection is read-only.".into());
            }
            return Err(error);
        }

        let limit = row_limit as usize;
        let mut stream = response.bytes_stream();
        let mut result = QueryResult::default();

        if format.as_deref().is_some_and(|f| f != FORMAT) {
            // The user chose an output format; show the server's text as is.
            let mut raw = Vec::new();
            while let Some(chunk) = stream.next().await {
                raw.extend_from_slice(&chunk.map_err(transport_error)?);
                if raw.len() > RAW_LIMIT {
                    raw.truncate(RAW_LIMIT);
                    result.more_available = true;
                    break;
                }
            }
            if let Some(error) = trailing_exception(&raw) {
                return Err(error);
            }
            result.raw = Some(String::from_utf8_lossy(&raw).into_owned());
            return Ok(result);
        }

        let mut decoder = Decoder::new();
        // Rows still wanted, plus one to learn whether more exist.
        let want = |have: usize| if limit == 0 { 0 } else { limit + 1 - have };
        let mut bytes = 0u64;
        let mut over_limit = |result: &mut QueryResult, rows: Vec<Vec<Option<String>>>| {
            bytes += rows.iter().map(|row| super::row_bytes(row)).sum::<u64>();
            result.rows.extend(rows);
            if limit > 0 && result.rows.len() > limit {
                result.rows.truncate(limit);
                result.more_available = true;
            } else if options.over_memory(bytes) {
                // The result memory cap: stop here and say so.
                result.stopped_at_memory_cap = true;
                result.more_available = true;
            }
            result.more_available
        };
        let mut ended = true;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(transport_error)?;
            let rows = decoder
                .push(&chunk, want(result.rows.len()))
                .map_err(DbError::other)?;
            if over_limit(&mut result, rows) {
                // Dropping the stream closes the connection: nothing past the
                // limit is fetched.
                ended = false;
                break;
            }
        }
        if ended {
            let rows = decoder
                .finish(want(result.rows.len()))
                .map_err(DbError::other)?;
            over_limit(&mut result, rows);
            if let Some(text) = decoder.exception() {
                return Err(parse_exception(&text, None));
            }
            // The cap may have been crossed by the very last rows.
            if result.stopped_at_memory_cap {
                result.stopped_at_memory_cap = false;
                result.more_available = false;
            }
            if !result.more_available && !decoder.pending().is_empty() {
                return Err(DbError::other(
                    "The server's response ended in the middle of a row.",
                ));
            }
        }
        result.columns = decoder.columns;
        if result.columns.is_empty() {
            result.rows_affected = written.filter(|&n| n > 0);
        }
        Ok(result)
    }

    /// A stateless request outside the user's session, for the navigator.
    async fn internal(&self, sql: &str) -> DbResult<QueryResult> {
        let id = uuid::Uuid::new_v4().to_string();
        self.run(sql, &id, ExecOptions::limit(0), false).await
    }
}

fn lit(value: &str) -> String {
    quote_literal(DbKind::Clickhouse, value)
}

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

const TABLES: &str = "Tables";
const VIEWS: &str = "Views";
const DICTIONARIES: &str = "Dictionaries";

#[async_trait]
impl Session for ChSession {
    fn kind(&self) -> DbKind {
        DbKind::Clickhouse
    }

    fn server_version(&self) -> String {
        format!("ClickHouse {}", self.version)
    }

    async fn execute(&self, sql: &str, options: ExecOptions) -> DbResult<QueryResult> {
        let _turn = self.gate.lock().await;
        let query_id = uuid::Uuid::new_v4().to_string();
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = Some(query_id.clone());
        let result = tokio::select! {
            result = self.run(sql, &query_id, options, true) => result,
            () = self.cancelled.notified() => Err(DbError::cancelled()),
        };
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
        result
    }

    async fn cancel(&self) {
        let running = self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(query_id) = running else { return };
        // Release the caller first, then tell the server to stop working.
        self.cancelled.notify_waiters();
        let kill = format!("KILL QUERY WHERE query_id = {}", lit(&query_id));
        let _ = tokio::time::timeout(Duration::from_secs(5), self.internal(&kill)).await;
    }

    async fn commit(&self) -> DbResult<()> {
        Ok(())
    }

    async fn rollback(&self) -> DbResult<()> {
        Ok(())
    }

    fn in_transaction(&self) -> bool {
        false
    }

    fn supports_transactions(&self) -> bool {
        false
    }

    fn set_auto_commit(&self, _auto_commit: bool) {}

    async fn children(&self, path: &[String]) -> DbResult<Vec<SchemaNode>> {
        match path {
            [] => {
                let result = self
                    .internal(
                        "SELECT name FROM system.databases \
                         ORDER BY name IN ('system', 'INFORMATION_SCHEMA', 'information_schema'), name",
                    )
                    .await?;
                Ok(result
                    .rows
                    .iter()
                    .map(|row| SchemaNode::branch(cell(row, 0), "database"))
                    .collect())
            }
            [_database] => Ok([TABLES, VIEWS, DICTIONARIES]
                .iter()
                .map(|group| SchemaNode::branch(*group, "group"))
                .collect()),
            [database, group] if group == DICTIONARIES => {
                let result = self
                    .internal(&format!(
                        "SELECT name, toString(status) FROM system.dictionaries WHERE database = {} ORDER BY name",
                        lit(database)
                    ))
                    .await?;
                Ok(result
                    .rows
                    .iter()
                    .map(|row| SchemaNode::leaf(cell(row, 0), "dictionary").detail(cell(row, 1)))
                    .collect())
            }
            [database, group] if group == TABLES || group == VIEWS => {
                let is_view = group == VIEWS;
                let result = self
                    .internal(&format!(
                        "SELECT name, engine, sorting_key, partition_key FROM system.tables \
                         WHERE database = {} AND engine != 'Dictionary' AND (engine LIKE '%View') = {} ORDER BY name",
                        lit(database),
                        if is_view { 1 } else { 0 }
                    ))
                    .await?;
                Ok(result
                    .rows
                    .iter()
                    .map(|row| {
                        // Engine, ORDER BY and partition key: what you need to
                        // know to query a ClickHouse table well.
                        let mut detail = cell(row, 1);
                        if !cell(row, 2).is_empty() {
                            detail.push_str(&format!(" · ORDER BY {}", cell(row, 2)));
                        }
                        if !cell(row, 3).is_empty() {
                            detail.push_str(&format!(" · PARTITION BY {}", cell(row, 3)));
                        }
                        SchemaNode::branch(cell(row, 0), if is_view { "view" } else { "table" })
                            .detail(detail)
                    })
                    .collect())
            }
            [database, group, table] if group == TABLES || group == VIEWS => {
                let result = self
                    .internal(&format!(
                        "SELECT name, type, is_in_sorting_key FROM system.columns \
                         WHERE database = {} AND table = {} ORDER BY position",
                        lit(database),
                        lit(table)
                    ))
                    .await?;
                Ok(result
                    .rows
                    .iter()
                    .map(|row| {
                        let mut detail = cell(row, 1);
                        if cell(row, 2) == "1" {
                            detail.push_str(" · sorting key");
                        }
                        SchemaNode::leaf(cell(row, 0), "column").detail(detail)
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    async fn ddl(&self, path: &[String]) -> DbResult<String> {
        let [database, group, name, ..] = path else {
            return Err(DbError::other("Select a table, view or dictionary."));
        };
        let what = if group == DICTIONARIES {
            "DICTIONARY"
        } else {
            "TABLE"
        };
        let result = self
            .internal(&format!(
                "SHOW CREATE {what} {}.{}",
                quote_ident(DbKind::Clickhouse, database),
                quote_ident(DbKind::Clickhouse, name)
            ))
            .await?;
        let statement = result
            .rows
            .first()
            .map(|row| cell(row, 0))
            .unwrap_or_default();
        Ok(format!("{};", statement.trim_end()))
    }

    async fn snapshot(&self) -> DbResult<SchemaSnapshot> {
        const USER_DATABASES: &str =
            "database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema')";
        let views: std::collections::HashSet<(String, String)> = self
            .internal(&format!(
                "SELECT database, name FROM system.tables WHERE engine LIKE '%View' AND {USER_DATABASES}"
            ))
            .await?
            .rows
            .iter()
            .map(|row| (cell(row, 0), cell(row, 1)))
            .collect();

        let mut tables: Vec<SnapshotTable> = Vec::new();
        let columns = self
            .internal(&format!(
                "SELECT database, table, name, type, is_in_primary_key FROM system.columns \
                 WHERE {USER_DATABASES} ORDER BY database, table, position"
            ))
            .await?;
        for row in &columns.rows {
            let (schema, name) = (cell(row, 0), cell(row, 1));
            if tables
                .last()
                .is_none_or(|t| t.schema != schema || t.name != name)
            {
                let kind = if views.contains(&(schema.clone(), name.clone())) {
                    "view"
                } else {
                    "table"
                };
                tables.push(SnapshotTable {
                    schema,
                    name,
                    kind,
                    ..Default::default()
                });
            }
            tables
                .last_mut()
                .expect("just pushed")
                .columns
                .push(SnapshotColumn {
                    name: cell(row, 2),
                    type_name: cell(row, 3),
                    primary_key: cell(row, 4) == "1",
                });
        }

        // `syntax` holds the argument hint on recent servers; older ones lack the column.
        let functions = match self
            .internal("SELECT name, syntax FROM system.functions ORDER BY name")
            .await
        {
            Ok(result) => result,
            Err(_) => {
                self.internal("SELECT name, '' FROM system.functions ORDER BY name")
                    .await?
            }
        };
        Ok(SchemaSnapshot {
            search_path: vec![if self.database.is_empty() {
                "default".into()
            } else {
                self.database.clone()
            }],
            tables,
            functions: functions
                .rows
                .iter()
                .map(|row| {
                    let (name, syntax) = (cell(row, 0), cell(row, 1));
                    // "name(args)" → "(args)"
                    let signature = syntax
                        .find('(')
                        .map(|open| syntax[open..].trim().to_string())
                        .unwrap_or_default();
                    SnapshotFunction { name, signature }
                })
                .collect(),
        })
    }

    async fn details(&self, _samples: bool) -> DbResult<SchemaDetails> {
        const USER_DATABASES: &str =
            "database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema')";
        let mut details = SchemaDetails {
            server_version: self
                .internal("SELECT version()")
                .await?
                .rows
                .first()
                .map(|row| cell(row, 0))
                .unwrap_or_default(),
            ..Default::default()
        };
        let tables = self
            .internal(&format!(
                "SELECT database, name, engine, sorting_key, partition_key, toString(total_rows), comment \
                 FROM system.tables WHERE {USER_DATABASES} ORDER BY database, name"
            ))
            .await?;
        for row in &tables.rows {
            let mut storage = cell(row, 2);
            if !cell(row, 3).is_empty() {
                storage.push_str(&format!(" ORDER BY ({})", cell(row, 3)));
            }
            if !cell(row, 4).is_empty() {
                storage.push_str(&format!(" PARTITION BY {}", cell(row, 4)));
            }
            details.tables.push(TableDetails {
                schema: cell(row, 0),
                name: cell(row, 1),
                storage,
                row_estimate: cell(row, 5).parse().ok(),
                comment: cell(row, 6),
                ..Default::default()
            });
        }
        let columns = self
            .internal(&format!(
                "SELECT database, table, name, type, comment FROM system.columns \
                 WHERE {USER_DATABASES} ORDER BY database, table, position"
            ))
            .await?;
        for row in &columns.rows {
            let (schema, name) = (cell(row, 0), cell(row, 1));
            if let Some(owner) = details
                .tables
                .iter_mut()
                .find(|t| t.schema == schema && t.name == name)
            {
                owner.columns.push(ColumnDetails {
                    name: cell(row, 2),
                    nullable: cell(row, 3).contains("Nullable("),
                    comment: cell(row, 4),
                    values: Vec::new(),
                });
            }
        }
        Ok(details)
    }

    async fn export(&self, sql: &str, mut sink: Box<dyn RowSink>) -> DbResult<u64> {
        let io = |e: std::io::Error| DbError::other(format!("Could not write the export: {e}"));
        let _turn = self.gate.lock().await;
        let query_id = uuid::Uuid::new_v4().to_string();
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = Some(query_id.clone());

        let work = async {
            // Same settings as a normal run, minus the row limit.
            let mut level = self.settings_level.load(Ordering::SeqCst);
            let response = loop {
                let response = self
                    .request(sql, &query_id, 0, level, true)
                    .send()
                    .await
                    .map_err(transport_error)?;
                if response.status().is_success() {
                    break response;
                }
                let code = response
                    .headers()
                    .get("X-ClickHouse-Exception-Code")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let error =
                    parse_exception(&response.text().await.unwrap_or_default(), code.as_deref());
                let refused_settings = error.code.as_deref() == Some("164")
                    && error.message.to_lowercase().contains("setting")
                    && level < SETTINGS_NONE;
                if !refused_settings {
                    return Err(error);
                }
                level = if level == SETTINGS_ALL && self.read_only {
                    SETTINGS_READONLY_ONLY
                } else {
                    SETTINGS_NONE
                };
                self.settings_level.store(level, Ordering::SeqCst);
            };
            let format = response
                .headers()
                .get("X-ClickHouse-Format")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            if format.as_deref().is_some_and(|f| f != FORMAT) {
                return Err(DbError::other(
                    "Remove the FORMAT clause to export: Rowboat writes the file format itself.",
                ));
            }

            let mut stream = response.bytes_stream();
            let mut decoder = Decoder::new();
            let mut started = false;
            let mut written = 0u64;
            let mut flush = |decoder: &Decoder,
                             rows: Vec<Vec<Option<String>>>,
                             started: &mut bool|
             -> DbResult<()> {
                if !*started && decoder.has_header() {
                    if decoder.columns.is_empty() {
                        return Err(DbError::other("This statement returns no rows to export."));
                    }
                    sink.start(&decoder.columns).map_err(io)?;
                    *started = true;
                }
                for row in &rows {
                    sink.row(row).map_err(io)?;
                }
                written += rows.len() as u64;
                Ok(())
            };
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(transport_error)?;
                let rows = decoder.push(&chunk, 0).map_err(DbError::other)?;
                flush(&decoder, rows, &mut started)?;
            }
            let rows = decoder.finish(0).map_err(DbError::other)?;
            flush(&decoder, rows, &mut started)?;
            if let Some(text) = decoder.exception() {
                return Err(parse_exception(&text, None));
            }
            if !started {
                return Err(DbError::other("This statement returns no rows to export."));
            }
            sink.finish().map_err(io)?;
            Ok(written)
        };
        let result = tokio::select! {
            result = work => result,
            () = self.cancelled.notified() => Err(DbError::cancelled()),
        };
        *self.running.lock().unwrap_or_else(|e| e.into_inner()) = None;
        result
    }

    async fn close(&self) {}
}

/// Lets tests share one [`ChSession`] across tasks.
pub type SharedChSession = Arc<ChSession>;

#[cfg(test)]
mod tests {
    use super::super::ch_rowbinary::testing::{header, string};
    use super::*;
    use crate::connections::ConnectionConfig;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// A request as the fake server saw it.
    #[derive(Debug, Clone, Default)]
    struct Seen {
        params: HashMap<String, String>,
        headers: HashMap<String, String>,
        sql: String,
    }

    enum Reply {
        /// 200 with a RowBinary body.
        Rows(Vec<u8>),
        /// 200 with a body in a format the user chose.
        Text(&'static str, &'static str),
        /// An exception: HTTP status, ClickHouse code, text.
        Fail(u16, &'static str, &'static str),
        /// 200, then an endless stream of UInt64 rows.
        Endless,
        /// Accepts the request and never answers.
        Hang,
        /// 200 with a written-rows summary and no body.
        Written(u64),
    }

    struct Server {
        port: u16,
        seen: Arc<Mutex<Vec<Seen>>>,
        chunks_sent: Arc<AtomicUsize>,
    }

    impl Server {
        fn requests(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
        fn user_queries(&self) -> Vec<Seen> {
            self.requests()
                .into_iter()
                .filter(|r| r.sql != "SELECT version()")
                .collect()
        }
    }

    async fn read_request(stream: &mut TcpStream) -> Option<Seen> {
        let mut buffer = Vec::new();
        let head_end = loop {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..n]);
            if let Some(i) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
        let mut lines = head.lines();
        let target = lines.next()?.split(' ').nth(1)?.to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(k, v)| (k.to_lowercase(), v.to_string()))
            .collect();
        let length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        while buffer.len() < head_end + length {
            let mut chunk = [0u8; 4096];
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..n]);
        }
        let url = reqwest::Url::parse(&format!("http://x{target}")).ok()?;
        Some(Seen {
            params: url.query_pairs().into_owned().collect(),
            headers,
            sql: String::from_utf8_lossy(&buffer[head_end..]).into_owned(),
        })
    }

    async fn chunk(stream: &mut TcpStream, data: &[u8]) -> std::io::Result<()> {
        stream
            .write_all(format!("{:x}\r\n", data.len()).as_bytes())
            .await?;
        stream.write_all(data).await?;
        stream.write_all(b"\r\n").await
    }

    /// Starts a fake ClickHouse HTTP endpoint. `script` decides each reply.
    async fn serve(script: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let chunks_sent = Arc::new(AtomicUsize::new(0));
        let script = Arc::new(script);
        let (log, counter) = (seen.clone(), chunks_sent.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let (log, counter, script) = (log.clone(), counter.clone(), script.clone());
                tokio::spawn(async move {
                    while let Some(request) = read_request(&mut stream).await {
                        log.lock().unwrap().push(request.clone());
                        let reply = if request.sql == "SELECT version()" {
                            let mut body = header(&[("version()", "String")]);
                            string("26.8.1.1", &mut body);
                            Reply::Rows(body)
                        } else {
                            script(&request)
                        };
                        let ok = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n";
                        let done = match reply {
                            Reply::Rows(body) => {
                                let head = format!("{ok}X-ClickHouse-Format: {FORMAT}\r\n\r\n");
                                stream.write_all(head.as_bytes()).await.is_ok()
                                    && chunk(&mut stream, &body).await.is_ok()
                                    && chunk(&mut stream, b"").await.is_ok()
                            }
                            Reply::Text(format, body) => {
                                let head = format!("{ok}X-ClickHouse-Format: {format}\r\n\r\n");
                                stream.write_all(head.as_bytes()).await.is_ok()
                                    && chunk(&mut stream, body.as_bytes()).await.is_ok()
                                    && chunk(&mut stream, b"").await.is_ok()
                            }
                            Reply::Written(rows) => {
                                let head = format!(
                                    "{ok}X-ClickHouse-Format: {FORMAT}\r\nX-ClickHouse-Summary: {{\"written_rows\":\"{rows}\"}}\r\n\r\n"
                                );
                                stream.write_all(head.as_bytes()).await.is_ok()
                                    && chunk(&mut stream, b"").await.is_ok()
                            }
                            Reply::Fail(status, code, text) => {
                                let head = format!(
                                    "HTTP/1.1 {status} Error\r\nX-ClickHouse-Exception-Code: {code}\r\nContent-Length: {}\r\n\r\n{text}",
                                    text.len()
                                );
                                stream.write_all(head.as_bytes()).await.is_ok()
                            }
                            Reply::Endless => {
                                let head = format!("{ok}X-ClickHouse-Format: {FORMAT}\r\n\r\n");
                                let _ = stream.write_all(head.as_bytes()).await;
                                let _ = chunk(&mut stream, &header(&[("n", "UInt64")])).await;
                                let mut n = 0u64;
                                loop {
                                    let mut block = Vec::new();
                                    for _ in 0..500 {
                                        block.extend(n.to_le_bytes());
                                        n += 1;
                                    }
                                    if chunk(&mut stream, &block).await.is_err() {
                                        break; // the client hung up
                                    }
                                    counter.fetch_add(1, Ordering::SeqCst);
                                    tokio::time::sleep(Duration::from_millis(1)).await;
                                }
                                false
                            }
                            Reply::Hang => {
                                tokio::time::sleep(Duration::from_secs(120)).await;
                                false
                            }
                        };
                        if !done {
                            break;
                        }
                    }
                });
            }
        });
        Server {
            port,
            seen,
            chunks_sent,
        }
    }

    fn params(port: u16) -> ConnectParams {
        ConnectParams {
            config: ConnectionConfig {
                name: "ch".into(),
                kind: DbKind::Clickhouse,
                host: "127.0.0.1".into(),
                port,
                database: "analytics".into(),
                user: "reader".into(),
                tls: crate::connections::Tls {
                    mode: TlsMode::Disable,
                    ..Default::default()
                },
                ..Default::default()
            },
            password: Some("s3cret".into()),
            client_id: "rowboat-dominic".into(),
            connect_timeout: Duration::from_secs(5),
            keepalive: Duration::from_secs(30),
            auto_commit: true,
            auto_reconnect: true,
            via: None,
        }
    }

    fn numbers(count: u64) -> Vec<u8> {
        let mut body = header(&[("n", "UInt64")]);
        for n in 0..count {
            body.extend(n.to_le_bytes());
        }
        body
    }

    #[tokio::test]
    async fn sends_identity_credentials_and_format_the_documented_way() {
        let server = serve(|_| Reply::Rows(numbers(3))).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        assert_eq!(session.server_version(), "ClickHouse 26.8.1.1");

        let result = session
            .execute(
                "SELECT number AS n FROM numbers(3)",
                ExecOptions::limit(1000),
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 3);
        assert!(!result.more_available);

        let request = &server.user_queries()[0];
        // The user's SQL is the body, untouched.
        assert_eq!(request.sql, "SELECT number AS n FROM numbers(3)");
        // Client ID as the client name, and as log_comment.
        assert_eq!(request.headers["user-agent"], "rowboat-dominic");
        assert_eq!(request.params["log_comment"], "rowboat-dominic");
        // Credentials travel in the Authorization header, never in the URL.
        assert_eq!(
            request.headers["authorization"],
            "Basic cmVhZGVyOnMzY3JldA=="
        ); // reader:s3cret
        assert!(!request.params.values().any(|v| v.contains("s3cret")));
        // Binary rows, compression, and the fetch-time row limit.
        assert_eq!(request.headers["x-clickhouse-format"], FORMAT);
        assert_eq!(request.params["enable_http_compression"], "1");
        assert!(request.headers["accept-encoding"].contains("gzip"));
        assert_eq!(request.params["max_result_rows"], "1000");
        assert_eq!(request.params["result_overflow_mode"], "break");
        assert_eq!(request.params["database"], "analytics");
        assert!(request.params.contains_key("session_id"));
        assert!(!request.params.contains_key("readonly"));
    }

    #[tokio::test]
    async fn passwords_with_any_characters_can_connect() {
        let server = serve(|_| Reply::Rows(numbers(1))).await;
        let mut p = params(server.port);
        p.password = Some("pässwörd-密码".into());
        let session = ChSession::connect(p).await.expect("non-ASCII password");
        session
            .execute("SELECT 1", ExecOptions::limit(0))
            .await
            .unwrap();
        // base64("reader:pässwörd-密码")
        assert_eq!(
            server.user_queries()[0].headers["authorization"],
            "Basic cmVhZGVyOnDDpHNzd8O2cmQt5a+G56CB"
        );
    }

    #[tokio::test]
    async fn the_database_is_set_once_so_use_keeps_working() {
        let server = serve(|_| Reply::Rows(numbers(1))).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let all = ExecOptions::limit(0);
        session.execute("SELECT 1", all).await.unwrap();
        session.execute("USE other_db", all).await.unwrap();
        session.execute("SELECT 2", all).await.unwrap();

        let queries = server.user_queries();
        assert_eq!(
            queries[0].params.get("database").map(String::as_str),
            Some("analytics")
        );
        // Later statements in the session must not reset the database.
        assert!(!queries[1].params.contains_key("database"));
        assert!(!queries[2].params.contains_key("database"));
        assert_eq!(
            queries[2].params["session_id"],
            queries[0].params["session_id"]
        );
    }

    #[tokio::test]
    async fn a_briefly_locked_session_is_retried() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = attempts.clone();
        let server = serve(move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                Reply::Fail(
                    500,
                    "373",
                    "Code: 373. DB::Exception: Session rowboat-x is locked by a concurrent client. (SESSION_IS_LOCKED)",
                )
            } else {
                Reply::Rows(numbers(3))
            }
        })
        .await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let result = session
            .execute("SELECT 1", ExecOptions::limit(0))
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn no_limit_sends_no_row_cap() {
        let server = serve(|_| Reply::Rows(numbers(5))).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let result = session
            .execute("SELECT 1", ExecOptions::limit(0))
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 5);
        assert!(!server.user_queries()[0]
            .params
            .contains_key("max_result_rows"));
    }

    #[tokio::test]
    async fn row_limit_cuts_an_endless_stream_and_hangs_up() {
        let server = serve(|_| Reply::Endless).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();

        let started = std::time::Instant::now();
        let result = session
            .execute(
                "SELECT number FROM system.numbers",
                ExecOptions::limit(1000),
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1000);
        assert!(result.more_available);
        assert_eq!(result.rows[999][0].as_deref(), Some("999"));
        assert!(started.elapsed() < Duration::from_secs(2));

        // The server must notice the client is gone and stop sending.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let sent = server.chunks_sent.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            server.chunks_sent.load(Ordering::SeqCst),
            sent,
            "the stream was not closed"
        );
    }

    #[tokio::test]
    async fn read_only_connections_send_readonly_and_stay_stateless() {
        let server = serve(|_| Reply::Rows(numbers(1))).await;
        let mut p = params(server.port);
        p.config.read_only = true;
        let session = ChSession::connect(p).await.unwrap();
        session
            .execute("SELECT 1", ExecOptions::limit(10))
            .await
            .unwrap();
        let request = &server.user_queries()[0];
        assert_eq!(request.params["readonly"], "1");
        assert!(!request.params.contains_key("session_id"));
    }

    #[tokio::test]
    async fn users_who_may_not_change_settings_still_get_a_bounded_result() {
        // A profile with readonly=1 rejects any request that carries settings.
        let server = serve(|request| {
            if request.params.contains_key("max_result_rows") || request.params.contains_key("log_comment") {
                Reply::Fail(
                    500,
                    "164",
                    "Code: 164. DB::Exception: Cannot modify 'max_result_rows' setting in readonly mode. (READONLY) (version 26.8.1.1)",
                )
            } else {
                Reply::Endless
            }
        })
        .await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let result = session
            .execute("SELECT 1", ExecOptions::limit(50))
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 50);
        assert!(result.more_available);

        // The refusal is remembered: the next statement sends no settings at all.
        let before = server.user_queries().len();
        session
            .execute("SELECT 2", ExecOptions::limit(50))
            .await
            .unwrap();
        let after = server.user_queries();
        assert_eq!(after.len(), before + 1, "no second probing round trip");
        assert!(!after.last().unwrap().params.contains_key("max_result_rows"));
    }

    #[tokio::test]
    async fn exceptions_become_classified_errors() {
        let server = serve(|request| match request.sql.as_str() {
            "SELECT 1 FROMM t" => Reply::Fail(
                400,
                "62",
                "Code: 62. DB::Exception: Syntax error: failed at position 10 ('FROMM') (line 1, col 10): FROMM t. Expected one of: token, Comma. (SYNTAX_ERROR) (version 26.8.1.1 (official build))",
            ),
            "SELECT * FROM nope" => Reply::Fail(
                404,
                "60",
                "Code: 60. DB::Exception: Unknown table expression identifier 'nope' in scope SELECT * FROM nope. (UNKNOWN_TABLE) (version 26.8.1.1)",
            ),
            "INSERT INTO t VALUES (1)" => Reply::Fail(
                500,
                "164",
                "Code: 164. DB::Exception: reader: Cannot execute query in readonly mode. (READONLY) (version 26.8.1.1)",
            ),
            _ => Reply::Fail(403, "516", "Code: 516. DB::Exception: reader: Authentication failed. (AUTHENTICATION_FAILED)"),
        })
        .await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let all = ExecOptions::limit(0);

        let syntax = session.execute("SELECT 1 FROMM t", all).await.unwrap_err();
        assert_eq!(syntax.kind, ErrorKind::Syntax);
        assert_eq!(syntax.position, Some(10));
        assert_eq!(syntax.code.as_deref(), Some("62"));
        assert!(syntax
            .message
            .starts_with("Syntax error: failed at position 10"));
        assert!(!syntax.message.contains("version"));

        let schema = session
            .execute("SELECT * FROM nope", all)
            .await
            .unwrap_err();
        assert_eq!(schema.kind, ErrorKind::Schema);

        let readonly = session
            .execute("INSERT INTO t VALUES (1)", all)
            .await
            .unwrap_err();
        assert_eq!(readonly.kind, ErrorKind::Permission);
        assert_eq!(
            readonly.hint.as_deref(),
            Some("This connection is read-only.")
        );

        let auth = session.execute("SELECT 1", all).await.unwrap_err();
        assert_eq!(auth.kind, ErrorKind::Permission);
    }

    #[tokio::test]
    async fn a_refused_connection_is_a_connection_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = ChSession::connect(params(port)).await.err().unwrap();
        assert_eq!(error.kind, ErrorKind::Connection, "{error:?}");
    }

    #[tokio::test]
    async fn cancel_returns_at_once_and_kills_the_query_by_id() {
        let server = serve(|request| {
            if request.sql.starts_with("KILL QUERY") {
                Reply::Rows(header(&[]))
            } else {
                Reply::Hang
            }
        })
        .await;
        let session = Arc::new(ChSession::connect(params(server.port)).await.unwrap());
        let runner = session.clone();
        let task = tokio::spawn(async move {
            runner
                .execute("SELECT sleep(60)", ExecOptions::limit(0))
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;

        let cancelled_at = std::time::Instant::now();
        session.cancel().await;
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind, ErrorKind::Cancelled);
        assert!(cancelled_at.elapsed() < Duration::from_secs(1));

        let requests = server.user_queries();
        let query_id = &requests[0].params["query_id"];
        assert_eq!(
            requests[1].sql,
            format!("KILL QUERY WHERE query_id = '{query_id}'")
        );
        // Cancelling when nothing runs does nothing.
        session.cancel().await;
        assert_eq!(server.user_queries().len(), 2);
    }

    #[tokio::test]
    async fn a_format_clause_returns_the_servers_text() {
        let server = serve(|_| Reply::Text("JSONEachRow", "{\"n\":1}\n{\"n\":2}\n")).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let result = session
            .execute(
                "SELECT n FROM t FORMAT JSONEachRow",
                ExecOptions::limit(100),
            )
            .await
            .unwrap();
        assert_eq!(result.raw.as_deref(), Some("{\"n\":1}\n{\"n\":2}\n"));
        assert!(result.rows.is_empty() && result.columns.is_empty());
    }

    #[tokio::test]
    async fn writes_report_rows_written() {
        let server = serve(|_| Reply::Written(42)).await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let result = session
            .execute("INSERT INTO t SELECT * FROM u", ExecOptions::limit(100))
            .await
            .unwrap();
        assert_eq!(result.rows_affected, Some(42));
        assert!(!session.supports_transactions());
    }

    #[tokio::test]
    async fn an_exception_after_streaming_started_is_surfaced() {
        let server = serve(|_| {
            let mut body = numbers(2);
            body.extend_from_slice(
                b"Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED) (version 26.8.1.1)\n",
            );
            Reply::Rows(body)
        })
        .await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let error = session
            .execute("SELECT heavy", ExecOptions::limit(0))
            .await
            .unwrap_err();
        assert!(error.message.contains("Memory limit"), "{error:?}");
        assert_eq!(error.code.as_deref(), Some("241"));
    }

    #[tokio::test]
    async fn navigator_shows_engine_order_by_and_partition_key() {
        let server = serve(|request| {
            let sql = &request.sql;
            if sql.contains("system.databases") {
                let mut body = header(&[("name", "String")]);
                for name in ["analytics", "default", "system"] {
                    string(name, &mut body);
                }
                Reply::Rows(body)
            } else if sql.contains("system.tables") {
                let mut body = header(&[
                    ("name", "String"),
                    ("engine", "String"),
                    ("sorting_key", "String"),
                    ("partition_key", "String"),
                ]);
                for value in ["events", "MergeTree", "user_id, ts", "toYYYYMM(ts)"] {
                    string(value, &mut body);
                }
                Reply::Rows(body)
            } else if sql.contains("system.columns") {
                let mut body = header(&[("name", "String"), ("type", "String"), ("is_in_sorting_key", "UInt8")]);
                string("user_id", &mut body);
                string("UInt64", &mut body);
                body.push(1);
                string("payload", &mut body);
                string("Map(String, String)", &mut body);
                body.push(0);
                Reply::Rows(body)
            } else {
                let mut body = header(&[("statement", "String")]);
                string("CREATE TABLE analytics.events (user_id UInt64) ENGINE = MergeTree ORDER BY user_id", &mut body);
                Reply::Rows(body)
            }
        })
        .await;
        let session = ChSession::connect(params(server.port)).await.unwrap();
        let path =
            |parts: &[&str]| -> Vec<String> { parts.iter().map(|p| p.to_string()).collect() };

        let databases = session.children(&[]).await.unwrap();
        assert_eq!(databases.len(), 3);
        assert_eq!(databases[0].kind, "database");

        let tables = session
            .children(&path(&["analytics", "Tables"]))
            .await
            .unwrap();
        assert_eq!(
            tables[0].detail.as_deref(),
            Some("MergeTree · ORDER BY user_id, ts · PARTITION BY toYYYYMM(ts)")
        );

        let columns = session
            .children(&path(&["analytics", "Tables", "events"]))
            .await
            .unwrap();
        assert_eq!(columns[0].detail.as_deref(), Some("UInt64 · sorting key"));
        assert_eq!(columns[1].detail.as_deref(), Some("Map(String, String)"));

        let ddl = session
            .ddl(&path(&["analytics", "Tables", "events"]))
            .await
            .unwrap();
        assert!(ddl.starts_with("CREATE TABLE analytics.events") && ddl.ends_with(';'));
        let show = server
            .requests()
            .into_iter()
            .find(|r| r.sql.starts_with("SHOW CREATE"))
            .unwrap();
        assert_eq!(show.sql, "SHOW CREATE TABLE `analytics`.`events`");
        // Navigator requests stay out of the user's session.
        assert!(!show.params.contains_key("session_id"));
    }

    #[test]
    fn exception_text_is_cleaned_up() {
        let error = parse_exception(
            "Code: 47. DB::Exception: Unknown expression identifier `x` in scope SELECT x. (UNKNOWN_IDENTIFIER) (version 26.8.1.1 (official build))\n",
            None,
        );
        assert_eq!(error.kind, ErrorKind::Schema);
        assert_eq!(error.code.as_deref(), Some("47"));
        assert_eq!(
            error.message,
            "Unknown expression identifier `x` in scope SELECT x. (UNKNOWN_IDENTIFIER)"
        );
        assert_eq!(parse_exception("", None).kind, ErrorKind::Other);
    }
}

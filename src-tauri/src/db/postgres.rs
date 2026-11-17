//! PostgreSQL driver (`tokio-postgres` + rustls).
//!
//! One task owns the connection and processes commands in order, which gives
//! the session a simple, explicit transaction state machine:
//!
//! * **auto-commit** — a statement that returns rows runs in its own short
//!   transaction, because the row limit is a protocol-level cursor (a portal
//!   executed with a row count) and portals only live inside a transaction.
//!   Statements that return nothing run bare, so `VACUUM`, `CREATE DATABASE`
//!   and friends work.
//! * **in a transaction** — entered by the user's `BEGIN`, or by the first
//!   statement in manual-commit mode; left by `COMMIT` / `ROLLBACK`, typed or
//!   from the toolbar.
//!
//! The SQL the user wrote is sent exactly as written.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::{pin_mut, StreamExt, TryStreamExt};
use tokio::sync::{mpsc, oneshot};
use tokio_postgres::config::SslMode;
use tokio_postgres::tls::{MakeTlsConnect, TlsConnect};
use tokio_postgres::{AsyncMessage, CancelToken, Client, Config, NoTls, Socket, Transaction};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::pg_types::{column_kind, render, Raw};
use super::{
    quote_ident, quote_literal, Column, ConnectParams, DbError, DbKind, DbResult, ErrorKind,
    ExecOptions, QueryResult, SchemaNode, Session,
};
use crate::connections::TlsMode;
use crate::statements::{tx_control, TxControl};

type Reply<T> = oneshot::Sender<DbResult<T>>;

enum Command {
    Execute(String, ExecOptions, Reply<QueryResult>),
    /// A catalog query from the navigator: never starts a user transaction.
    Internal(String, Reply<QueryResult>),
    EndTransaction {
        commit: bool,
        reply: Reply<()>,
    },
    Close,
}

struct Shared {
    auto_commit: AtomicBool,
    in_transaction: AtomicBool,
    cancel: Mutex<Option<(CancelToken, Option<MakeRustlsConnect>)>>,
    notices: Mutex<Vec<String>>,
}

impl Shared {
    fn take_notices(&self) -> Vec<String> {
        std::mem::take(&mut *self.notices.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

pub struct PgSession {
    commands: mpsc::UnboundedSender<Command>,
    shared: Arc<Shared>,
    version: String,
}

// ------------------------------------------------------------ connecting ----

/// Looks a password up in a pgpass file's contents.
/// Format: `host:port:database:user:password`, `*` matches anything, `\`
/// escapes `:` and `\`.
pub fn pgpass_lookup(contents: &str, host: &str, port: u16, database: &str, user: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = vec![String::new()];
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(next) = chars.next() {
                        fields.last_mut().unwrap().push(next);
                    }
                }
                ':' if fields.len() < 5 => fields.push(String::new()),
                other => fields.last_mut().unwrap().push(other),
            }
        }
        if fields.len() != 5 {
            continue;
        }
        let matches = |pattern: &str, value: &str| pattern == "*" || pattern == value;
        if matches(&fields[0], host)
            && matches(&fields[1], &port.to_string())
            && matches(&fields[2], database)
            && matches(&fields[3], user)
        {
            return Some(fields.swap_remove(4));
        }
    }
    None
}

fn pgpass_file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("PGPASSFILE") {
        return Some(path.into());
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|dir| std::path::PathBuf::from(dir).join("postgresql").join("pgpass.conf"))
    } else {
        std::env::var_os("HOME").map(|dir| std::path::PathBuf::from(dir).join(".pgpass"))
    }
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
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

fn map_error(error: tokio_postgres::Error) -> DbError {
    if let Some(db) = error.as_db_error() {
        let code = db.code().code();
        let kind = match code {
            "57014" => ErrorKind::Cancelled,
            "42601" => ErrorKind::Syntax,
            "42501" => ErrorKind::Permission,
            "25006" => ErrorKind::Permission, // read-only transaction
            c if c.starts_with("28") => ErrorKind::Permission,
            c if c.starts_with("42") || c == "3D000" || c == "3F000" => ErrorKind::Schema,
            c if c.starts_with("08") || c.starts_with("57P") => ErrorKind::Connection,
            _ => ErrorKind::Other,
        };
        let position = match db.position() {
            Some(tokio_postgres::error::ErrorPosition::Original(p)) => Some(*p),
            _ => None,
        };
        return DbError {
            kind,
            message: if kind == ErrorKind::Cancelled {
                "Query cancelled.".into()
            } else {
                db.message().to_string()
            },
            detail: db.detail().map(str::to_string),
            hint: db.hint().map(str::to_string).or_else(|| {
                (code == "25006").then(|| "This connection is read-only.".to_string())
            }),
            code: Some(code.to_string()),
            position,
        };
    }
    let message = error_chain(&error);
    if error.is_closed() {
        return DbError::connection("The connection to the server was lost.").with_detail(message);
    }
    let lower = message.to_lowercase();
    let connectionish = [
        "connect", "tls", "certificate", "timed out", "refused", "reset", "broken pipe", "eof",
        "dns", "resolve", "handshake",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    DbError::new(
        if connectionish {
            ErrorKind::Connection
        } else {
            ErrorKind::Other
        },
        message,
    )
}

/// Drives the connection and collects server notices (`RAISE NOTICE`, warnings).
fn drive<S, T>(mut connection: tokio_postgres::Connection<S, T>, shared: Arc<Shared>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut messages = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(message) = messages.next().await {
            match message {
                Ok(AsyncMessage::Notice(notice)) => {
                    let mut notices = shared.notices.lock().unwrap_or_else(|e| e.into_inner());
                    if notices.len() < 200 {
                        notices.push(format!("{}: {}", notice.severity(), notice.message()));
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

async fn connect_with<T>(config: &Config, tls: T, shared: &Arc<Shared>) -> Result<Client, tokio_postgres::Error>
where
    T: MakeTlsConnect<Socket> + 'static,
    T::Stream: Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    let (client, connection) = config.connect(tls).await?;
    drive(connection, shared.clone());
    Ok(client)
}

async fn open(params: &ConnectParams, shared: &Arc<Shared>) -> DbResult<Client> {
    let c = &params.config;
    let user = if c.user.is_empty() { "postgres" } else { &c.user };
    let database = if c.database.is_empty() { user } else { &c.database };

    let mut config = Config::new();
    config
        .host(&c.host)
        // Through an SSH tunnel the socket goes to the local forward; `host`
        // stays, so TLS still verifies the real server name.
        .port(params.via.map_or(c.port, |via| via.port()))
        .user(user)
        .dbname(database)
        .application_name(&params.client_id)
        .connect_timeout(params.connect_timeout);

    let password = params.password.clone().filter(|p| !p.is_empty()).or_else(|| {
        if !c.use_pgpass {
            return None;
        }
        let contents = std::fs::read_to_string(pgpass_file()?).ok()?;
        pgpass_lookup(&contents, &c.host, c.port, database, user)
    });
    if let Some(password) = password {
        config.password(password);
    }
    if let Some(via) = params.via {
        config.hostaddr(via.ip());
    }
    if params.keepalive.is_zero() {
        config.keepalives(false);
    } else {
        config.keepalives(true).keepalives_idle(params.keepalive);
    }
    if c.read_only {
        config.options("-c default_transaction_read_only=on");
    }

    let (client, rustls) = match c.tls.mode {
        TlsMode::Disable => {
            config.ssl_mode(SslMode::Disable);
            (connect_with(&config, NoTls, shared).await, None)
        }
        mode => {
            config.ssl_mode(if mode == TlsMode::Prefer {
                SslMode::Prefer
            } else {
                SslMode::Require
            });
            let tls = MakeRustlsConnect::new(crate::tls::client_config(&c.tls)?);
            (
                connect_with(&config, tls.clone(), shared).await,
                Some(tls),
            )
        }
    };
    let client = client.map_err(|e| {
        let mut error = map_error(e);
        if error.kind == ErrorKind::Other {
            error.kind = ErrorKind::Connection;
        }
        error
    })?;
    *shared.cancel.lock().unwrap_or_else(|e| e.into_inner()) = Some((client.cancel_token(), rustls));
    Ok(client)
}

impl PgSession {
    pub async fn connect(params: ConnectParams) -> DbResult<Self> {
        let shared = Arc::new(Shared {
            auto_commit: AtomicBool::new(params.auto_commit),
            in_transaction: AtomicBool::new(false),
            cancel: Mutex::new(None),
            notices: Mutex::new(Vec::new()),
        });
        let client = open(&params, &shared).await?;
        let version = client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(map_error)?
            .get::<_, String>(0);
        shared.take_notices();

        let (commands, inbox) = mpsc::unbounded_channel();
        tokio::spawn(serve(client, inbox, shared.clone(), params));
        Ok(Self {
            commands,
            shared,
            version,
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

    async fn catalog(&self, sql: String) -> DbResult<Vec<Vec<Option<String>>>> {
        Ok(self
            .request(|reply| Command::Internal(sql, reply))
            .await?
            .rows)
    }
}

// ------------------------------------------------------------- executing ----

fn is_stream_copy(sql: &str) -> bool {
    let upper = sql.trim_start().to_ascii_uppercase();
    upper.starts_with("COPY") && (upper.contains("STDIN") || upper.contains("STDOUT"))
}

async fn fetch(tx: &Transaction<'_>, sql: &str, options: ExecOptions) -> Result<QueryResult, tokio_postgres::Error> {
    let statement = tx.prepare(sql).await?;
    if statement.columns().is_empty() {
        let affected = tx.execute(&statement, &[]).await?;
        return Ok(QueryResult {
            rows_affected: Some(affected),
            ..Default::default()
        });
    }
    let columns = statement
        .columns()
        .iter()
        .map(|c| Column {
            name: c.name().to_string(),
            type_name: c.type_().name().to_string(),
            kind: column_kind(c.type_()),
        })
        .collect();

    // The server-side cursor: ask the portal for one row more than the limit,
    // so "more available" is known without fetching the rest.
    let limit = options.row_limit as usize;
    let ask = if limit == 0 { 0 } else { (limit + 1).min(i32::MAX as usize) as i32 };
    let portal = tx.bind(&statement, &[]).await?;
    let stream = tx.query_portal_raw(&portal, ask).await?;
    pin_mut!(stream);

    let mut rows = Vec::new();
    let mut more = false;
    while let Some(row) = stream.try_next().await? {
        if limit > 0 && rows.len() == limit {
            more = true;
            continue; // the one extra row; drain so the protocol stays in step
        }
        let mut cells = Vec::with_capacity(row.len());
        for (i, column) in row.columns().iter().enumerate() {
            let raw: Raw = row.try_get(i)?;
            cells.push(raw.0.map(|bytes| render(column.type_(), bytes)));
        }
        rows.push(cells);
    }
    Ok(QueryResult {
        columns,
        rows,
        more_available: more,
        ..Default::default()
    })
}

/// Runs a statement outside any user transaction.
async fn run_auto_commit(client: &mut Client, sql: &str, options: ExecOptions) -> Result<QueryResult, tokio_postgres::Error> {
    let statement = client.prepare(sql).await?;
    if statement.columns().is_empty() {
        let affected = client.execute(&statement, &[]).await?;
        return Ok(QueryResult {
            rows_affected: Some(affected),
            ..Default::default()
        });
    }
    let tx = client.transaction().await?;
    let result = fetch(&tx, sql, options).await?;
    tx.commit().await?;
    Ok(result)
}

fn finish(result: Result<QueryResult, tokio_postgres::Error>, shared: &Shared, in_transaction: bool) -> DbResult<QueryResult> {
    let notices = shared.take_notices();
    result
        .map(|mut r| {
            r.notices = notices;
            r.in_transaction = in_transaction;
            r
        })
        .map_err(map_error)
}

fn note(text: &str, in_transaction: bool) -> QueryResult {
    QueryResult {
        notices: vec![text.to_string()],
        in_transaction,
        ..Default::default()
    }
}

/// `SET TRANSACTION …` carrying the modes of the user's `BEGIN …`, if any.
fn transaction_modes(begin_sql: &str) -> Option<String> {
    let text = begin_sql.trim().trim_end_matches(';').trim();
    let upper = text.to_ascii_uppercase();
    let rest = if let Some(rest) = upper.strip_prefix("BEGIN") {
        &text[text.len() - rest.len()..]
    } else if let Some(rest) = upper.strip_prefix("START TRANSACTION") {
        &text[text.len() - rest.len()..]
    } else {
        return None;
    };
    let rest = rest.trim();
    let rest_upper = rest.to_ascii_uppercase();
    let rest = ["TRANSACTION", "WORK"]
        .iter()
        .find_map(|word| rest_upper.strip_prefix(word).map(|r| &rest[rest.len() - r.len()..]))
        .unwrap_or(rest)
        .trim();
    (!rest.is_empty()).then(|| format!("SET TRANSACTION {rest}"))
}

async fn serve(
    mut client: Client,
    mut inbox: mpsc::UnboundedReceiver<Command>,
    shared: Arc<Shared>,
    params: ConnectParams,
) {
    while let Some(command) = inbox.recv().await {
        // Reconnect transparently between statements if the server went away.
        let mut reconnected = false;
        if client.is_closed() && !matches!(command, Command::Close) {
            let attempt = if params.auto_reconnect {
                open(&params, &shared).await
            } else {
                Err(DbError::connection("The connection to the server was lost."))
            };
            match attempt {
                Ok(fresh) => {
                    client = fresh;
                    reconnected = true;
                    shared.take_notices();
                }
                Err(error) => {
                    match command {
                        Command::Execute(_, _, reply) | Command::Internal(_, reply) => {
                            let _ = reply.send(Err(error));
                        }
                        Command::EndTransaction { reply, .. } => {
                            let _ = reply.send(Err(error));
                        }
                        Command::Close => {}
                    }
                    continue;
                }
            }
        }
        let with_reconnect_note = |mut result: DbResult<QueryResult>| {
            if reconnected {
                if let Ok(r) = &mut result {
                    r.notices.insert(0, "Reconnected to the server.".into());
                }
            }
            result
        };

        match command {
            Command::Close => break,
            Command::EndTransaction { reply, .. } => {
                let _ = reply.send(Ok(()));
            }
            Command::Internal(sql, reply) => {
                let result = run_auto_commit(&mut client, &sql, ExecOptions { row_limit: 0 }).await;
                shared.take_notices();
                let _ = reply.send(result.map_err(map_error));
            }
            Command::Execute(sql, options, reply) => {
                if is_stream_copy(&sql) {
                    let _ = reply.send(Err(DbError::other(
                        "COPY to or from the client is not supported yet. Use a server-side file, or Export.",
                    )));
                    continue;
                }
                let manual = !shared.auto_commit.load(Ordering::SeqCst);
                match tx_control(DbKind::Postgres, &sql) {
                    TxControl::Commit | TxControl::Rollback => {
                        let _ = reply.send(with_reconnect_note(Ok(note(
                            "There is no transaction in progress.",
                            false,
                        ))));
                    }
                    TxControl::Begin => {
                        in_transaction(&mut client, &mut inbox, &shared, Some(&sql), None, reply).await;
                    }
                    TxControl::None if manual => {
                        in_transaction(&mut client, &mut inbox, &shared, None, Some((sql, options)), reply)
                            .await;
                    }
                    TxControl::None => {
                        let result = run_auto_commit(&mut client, &sql, options).await;
                        let _ = reply.send(with_reconnect_note(finish(result, &shared, false)));
                    }
                }
            }
        }
    }
}

/// Runs the session while a user transaction is open. Returns when it ends.
async fn in_transaction(
    client: &mut Client,
    inbox: &mut mpsc::UnboundedReceiver<Command>,
    shared: &Arc<Shared>,
    begin_sql: Option<&str>,
    first: Option<(String, ExecOptions)>,
    reply: Reply<QueryResult>,
) {
    let tx = match client.transaction().await {
        Ok(tx) => tx,
        Err(error) => {
            let _ = reply.send(Err(map_error(error)));
            return;
        }
    };
    shared.in_transaction.store(true, Ordering::SeqCst);

    let opening = match (begin_sql.and_then(transaction_modes), first) {
        (Some(modes), _) => tx
            .batch_execute(&modes)
            .await
            .map(|()| QueryResult::default()),
        (None, Some((sql, options))) => fetch(&tx, &sql, options).await,
        (None, None) => Ok(QueryResult::default()),
    };
    let lost = matches!(&opening, Err(e) if e.is_closed());
    let _ = reply.send(finish(opening, shared, !lost));

    let mut tx = Some(tx);
    while !lost {
        let Some(command) = inbox.recv().await else { break };
        let current = tx.as_ref().expect("transaction is open");
        match command {
            Command::Close => break,
            Command::Internal(sql, reply) => {
                let result = fetch(current, &sql, ExecOptions { row_limit: 0 }).await;
                shared.take_notices();
                let _ = reply.send(result.map_err(map_error));
            }
            Command::EndTransaction { commit, reply } => {
                let tx = tx.take().expect("transaction is open");
                let result = if commit { tx.commit().await } else { tx.rollback().await };
                shared.take_notices();
                let _ = reply.send(result.map_err(map_error));
                break;
            }
            Command::Execute(sql, options, reply) => match tx_control(DbKind::Postgres, &sql) {
                TxControl::Begin => {
                    let _ = reply.send(Ok(note("There is already a transaction in progress.", true)));
                }
                control @ (TxControl::Commit | TxControl::Rollback) => {
                    let tx = tx.take().expect("transaction is open");
                    let result = if control == TxControl::Commit {
                        tx.commit().await
                    } else {
                        tx.rollback().await
                    };
                    let _ = reply.send(finish(result.map(|()| QueryResult::default()), shared, false));
                    break;
                }
                TxControl::None => {
                    let result = fetch(current, &sql, options).await;
                    let closed = matches!(&result, Err(e) if e.is_closed());
                    let _ = reply.send(finish(result, shared, !closed));
                    if closed {
                        break;
                    }
                }
            },
        }
    }
    // Dropping an open transaction rolls it back.
    drop(tx);
    shared.in_transaction.store(false, Ordering::SeqCst);
}

// ------------------------------------------------------------- navigator ----

const TABLES: &str = "Tables";
const VIEWS: &str = "Views";
const MATVIEWS: &str = "Materialized views";
const FUNCTIONS: &str = "Functions";
const SEQUENCES: &str = "Sequences";
const TYPES: &str = "Types";

fn lit(value: &str) -> String {
    quote_literal(DbKind::Postgres, value)
}

fn ident(value: &str) -> String {
    quote_ident(DbKind::Postgres, value)
}

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// Shows an estimated row count compactly: `~1.2M rows`.
fn row_estimate(text: &str) -> String {
    let Ok(n) = text.parse::<f64>() else {
        return String::new();
    };
    if n < 0.0 {
        return String::new(); // never analyzed
    }
    let (value, unit) = if n >= 1e9 {
        (n / 1e9, "B")
    } else if n >= 1e6 {
        (n / 1e6, "M")
    } else if n >= 1e3 {
        (n / 1e3, "k")
    } else {
        (n, "")
    };
    if unit.is_empty() {
        format!("~{value:.0} rows")
    } else {
        format!("~{value:.1}{unit} rows")
    }
}

#[async_trait]
impl Session for PgSession {
    fn kind(&self) -> DbKind {
        DbKind::Postgres
    }

    fn server_version(&self) -> String {
        format!("PostgreSQL {}", self.version)
    }

    async fn execute(&self, sql: &str, options: ExecOptions) -> DbResult<QueryResult> {
        let sql = sql.to_string();
        self.request(|reply| Command::Execute(sql, options, reply))
            .await
    }

    async fn cancel(&self) {
        let token = self.shared.cancel.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((token, tls)) = token {
            let _ = match tls {
                Some(tls) => token.cancel_query(tls).await,
                None => token.cancel_query(NoTls).await,
            };
        }
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
            [] => {
                let rows = self
                    .catalog(
                        "SELECT nspname FROM pg_catalog.pg_namespace \
                         WHERE nspname !~ '^pg_(toast|temp)' \
                         ORDER BY nspname IN ('pg_catalog', 'information_schema'), nspname"
                            .into(),
                    )
                    .await?;
                Ok(rows
                    .iter()
                    .map(|row| SchemaNode::branch(cell(row, 0), "schema"))
                    .collect())
            }
            [_schema] => Ok([TABLES, VIEWS, MATVIEWS, FUNCTIONS, SEQUENCES, TYPES]
                .iter()
                .map(|group| SchemaNode::branch(*group, "group"))
                .collect()),
            [schema, group] => {
                let relations = |kinds: &str| {
                    format!(
                        "SELECT c.relname, c.reltuples::bigint::text \
                         FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = {} AND c.relkind IN ({kinds}) ORDER BY c.relname",
                        lit(schema)
                    )
                };
                match group.as_str() {
                    TABLES | VIEWS | MATVIEWS => {
                        let (kinds, kind) = match group.as_str() {
                            TABLES => ("'r', 'p', 'f'", "table"),
                            VIEWS => ("'v'", "view"),
                            _ => ("'m'", "matview"),
                        };
                        let rows = self.catalog(relations(kinds)).await?;
                        Ok(rows
                            .iter()
                            .map(|row| {
                                let node = SchemaNode::branch(cell(row, 0), kind);
                                if kind == "view" {
                                    node
                                } else {
                                    node.detail(row_estimate(&cell(row, 1)))
                                }
                            })
                            .collect())
                    }
                    SEQUENCES => {
                        let rows = self.catalog(relations("'S'")).await?;
                        Ok(rows
                            .iter()
                            .map(|row| SchemaNode::leaf(cell(row, 0), "sequence"))
                            .collect())
                    }
                    FUNCTIONS => {
                        let rows = self
                            .catalog(format!(
                                "SELECT p.proname || '(' || pg_catalog.oidvectortypes(p.proargtypes) || ')', \
                                        CASE p.prokind WHEN 'p' THEN 'procedure' WHEN 'a' THEN 'aggregate' \
                                             ELSE pg_catalog.pg_get_function_result(p.oid) END \
                                 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                                 WHERE n.nspname = {} ORDER BY 1",
                                lit(schema)
                            ))
                            .await?;
                        Ok(rows
                            .iter()
                            .map(|row| SchemaNode::leaf(cell(row, 0), "function").detail(cell(row, 1)))
                            .collect())
                    }
                    TYPES => {
                        let rows = self
                            .catalog(format!(
                                "SELECT t.typname, CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'd' THEN 'domain' \
                                        WHEN 'r' THEN 'range' WHEN 'm' THEN 'multirange' ELSE 'composite' END \
                                 FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                                 LEFT JOIN pg_catalog.pg_class c ON c.oid = t.typrelid \
                                 WHERE n.nspname = {} AND t.typtype IN ('e', 'd', 'r', 'm', 'c') \
                                   AND (t.typtype <> 'c' OR c.relkind = 'c') ORDER BY 1",
                                lit(schema)
                            ))
                            .await?;
                        Ok(rows
                            .iter()
                            .map(|row| SchemaNode::leaf(cell(row, 0), "type").detail(cell(row, 1)))
                            .collect())
                    }
                    _ => Ok(vec![]),
                }
            }
            [schema, group, relation] if [TABLES, VIEWS, MATVIEWS].contains(&group.as_str()) => {
                let rows = self
                    .catalog(format!(
                        "SELECT a.attname, pg_catalog.format_type(a.atttypid, a.atttypmod), a.attnotnull::text, \
                                EXISTS (SELECT 1 FROM pg_catalog.pg_index i WHERE i.indrelid = a.attrelid \
                                        AND i.indisprimary AND a.attnum = ANY (i.indkey))::text \
                         FROM pg_catalog.pg_attribute a \
                         WHERE a.attrelid = {}::regclass AND a.attnum > 0 AND NOT a.attisdropped \
                         ORDER BY a.attnum",
                        lit(&format!("{}.{}", ident(schema), ident(relation)))
                    ))
                    .await?;
                Ok(rows
                    .iter()
                    .map(|row| {
                        let mut detail = cell(row, 1);
                        if cell(row, 3) == "true" {
                            detail.push_str(" · primary key");
                        } else if cell(row, 2) == "true" {
                            detail.push_str(" · not null");
                        }
                        SchemaNode::leaf(cell(row, 0), "column").detail(detail)
                    })
                    .collect())
            }
            _ => Ok(vec![]),
        }
    }

    async fn ddl(&self, path: &[String]) -> DbResult<String> {
        let [schema, group, name, ..] = path else {
            return Err(DbError::other("Select a table, view, function, sequence or type."));
        };
        let qualified = format!("{}.{}", ident(schema), ident(name));
        let regclass = format!("{}::regclass", lit(&qualified));
        let one = |rows: Vec<Vec<Option<String>>>| rows.first().map(|r| cell(r, 0)).unwrap_or_default();

        match group.as_str() {
            VIEWS | MATVIEWS => {
                let body = one(self
                    .catalog(format!("SELECT pg_catalog.pg_get_viewdef({regclass}, true)"))
                    .await?);
                let head = if group == VIEWS {
                    "CREATE OR REPLACE VIEW"
                } else {
                    "CREATE MATERIALIZED VIEW"
                };
                Ok(format!("{head} {qualified} AS\n{}", body.trim_end()))
            }
            FUNCTIONS => {
                // `name` is "f(argument types)", which casts to regprocedure.
                let (bare, args) = name.split_once('(').unwrap_or((name, ")"));
                let signature = format!("{}.{}({}", ident(schema), ident(bare), args);
                let definition = self
                    .catalog(format!(
                        "SELECT pg_catalog.pg_get_functiondef({}::regprocedure)",
                        lit(&signature)
                    ))
                    .await
                    .map_err(|mut e| {
                        if e.message.contains("aggregate") {
                            e.message = "PostgreSQL cannot print the definition of an aggregate.".into();
                        }
                        e
                    })?;
                Ok(format!("{};", one(definition).trim_end()))
            }
            SEQUENCES => {
                let rows = self
                    .catalog(format!(
                        "SELECT data_type::text, start_value::text, min_value::text, max_value::text, \
                                increment_by::text, cycle::text, cache_size::text \
                         FROM pg_catalog.pg_sequences WHERE schemaname = {} AND sequencename = {}",
                        lit(schema),
                        lit(name)
                    ))
                    .await?;
                let row = rows
                    .first()
                    .ok_or_else(|| DbError::other("That sequence no longer exists."))?;
                Ok(format!(
                    "CREATE SEQUENCE {qualified}\n    AS {}\n    START WITH {}\n    INCREMENT BY {}\n    MINVALUE {}\n    MAXVALUE {}\n    CACHE {}{};",
                    cell(row, 0),
                    cell(row, 1),
                    cell(row, 4),
                    cell(row, 2),
                    cell(row, 3),
                    cell(row, 6),
                    if cell(row, 5) == "true" { "\n    CYCLE" } else { "" }
                ))
            }
            TYPES => {
                let rows = self
                    .catalog(format!(
                        "SELECT t.typtype::text, \
                                (SELECT string_agg(quote_literal(e.enumlabel), ', ' ORDER BY e.enumsortorder) \
                                   FROM pg_catalog.pg_enum e WHERE e.enumtypid = t.oid), \
                                pg_catalog.format_type(t.typbasetype, t.typtypmod), \
                                (SELECT string_agg(quote_ident(a.attname) || ' ' || pg_catalog.format_type(a.atttypid, a.atttypmod), ', ' ORDER BY a.attnum) \
                                   FROM pg_catalog.pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped) \
                         FROM pg_catalog.pg_type t WHERE t.oid = {}::regtype",
                        lit(&qualified)
                    ))
                    .await?;
                let row = rows
                    .first()
                    .ok_or_else(|| DbError::other("That type no longer exists."))?;
                Ok(match cell(row, 0).as_str() {
                    "e" => format!("CREATE TYPE {qualified} AS ENUM ({});", cell(row, 1)),
                    "d" => format!("CREATE DOMAIN {qualified} AS {};", cell(row, 2)),
                    "c" => format!("CREATE TYPE {qualified} AS ({});", cell(row, 3)),
                    _ => format!("-- {qualified} is a range type; its definition is not reconstructed yet."),
                })
            }
            TABLES => {
                let columns = self
                    .catalog(format!(
                        "SELECT quote_ident(a.attname) || ' ' || pg_catalog.format_type(a.atttypid, a.atttypmod) \
                                || CASE WHEN a.attidentity = 'a' THEN ' GENERATED ALWAYS AS IDENTITY' \
                                        WHEN a.attidentity = 'd' THEN ' GENERATED BY DEFAULT AS IDENTITY' \
                                        WHEN a.attgenerated = 's' THEN ' GENERATED ALWAYS AS (' || pg_catalog.pg_get_expr(d.adbin, d.adrelid) || ') STORED' \
                                        WHEN d.adbin IS NOT NULL THEN ' DEFAULT ' || pg_catalog.pg_get_expr(d.adbin, d.adrelid) \
                                        ELSE '' END \
                                || CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END \
                         FROM pg_catalog.pg_attribute a \
                         LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                         WHERE a.attrelid = {regclass} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
                    ))
                    .await?;
                let constraints = self
                    .catalog(format!(
                        "SELECT 'CONSTRAINT ' || quote_ident(conname) || ' ' || pg_catalog.pg_get_constraintdef(oid) \
                         FROM pg_catalog.pg_constraint WHERE conrelid = {regclass} AND contype <> 'n' \
                         ORDER BY contype <> 'p', conname"
                    ))
                    .await?;
                let indexes = self
                    .catalog(format!(
                        "SELECT pg_catalog.pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i \
                         WHERE i.indrelid = {regclass} AND NOT EXISTS \
                               (SELECT 1 FROM pg_catalog.pg_constraint c WHERE c.conindid = i.indexrelid) \
                         ORDER BY 1"
                    ))
                    .await?;
                let partition = one(self
                    .catalog(format!(
                        "SELECT COALESCE(pg_catalog.pg_get_partkeydef({regclass}), '')"
                    ))
                    .await?);

                let body: Vec<String> = columns
                    .iter()
                    .chain(constraints.iter())
                    .map(|row| format!("    {}", cell(row, 0)))
                    .collect();
                let mut ddl = format!("CREATE TABLE {qualified} (\n{}\n)", body.join(",\n"));
                if !partition.is_empty() {
                    ddl.push_str(&format!(" PARTITION BY {partition}"));
                }
                ddl.push(';');
                for row in &indexes {
                    ddl.push_str(&format!("\n\n{};", cell(row, 0)));
                }
                Ok(ddl)
            }
            _ => Err(DbError::other("Select a table, view, function, sequence or type.")),
        }
    }

    async fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pgpass_matches_fields_and_wildcards() {
        let file = "# comment\n\
                    db.example.com:5432:app:alice:s3cret\n\
                    *:*:*:bob:bobpw\n\
                    localhost:5433:*:carol:with\\:colon\\\\slash\n";
        assert_eq!(
            pgpass_lookup(file, "db.example.com", 5432, "app", "alice").as_deref(),
            Some("s3cret")
        );
        assert_eq!(pgpass_lookup(file, "anything", 1, "x", "bob").as_deref(), Some("bobpw"));
        assert_eq!(
            pgpass_lookup(file, "localhost", 5433, "any", "carol").as_deref(),
            Some("with:colon\\slash")
        );
        assert_eq!(pgpass_lookup(file, "db.example.com", 5432, "other", "alice"), None);
        assert_eq!(pgpass_lookup("garbage line\n", "h", 1, "d", "u"), None);
    }

    #[test]
    fn first_matching_pgpass_line_wins() {
        let file = "*:*:*:u:first\nhost:1:d:u:second\n";
        assert_eq!(pgpass_lookup(file, "host", 1, "d", "u").as_deref(), Some("first"));
    }

    #[test]
    fn begin_modes_are_carried_over() {
        assert_eq!(transaction_modes("BEGIN"), None);
        assert_eq!(transaction_modes("begin transaction;"), None);
        assert_eq!(
            transaction_modes("BEGIN ISOLATION LEVEL SERIALIZABLE").as_deref(),
            Some("SET TRANSACTION ISOLATION LEVEL SERIALIZABLE")
        );
        assert_eq!(
            transaction_modes("start transaction read only").as_deref(),
            Some("SET TRANSACTION read only")
        );
    }

    #[test]
    fn row_estimates_are_compact() {
        assert_eq!(row_estimate("-1"), "");
        assert_eq!(row_estimate("42"), "~42 rows");
        assert_eq!(row_estimate("1234567"), "~1.2M rows");
    }
}

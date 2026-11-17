//! What the UI calls: Ask, Fix and Explain.
//!
//! The copilot only ever produces text. Nothing here can run a statement:
//! the SQL it returns is shown to the user, who decides what happens to it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlparser::parser::Parser;
use tokio::sync::Notify;

use super::schema_index::{self, SchemaIndex};
use super::{AiError, AiErrorKind, AiResult, ChatRequest, Endpoint, Message, Role, Usage};
use crate::db::{DbKind, ErrorKind};
use crate::manager::{now_ms, Manager};
use crate::settings::{AiProvider, Settings};
use crate::statements::{destructive_reason, dialect, split};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Task {
    Ask,
    Fix,
    Explain,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    /// Chosen by the UI so it can cancel.
    pub id: String,
    pub task: Task,
    #[serde(default)]
    pub connection_id: Option<String>,
    /// Ask: what the user wants.
    #[serde(default)]
    pub prompt: String,
    /// Ask: the editor's text for reference. Fix: the failed statement.
    /// Explain: the SQL to explain.
    #[serde(default)]
    pub sql: String,
    /// Fix: the database's error message.
    #[serde(default)]
    pub error: String,
    /// Fix: the error's category. Only syntax and schema errors are sent.
    #[serde(default)]
    pub error_kind: Option<ErrorKind>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AiEvent {
    /// The exact payload, before it is sent.
    #[serde(rename_all = "camelCase")]
    Context {
        provider: String,
        request: ChatRequest,
        tables_sent: usize,
        tables_total: usize,
    },
    Delta {
        text: String,
    },
    /// The SQL did not parse; a second attempt has started.
    Retry {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOutcome {
    /// The whole answer as the model wrote it.
    pub text: String,
    /// The SQL it contains, if any.
    pub sql: Option<String>,
    /// The answer without the SQL block.
    pub explanation: String,
    /// Why the SQL is destructive, when it is.
    pub destructive: Option<String>,
    /// Set when the SQL still does not parse after the retry.
    pub parse_error: Option<String>,
    pub usage: Usage,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub tables: usize,
    pub columns: usize,
    pub built_at: i64,
    pub fingerprint: String,
    /// False when a refresh found the schema unchanged.
    pub changed: bool,
    pub with_samples: bool,
}

#[derive(Default)]
pub struct Copilot {
    http: OnceLock<reqwest::Client>,
    running: Mutex<HashMap<String, Arc<Notify>>>,
    indexes: Mutex<HashMap<String, Arc<SchemaIndex>>>,
}

fn index_key(connection_id: &str) -> String {
    format!("schema-index/{connection_id}")
}

/// Resolves a provider's settings and its key into something callable.
pub fn endpoint(provider: &AiProvider, manager: &Manager, timeout_secs: u32) -> AiResult<Endpoint> {
    let base_url = if provider.base_url.is_empty() {
        provider.kind.default_base_url().to_string()
    } else {
        provider.base_url.trim_end_matches('/').to_string()
    };
    if base_url.is_empty() {
        return Err(AiError::new(
            AiErrorKind::NotConfigured,
            "This provider needs a base URL.",
        ));
    }
    if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
        return Err(AiError::new(
            AiErrorKind::NotConfigured,
            "The base URL must start with https:// or http://.",
        ));
    }
    if provider.kind == super::ProviderKind::Azure && provider.deployment.is_empty() {
        return Err(AiError::new(
            AiErrorKind::NotConfigured,
            "Azure OpenAI needs a deployment name.",
        ));
    }
    let key = manager
        .ai_key(&provider.id)
        .map_err(|e| AiError::other(format!("Could not read the API key from the keychain: {e}")))?
        .filter(|k| !k.is_empty());
    if key.is_none() && provider.kind.key_required() {
        return Err(AiError::new(
            AiErrorKind::NotConfigured,
            format!("No API key is saved for {}.", display_name(provider)),
        ));
    }
    Ok(Endpoint {
        kind: provider.kind,
        base_url,
        key,
        deployment: provider.deployment.clone(),
        timeout: Duration::from_secs(u64::from(timeout_secs)),
    })
}

fn display_name(provider: &AiProvider) -> String {
    if provider.name.is_empty() {
        format!("{:?}", provider.kind)
    } else {
        provider.name.clone()
    }
}

fn dialect_name(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Postgres => "PostgreSQL",
        DbKind::Clickhouse => "ClickHouse",
        DbKind::Sqlite => "SQLite",
    }
}

/// The first fenced code block and the text around it.
pub fn extract_sql(answer: &str) -> (Option<String>, String) {
    let Some(open) = answer.find("```") else {
        return (None, answer.trim().to_string());
    };
    let after = &answer[open + 3..];
    // Skip the language tag on the opening fence.
    let body_start = after.find('\n').map_or(after.len(), |i| i + 1);
    let body = &after[body_start..];
    let (sql, rest) = match body.find("```") {
        Some(close) => (&body[..close], &body[close + 3..]),
        None => (body, ""),
    };
    let explanation = format!("{} {}", answer[..open].trim(), rest.trim());
    let sql = sql.trim();
    (
        (!sql.is_empty()).then(|| sql.to_string()),
        explanation.trim().to_string(),
    )
}

/// `None` when the SQL parses in the connection's dialect.
pub fn parse_problem(kind: Option<DbKind>, sql: &str) -> Option<String> {
    let dialect = match kind {
        Some(kind) => dialect(kind),
        None => Box::new(sqlparser::dialect::GenericDialect {}),
    };
    Parser::parse_sql(dialect.as_ref(), sql)
        .err()
        .map(|e| e.to_string())
}

fn destructive(kind: Option<DbKind>, sql: &str) -> Option<String> {
    let kind = kind.unwrap_or(DbKind::Postgres);
    // Spans are UTF-16 offsets (what the editor uses).
    let units: Vec<u16> = sql.encode_utf16().collect();
    split(kind, sql).iter().find_map(|span| {
        let statement = String::from_utf16_lossy(
            &units[span.start.min(units.len())..span.end.min(units.len())],
        );
        destructive_reason(kind, &statement).map(str::to_string)
    })
}

const NOTATION: &str = "Schema notation: one table per line as table(column type flags). \
`pk` = primary key, `fk→t.c` = foreign key to t.c, `?` after the type = nullable, \
`{a|b}` = known values, `/*…*/` = column comment, and after `--` come row estimate, \
storage, indexes and the table comment.";

struct Context {
    kind: Option<DbKind>,
    system: String,
    tables_sent: usize,
    tables_total: usize,
}

impl Copilot {
    fn http(&self) -> AiResult<reqwest::Client> {
        if let Some(client) = self.http.get() {
            return Ok(client.clone());
        }
        let client = super::http_client()?;
        Ok(self.http.get_or_init(|| client).clone())
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Notify>>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn indexes(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<SchemaIndex>>> {
        self.indexes.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn cancel(&self, id: &str) {
        if let Some(cancel) = self.running().get(id) {
            // `notify_one` leaves a permit, so a cancel that lands between
            // two awaits is not lost.
            cancel.notify_one();
        }
    }

    /// The connection's schema index: from memory, else from disk, else built
    /// now if the connection is open. `refresh` rebuilds it from the catalog.
    pub async fn index(
        &self,
        manager: &Manager,
        connection_id: &str,
        refresh: bool,
    ) -> AiResult<Option<(Arc<SchemaIndex>, bool)>> {
        let cached = self.indexes().get(connection_id).cloned().or_else(|| {
            let stored = manager.session_load(&index_key(connection_id)).ok()??;
            let index: SchemaIndex = serde_json::from_str(&stored).ok()?;
            Some(Arc::new(index))
        });
        let config = manager
            .config(connection_id)
            .map_err(|e| AiError::other(e.message))?;
        // Turning sample values off must drop the ones already collected.
        let stale = cached
            .as_ref()
            .is_some_and(|index| index.with_samples != config.ai_sample_values);
        if let (Some(index), false, false) = (&cached, refresh, stale) {
            self.indexes()
                .insert(connection_id.to_string(), index.clone());
            return Ok(Some((index.clone(), false)));
        }
        if !manager.is_open(connection_id).await {
            return Ok(if stale {
                None
            } else {
                cached.map(|i| (i, false))
            });
        }

        let snapshot = manager
            .snapshot(connection_id)
            .await
            .map_err(|e| AiError::other(format!("Could not read the schema: {}", e.message)))?;
        // Details are best effort: a catalog the user may not read costs
        // comments and indexes, not the whole index.
        let details = manager
            .details(connection_id, config.ai_sample_values)
            .await
            .unwrap_or_default();
        let built = schema_index::build(&snapshot, &details, config.ai_sample_values, now_ms());
        let changed = cached
            .as_ref()
            .is_none_or(|old| old.fingerprint != built.fingerprint);
        let index = if changed {
            let index = Arc::new(built);
            if let Ok(json) = serde_json::to_string(index.as_ref()) {
                let _ = manager.session_save(&index_key(connection_id), &json);
            }
            index
        } else {
            cached.expect("unchanged implies a previous index")
        };
        self.indexes()
            .insert(connection_id.to_string(), index.clone());
        Ok(Some((index, changed)))
    }

    pub async fn index_info(
        &self,
        manager: &Manager,
        connection_id: &str,
        refresh: bool,
    ) -> AiResult<Option<IndexInfo>> {
        Ok(self
            .index(manager, connection_id, refresh)
            .await?
            .map(|(index, changed)| IndexInfo {
                tables: index.tables.len(),
                columns: index.tables.iter().map(|t| t.columns.len()).sum(),
                built_at: index.built_at,
                fingerprint: index.fingerprint.clone(),
                changed,
                with_samples: index.with_samples,
            }))
    }

    async fn context(
        &self,
        manager: &Manager,
        settings: &Settings,
        request: &AiRequest,
    ) -> AiResult<Context> {
        let mut kind = None;
        let mut schema = None;
        let mut version = String::new();
        if let Some(id) = request.connection_id.as_deref().filter(|id| !id.is_empty()) {
            let config = manager.config(id).map_err(|e| AiError::other(e.message))?;
            if !config.ai_enabled {
                return Err(AiError::new(
                    AiErrorKind::NotConfigured,
                    format!("AI is turned off for the connection \"{}\".", config.name),
                ));
            }
            kind = Some(config.kind);
            if let Some((index, _)) = self.index(manager, id, false).await? {
                let history: Vec<String> = manager
                    .history(Some(id), "", 200)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| entry.sql)
                    .collect();
                let question = format!("{} {} {}", request.prompt, request.sql, request.error);
                version = index.server_version.clone();
                schema = Some(index.select(
                    &question,
                    &history,
                    settings.ai.schema_token_budget as usize,
                ));
            }
        }

        let target = match kind {
            Some(kind) if version.is_empty() => dialect_name(kind).to_string(),
            Some(kind) => format!("{} {version}", dialect_name(kind)),
            None => "standard SQL (no database is connected)".to_string(),
        };
        let mut system = match request.task {
            Task::Ask => format!(
                "You are the SQL copilot inside Rowboat, a desktop SQL client. Write {target}. \
                 You never run anything: the user reads your SQL and decides whether to run it.\n\n\
                 Answer with exactly one ```sql fenced block holding the complete statement or statements, \
                 followed by one short sentence that says what it does. Use only tables and columns from the \
                 schema below. If the request cannot be answered from that schema, say what is missing in one \
                 sentence instead of guessing names. Do not add a LIMIT the user did not ask for: the client \
                 limits rows itself."
            ),
            Task::Fix => format!(
                "You are the SQL copilot inside Rowboat, a desktop SQL client. A statement written for \
                 {target} failed. You never run anything: the user reads your fix and decides whether to accept it.\n\n\
                 Answer with exactly one ```sql fenced block holding the corrected statement, followed by one \
                 short sentence that says what was wrong. Change only what the error requires and keep the \
                 user's formatting. Use only tables and columns from the schema below. If you cannot tell what \
                 was meant, say so in one sentence and give no SQL."
            ),
            Task::Explain => format!(
                "You are the SQL copilot inside Rowboat, a desktop SQL client. Explain the user's {target} \
                 in plain English for someone who knows their data but may not know every SQL feature: what \
                 it returns or changes, then anything surprising (join fan-out, NULL handling, a filter that \
                 defeats an index, a statement that changes data). Be brief: a few sentences or a short list. \
                 Do not rewrite the query unless it is wrong."
            ),
        };
        let (mut tables_sent, mut tables_total) = (0, 0);
        match schema {
            Some(selection) if selection.tables_total > 0 => {
                tables_sent = selection.tables_sent;
                tables_total = selection.tables_total;
                system.push_str(&format!("\n\n{NOTATION}\n\n<schema>\n{}\n</schema>", selection.text));
            }
            _ => system.push_str(
                "\n\nNo schema is available for this connection, so table and column names are unknown to you. \
                 Say so if the answer depends on them.",
            ),
        }
        Ok(Context {
            kind,
            system,
            tables_sent,
            tables_total,
        })
    }

    /// Runs one request, streaming progress to `emit`.
    pub async fn run(
        &self,
        manager: &Manager,
        settings: &Settings,
        request: AiRequest,
        emit: &(dyn Fn(AiEvent) + Send + Sync),
    ) -> AiResult<AiOutcome> {
        if request.task == Task::Fix
            && !matches!(
                request.error_kind,
                Some(ErrorKind::Syntax | ErrorKind::Schema)
            )
        {
            // Connection, permission and timeout errors say nothing a model
            // could fix, and may carry host names: they stay on this machine.
            return Err(AiError::other(
                "Only syntax and schema errors are sent to the AI.",
            ));
        }
        let provider = settings
            .ai
            .providers
            .iter()
            .find(|p| p.id == settings.ai.default_provider)
            .ok_or_else(|| {
                AiError::new(
                    AiErrorKind::NotConfigured,
                    "No AI provider is set up. Add one in Settings → AI.",
                )
            })?;
        let model = if request.task == Task::Fix && !provider.fix_model.is_empty() {
            provider.fix_model.clone()
        } else {
            provider.model.clone()
        };
        if model.is_empty() {
            return Err(AiError::new(
                AiErrorKind::NotConfigured,
                format!("No model is chosen for {}.", display_name(provider)),
            ));
        }
        let endpoint = endpoint(provider, manager, settings.ai.request_timeout_secs)?;
        let context = self.context(manager, settings, &request).await?;

        let fence = |sql: &str| format!("```sql\n{}\n```", sql.trim());
        let user = match request.task {
            Task::Ask if request.sql.trim().is_empty() => request.prompt.trim().to_string(),
            Task::Ask => format!(
                "{}\n\nThe editor currently holds this, for reference:\n{}",
                request.prompt.trim(),
                fence(&request.sql)
            ),
            Task::Fix => format!(
                "This statement failed:\n{}\n\nThe database said:\n{}",
                fence(&request.sql),
                request.error.trim()
            ),
            Task::Explain => fence(&request.sql),
        };
        let mut chat = ChatRequest {
            model: model.clone(),
            system: context.system,
            messages: vec![Message {
                role: Role::User,
                content: user,
            }],
        };

        let cancel = Arc::new(Notify::new());
        self.running().insert(request.id.clone(), cancel.clone());
        let http = self.http();
        let result = async {
            let http = http?;
            let mut usage = Usage::default();
            let mut attempt = 0;
            loop {
                emit(AiEvent::Context {
                    provider: display_name(provider),
                    request: chat.clone(),
                    tables_sent: context.tables_sent,
                    tables_total: context.tables_total,
                });
                let mut text = String::new();
                let mut on_delta = |delta: &str| {
                    text.push_str(delta);
                    emit(AiEvent::Delta {
                        text: delta.to_string(),
                    });
                };
                let used = super::stream_chat(&http, &endpoint, &chat, &cancel, &mut on_delta).await?;
                usage.input_tokens = add(usage.input_tokens, used.input_tokens);
                usage.output_tokens = add(usage.output_tokens, used.output_tokens);
                if text.trim().is_empty() {
                    return Err(AiError::other("The model returned an empty answer."));
                }

                let (sql, explanation) = match request.task {
                    Task::Explain => (None, text.trim().to_string()),
                    _ => extract_sql(&text),
                };
                let parse_error = sql.as_deref().and_then(|sql| parse_problem(context.kind, sql));
                if let (Some(problem), 0) = (&parse_error, attempt) {
                    attempt += 1;
                    emit(AiEvent::Retry {
                        reason: problem.clone(),
                    });
                    chat.messages.push(Message {
                        role: Role::Assistant,
                        content: text,
                    });
                    chat.messages.push(Message {
                        role: Role::User,
                        content: format!(
                            "That SQL does not parse: {problem}\nReturn the corrected SQL in the same format."
                        ),
                    });
                    continue;
                }
                return Ok(AiOutcome {
                    destructive: sql.as_deref().and_then(|sql| destructive(context.kind, sql)),
                    text,
                    sql,
                    explanation,
                    parse_error,
                    usage,
                    provider: display_name(provider),
                    model,
                });
            }
        }
        .await;
        self.running().remove(&request.id);
        result
    }
}

fn add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_is_taken_from_the_first_fence() {
        let (sql, why) = extract_sql("```sql\nSELECT 1;\n```\nReturns one.");
        assert_eq!(sql.as_deref(), Some("SELECT 1;"));
        assert_eq!(why, "Returns one.");

        let (sql, why) = extract_sql("Here you go:\n```\nSELECT 2\n```");
        assert_eq!(sql.as_deref(), Some("SELECT 2"));
        assert_eq!(why, "Here you go:");
    }

    #[test]
    fn an_answer_without_a_fence_is_all_explanation() {
        let (sql, why) = extract_sql("There is no table for refunds.");
        assert_eq!(sql, None);
        assert_eq!(why, "There is no table for refunds.");
    }

    #[test]
    fn an_unclosed_fence_still_yields_the_sql() {
        let (sql, _) = extract_sql("```sql\nSELECT 3");
        assert_eq!(sql.as_deref(), Some("SELECT 3"));
    }

    #[test]
    fn generated_sql_is_parsed_in_the_connection_dialect() {
        assert_eq!(
            parse_problem(Some(DbKind::Postgres), "SELECT a::int FROM t"),
            None
        );
        assert!(parse_problem(Some(DbKind::Postgres), "SELEC a FROM t").is_some());
        assert!(parse_problem(Some(DbKind::Sqlite), "SELECT FROM WHERE").is_some());
    }

    #[test]
    fn destructive_output_is_flagged() {
        assert!(destructive(Some(DbKind::Postgres), "SELECT 1; DELETE FROM orders").is_some());
        assert!(destructive(Some(DbKind::Postgres), "SELECT 1").is_none());
    }
}

//! The adapters and the copilot against a local fake provider.
//!
//! No real provider is reachable from the test environment, so these check
//! the wire shapes we send and that we read the documented stream shapes.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::copilot::{AiEvent, AiRequest, Copilot, Task};
use super::*;
use crate::connections::{AppDb, ConnectionConfig};
use crate::db::{DbKind, ErrorKind};
use crate::manager::{Manager, SecretInput};
use crate::secrets::Secrets;
use crate::settings::{AiProvider, Settings};

const KEY: &str = "sk-test-SECRET-123";

#[derive(Clone)]
struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    /// Keep the connection open after the body instead of ending the stream.
    hang: bool,
}

fn sse(events: &[&str]) -> Reply {
    Reply {
        status: 200,
        content_type: "text/event-stream",
        body: events.iter().map(|e| format!("data: {e}\n\n")).collect(),
        hang: false,
    }
}

fn json(status: u16, body: &str) -> Reply {
    Reply {
        status,
        content_type: "application/json",
        body: body.to_string(),
        hang: false,
    }
}

/// Serves `replies` in order (the last one repeats) and records each request
/// as "METHOD path\nheaders\n\nbody".
async fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        let mut n = 0;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let reply = replies[n.min(replies.len() - 1)].clone();
            n += 1;
            let log = log.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let request = loop {
                    let Ok(read) = socket.read(&mut buf).await else {
                        return;
                    };
                    raw.extend_from_slice(&buf[..read]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let length = text[..head_end]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        if raw.len() >= head_end + 4 + length {
                            break text;
                        }
                    }
                    if read == 0 {
                        return;
                    }
                };
                log.lock().unwrap().push(request);
                let head = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: {}\r\nconnection: close\r\n\r\n",
                    reply.status, reply.content_type
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(reply.body.as_bytes()).await;
                let _ = socket.flush().await;
                if reply.hang {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
        }
    });
    (base, seen)
}

fn endpoint(kind: ProviderKind, base: &str) -> Endpoint {
    Endpoint {
        kind,
        base_url: base.to_string(),
        key: Some(KEY.into()),
        deployment: "prod-gpt".into(),
        timeout: Duration::from_secs(10),
    }
}

fn chat() -> ChatRequest {
    ChatRequest {
        model: "m-1".into(),
        system: "be brief".into(),
        messages: vec![Message {
            role: Role::User,
            content: "hello".into(),
        }],
    }
}

async fn stream(endpoint: &Endpoint) -> (AiResult<Usage>, String) {
    let http = http_client().unwrap();
    let mut text = String::new();
    let mut on_delta = |d: &str| text.push_str(d);
    let result = stream_chat(&http, endpoint, &chat(), &Notify::new(), &mut on_delta).await;
    (result, text)
}

fn body_of(request: &str) -> serde_json::Value {
    serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

// ---------------------------------------------------------------- OpenAI ----

#[tokio::test]
async fn openai_compatible_stream_is_read_and_the_request_is_shaped_right() {
    let (base, seen) = serve(vec![sse(&[
        r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
        r#"{"choices":[{"delta":{"content":"SELECT"}}]}"#,
        r#"{"choices":[{"delta":{"content":" 1"},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":42,"completion_tokens":7}}"#,
        "[DONE]",
    ])])
    .await;
    let (usage, text) = stream(&endpoint(ProviderKind::Deepseek, &base)).await;
    assert_eq!(text, "SELECT 1");
    assert_eq!(
        usage.unwrap(),
        Usage {
            input_tokens: Some(42),
            output_tokens: Some(7)
        }
    );

    let request = seen.lock().unwrap()[0].clone();
    assert!(request.starts_with("POST /chat/completions "), "{request}");
    assert!(request
        .to_lowercase()
        .contains(&format!("authorization: bearer {}", KEY.to_lowercase())));
    let body = body_of(&request);
    assert_eq!(body["model"], "m-1");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "hello");
}

#[tokio::test]
async fn a_server_that_rejects_stream_options_gets_a_second_try_without_them() {
    let (base, seen) = serve(vec![
        json(
            400,
            r#"{"error":{"message":"unknown field stream_options"}}"#,
        ),
        sse(&[r#"{"choices":[{"delta":{"content":"ok"}}]}"#, "[DONE]"]),
    ])
    .await;
    let (usage, text) = stream(&endpoint(ProviderKind::Custom, &base)).await;
    assert_eq!(text, "ok");
    assert_eq!(usage.unwrap(), Usage::default(), "usage is simply unknown");
    let seen = seen.lock().unwrap();
    assert!(body_of(&seen[0]).get("stream_options").is_some());
    assert!(body_of(&seen[1]).get("stream_options").is_none());
}

#[tokio::test]
async fn a_rejected_key_is_an_auth_error_that_does_not_repeat_the_key() {
    let (base, _) = serve(vec![json(
        401,
        r#"{"error":{"message":"Incorrect API key provided"}}"#,
    )])
    .await;
    let (result, _) = stream(&endpoint(ProviderKind::Openai, &base)).await;
    let error = result.unwrap_err();
    assert_eq!(error.kind, AiErrorKind::Auth);
    assert!(error.message.contains("Incorrect API key provided"));
    assert!(!error.message.contains(KEY));
}

#[tokio::test]
async fn azure_addresses_the_deployment_and_uses_its_own_key_header() {
    let (base, seen) = serve(vec![sse(&[
        r#"{"choices":[{"delta":{"content":"x"}}]}"#,
        "[DONE]",
    ])])
    .await;
    let azure = endpoint(ProviderKind::Azure, &base);
    stream(&azure).await.0.unwrap();
    let request = seen.lock().unwrap()[0].clone();
    assert!(
        request.starts_with("POST /openai/deployments/prod-gpt/chat/completions?api-version="),
        "{request}"
    );
    assert!(request
        .to_lowercase()
        .contains("api-key: sk-test-secret-123"));
    assert!(!request.to_lowercase().contains("authorization:"));
    assert_eq!(
        list_models(&http_client().unwrap(), &azure).await.unwrap(),
        ["prod-gpt"]
    );
}

#[tokio::test]
async fn a_local_server_needs_no_key_and_lists_its_models() {
    let (base, seen) = serve(vec![json(
        200,
        r#"{"data":[{"id":"qwen3"},{"id":"llama4"}]}"#,
    )])
    .await;
    let mut local = endpoint(ProviderKind::Local, &base);
    local.key = None;
    let models = list_models(&http_client().unwrap(), &local).await.unwrap();
    assert_eq!(models, ["llama4", "qwen3"]);
    let request = seen.lock().unwrap()[0].clone();
    assert!(request.starts_with("GET /models "));
    assert!(!request.to_lowercase().contains("authorization"));
}

// ------------------------------------------------------------- Anthropic ----

#[tokio::test]
async fn anthropic_stream_skips_thinking_and_reports_usage() {
    let (base, seen) = serve(vec![sse(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":120}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#,
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"SELECT "}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"1"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":31}}"#,
        r#"{"type":"message_stop"}"#,
    ])])
    .await;
    let (usage, text) = stream(&endpoint(ProviderKind::Anthropic, &base)).await;
    assert_eq!(text, "SELECT 1");
    assert_eq!(
        usage.unwrap(),
        Usage {
            input_tokens: Some(120),
            output_tokens: Some(31)
        }
    );

    let request = seen.lock().unwrap()[0].clone();
    let lower = request.to_lowercase();
    assert!(request.starts_with("POST /v1/messages "));
    assert!(lower.contains("x-api-key: sk-test-secret-123"));
    assert!(lower.contains("anthropic-version: 2026-10-21"));
    let body = body_of(&request);
    assert_eq!(body["system"], "be brief");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["stream"], true);
    assert!(body["max_tokens"].as_u64().unwrap() >= 4096);
    for rejected in ["temperature", "top_p", "top_k", "thinking"] {
        assert!(body.get(rejected).is_none(), "{rejected} must not be sent");
    }
}

#[tokio::test]
async fn an_anthropic_refusal_is_reported_as_one() {
    let (base, _) = serve(vec![sse(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":5}}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"refusal"},"usage":{"output_tokens":0}}"#,
        r#"{"type":"message_stop"}"#,
    ])])
    .await;
    let (result, _) = stream(&endpoint(ProviderKind::Anthropic, &base)).await;
    assert_eq!(result.unwrap_err().kind, AiErrorKind::Refused);
}

#[tokio::test]
async fn an_error_event_in_the_middle_of_an_anthropic_stream_fails_the_request() {
    let (base, _) = serve(vec![sse(&[
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"SEL"}}"#,
        r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
    ])])
    .await;
    let (result, text) = stream(&endpoint(ProviderKind::Anthropic, &base)).await;
    assert_eq!(text, "SEL");
    let error = result.unwrap_err();
    assert_eq!(error.kind, AiErrorKind::RateLimit);
    assert!(error.message.contains("Overloaded"));
}

#[tokio::test]
async fn anthropic_models_are_listed_across_pages() {
    let (base, seen) = serve(vec![
        json(
            200,
            r#"{"data":[{"id":"claude-b"}],"has_more":true,"last_id":"claude-b"}"#,
        ),
        json(
            200,
            r#"{"data":[{"id":"claude-a"}],"has_more":false,"last_id":"claude-a"}"#,
        ),
    ])
    .await;
    let models = list_models(
        &http_client().unwrap(),
        &endpoint(ProviderKind::Anthropic, &base),
    )
    .await
    .unwrap();
    assert_eq!(models, ["claude-a", "claude-b"]);
    assert!(seen.lock().unwrap()[1].contains("after_id=claude-b"));
}

// ---------------------------------------------------------------- Gemini ----

#[tokio::test]
async fn gemini_stream_is_read_and_the_key_stays_out_of_the_url() {
    let (base, seen) = serve(vec![sse(&[
        r#"{"candidates":[{"content":{"parts":[{"text":"thinking…","thought":true},{"text":"SELECT"}]}}]}"#,
        r#"{"candidates":[{"content":{"parts":[{"text":" 1"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":3,"thoughtsTokenCount":4}}"#,
    ])])
    .await;
    let (usage, text) = stream(&endpoint(ProviderKind::Gemini, &base)).await;
    assert_eq!(text, "SELECT 1");
    assert_eq!(
        usage.unwrap(),
        Usage {
            input_tokens: Some(9),
            output_tokens: Some(7)
        }
    );
    let request = seen.lock().unwrap()[0].clone();
    let first_line = request.lines().next().unwrap();
    assert_eq!(
        first_line,
        "POST /v1beta/models/m-1:streamGenerateContent?alt=sse HTTP/1.1"
    );
    assert!(request
        .to_lowercase()
        .contains("x-goog-api-key: sk-test-secret-123"));
    let body = body_of(&request);
    assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be brief");
    assert_eq!(body["contents"][0]["role"], "user");
}

#[tokio::test]
async fn gemini_lists_only_models_that_generate_text() {
    let (base, _) = serve(vec![json(
        200,
        r#"{"models":[
            {"name":"models/gemini-x","supportedGenerationMethods":["generateContent"]},
            {"name":"models/embed-1","supportedGenerationMethods":["embedContent"]}]}"#,
    )])
    .await;
    let models = list_models(
        &http_client().unwrap(),
        &endpoint(ProviderKind::Gemini, &base),
    )
    .await
    .unwrap();
    assert_eq!(models, ["gemini-x"]);
}

// ---------------------------------------------------------------- cancel ----

#[tokio::test]
async fn cancel_ends_a_stream_that_is_still_open() {
    let mut reply = sse(&[r#"{"choices":[{"delta":{"content":"part"}}]}"#]);
    reply.hang = true;
    let (base, _) = serve(vec![reply]).await;
    let endpoint = endpoint(ProviderKind::Openai, &base);
    let http = http_client().unwrap();
    let cancel = Arc::new(Notify::new());
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.notify_one();
    });
    let started = std::time::Instant::now();
    let mut text = String::new();
    let mut on_delta = |d: &str| text.push_str(d);
    let result = stream_chat(&http, &endpoint, &chat(), &cancel, &mut on_delta).await;
    assert_eq!(result.unwrap_err().kind, AiErrorKind::Cancelled);
    assert_eq!(text, "part", "what arrived before the cancel is kept");
    assert!(started.elapsed() < Duration::from_secs(3));
}

// --------------------------------------------------------------- copilot ----

struct Fixture {
    manager: Manager,
    settings: Settings,
    connection: String,
    _dir: tempfile::TempDir,
}

async fn fixture(base: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let manager = Manager::new(AppDb::in_memory().unwrap(), Secrets::in_memory());
    let mut settings = Settings::default();
    settings.ai.providers.push(AiProvider {
        id: "p1".into(),
        kind: ProviderKind::Custom,
        name: "Fake".into(),
        base_url: base.into(),
        model: "big".into(),
        fix_model: "small".into(),
        ..Default::default()
    });
    settings.ai.default_provider = "p1".into();
    let view = manager
        .save(
            ConnectionConfig {
                name: "shop".into(),
                kind: DbKind::Sqlite,
                path: dir.path().join("shop.db").display().to_string(),
                create: true,
                ..Default::default()
            },
            SecretInput::default(),
        )
        .await
        .unwrap();
    let id = view.config.id.clone();
    manager.open(&id, &settings, None).await.unwrap();
    for sql in [
        "CREATE TABLE customers (id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers (id), total REAL)",
        "INSERT INTO customers VALUES (1, 'private@example.com')",
    ] {
        manager.run(&id, "setup", sql, 10, true, &settings).await.unwrap();
    }
    Fixture {
        manager,
        settings,
        connection: id,
        _dir: dir,
    }
}

fn ask(f: &Fixture, prompt: &str) -> AiRequest {
    AiRequest {
        id: "r1".into(),
        task: Task::Ask,
        connection_id: Some(f.connection.clone()),
        prompt: prompt.into(),
        sql: String::new(),
        error: String::new(),
        error_kind: None,
    }
}

fn answer(text: &str) -> Reply {
    let event = serde_json::json!({ "choices": [{ "delta": { "content": text } }] }).to_string();
    sse(&[&event, "[DONE]"])
}

#[tokio::test]
async fn ask_sends_the_schema_but_no_row_data_and_returns_parsed_sql() {
    let (base, seen) = serve(vec![answer(
        "```sql\nSELECT count(*) FROM orders;\n```\nCounts the orders.",
    )])
    .await;
    let f = fixture(&base).await;
    let events = Mutex::new(Vec::new());
    let outcome = Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "how many orders?"), &|e| {
            events.lock().unwrap().push(e)
        })
        .await
        .unwrap();
    assert_eq!(outcome.sql.as_deref(), Some("SELECT count(*) FROM orders;"));
    assert_eq!(outcome.explanation, "Counts the orders.");
    assert_eq!(outcome.destructive, None);
    assert_eq!(outcome.parse_error, None);
    assert_eq!(outcome.model, "big");

    let request = seen.lock().unwrap()[0].clone();
    let system = body_of(&request)["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(system.contains("SQLite"), "the dialect is named");
    assert!(
        system.contains("orders(id int pk, customer_id int? fk→customers.id, total REAL?)"),
        "{system}"
    );
    assert!(
        !request.contains("private@example.com"),
        "row data never leaves"
    );

    // "View context" gets exactly what was sent.
    let events = events.lock().unwrap();
    let AiEvent::Context {
        request: shown,
        tables_sent,
        tables_total,
        ..
    } = &events[0]
    else {
        panic!("the first event is the context");
    };
    assert_eq!(shown.system, system);
    assert_eq!((*tables_sent, *tables_total), (2, 2));
    assert!(events.iter().any(|e| matches!(e, AiEvent::Delta { .. })));
}

#[tokio::test]
async fn sql_that_does_not_parse_gets_exactly_one_retry_with_the_parser_error() {
    let (base, seen) = serve(vec![
        answer("```sql\nSELEC * FORM orders\n```"),
        answer("```sql\nSELECT * FROM orders\n```\nFixed the typos."),
    ])
    .await;
    let f = fixture(&base).await;
    let events = Mutex::new(Vec::new());
    let outcome = Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "all orders"), &|e| {
            events.lock().unwrap().push(e)
        })
        .await
        .unwrap();
    assert_eq!(outcome.sql.as_deref(), Some("SELECT * FROM orders"));
    assert_eq!(outcome.parse_error, None);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, AiEvent::Retry { .. })));

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let second = body_of(&seen[1]);
    let last = second["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert!(last["content"]
        .as_str()
        .unwrap()
        .starts_with("That SQL does not parse:"));
}

#[tokio::test]
async fn sql_that_still_does_not_parse_is_shown_with_the_problem_not_retried_forever() {
    let (base, seen) = serve(vec![answer("```sql\nSELEC 1\n```")]).await;
    let f = fixture(&base).await;
    let outcome = Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "one"), &|_| {})
        .await
        .unwrap();
    assert_eq!(outcome.sql.as_deref(), Some("SELEC 1"));
    assert!(outcome.parse_error.is_some());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn destructive_output_carries_a_warning() {
    let (base, _) = serve(vec![answer(
        "```sql\nDELETE FROM orders\n```\nRemoves every order.",
    )])
    .await;
    let f = fixture(&base).await;
    let outcome = Copilot::default()
        .run(
            &f.manager,
            &f.settings,
            ask(&f, "empty the orders table"),
            &|_| {},
        )
        .await
        .unwrap();
    assert!(outcome.destructive.is_some());
}

#[tokio::test]
async fn fix_uses_the_fix_model_and_only_goes_out_for_syntax_and_schema_errors() {
    let (base, seen) = serve(vec![answer(
        "```sql\nSELECT total FROM orders\n```\nThe column is called total.",
    )])
    .await;
    let f = fixture(&base).await;
    let mut request = ask(&f, "");
    request.task = Task::Fix;
    request.sql = "SELECT totl FROM orders".into();
    request.error = "no such column: totl".into();

    request.error_kind = Some(ErrorKind::Connection);
    let refused = Copilot::default()
        .run(&f.manager, &f.settings, request.clone(), &|_| {})
        .await;
    assert!(refused.is_err());
    assert!(seen.lock().unwrap().is_empty(), "nothing was sent");

    request.error_kind = Some(ErrorKind::Schema);
    let outcome = Copilot::default()
        .run(&f.manager, &f.settings, request, &|_| {})
        .await
        .unwrap();
    assert_eq!(outcome.sql.as_deref(), Some("SELECT total FROM orders"));
    let body = body_of(&seen.lock().unwrap()[0]);
    assert_eq!(body["model"], "small");
    let user = body["messages"][1]["content"].as_str().unwrap();
    assert!(user.contains("SELECT totl FROM orders") && user.contains("no such column: totl"));
}

#[tokio::test]
async fn a_connection_with_ai_turned_off_sends_nothing() {
    let (base, seen) = serve(vec![answer("x")]).await;
    let f = fixture(&base).await;
    let mut config = f.manager.config(&f.connection).unwrap();
    config.ai_enabled = false;
    f.manager
        .save(config, SecretInput::default())
        .await
        .unwrap();
    let error = Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "anything"), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(error.kind, AiErrorKind::NotConfigured);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_provider_that_needs_a_key_and_has_none_is_not_called() {
    let (base, seen) = serve(vec![answer("x")]).await;
    let mut f = fixture(&base).await;
    f.settings.ai.providers[0].kind = ProviderKind::Openai;
    let error = Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "anything"), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(error.kind, AiErrorKind::NotConfigured);
    assert!(seen.lock().unwrap().is_empty());

    f.manager.set_ai_key("p1", KEY).unwrap();
    Copilot::default()
        .run(&f.manager, &f.settings, ask(&f, "anything"), &|_| {})
        .await
        .unwrap();
    assert!(
        seen.lock().unwrap()[0].contains(KEY),
        "the saved key is used"
    );
}

#[tokio::test]
async fn the_index_is_stored_and_a_refresh_reports_whether_the_schema_changed() {
    let (base, _) = serve(vec![answer("x")]).await;
    let f = fixture(&base).await;
    let copilot = Copilot::default();
    let first = copilot
        .index_info(&f.manager, &f.connection, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((first.tables, first.columns), (2, 5));
    assert!(first.changed);

    let same = copilot
        .index_info(&f.manager, &f.connection, true)
        .await
        .unwrap()
        .unwrap();
    assert!(!same.changed);
    assert_eq!(
        same.built_at, first.built_at,
        "an unchanged schema is not rewritten"
    );

    f.manager
        .run(
            &f.connection,
            "ddl",
            "ALTER TABLE orders ADD COLUMN note TEXT",
            10,
            true,
            &f.settings,
        )
        .await
        .unwrap();
    let changed = copilot
        .index_info(&f.manager, &f.connection, true)
        .await
        .unwrap()
        .unwrap();
    assert!(changed.changed);
    assert_eq!(changed.columns, 6);

    // A new process (no in-memory cache) finds it on disk even when closed.
    f.manager.close(&f.connection).await;
    let stored = Copilot::default()
        .index_info(&f.manager, &f.connection, false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.fingerprint, changed.fingerprint);
}

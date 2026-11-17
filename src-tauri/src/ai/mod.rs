//! The AI copilot's network side: three adapters cover every provider.
//!
//! Keys are read from the keychain here and go straight into a request
//! header. They are never part of a URL, an error message or anything that
//! crosses back to the webview.

pub mod anthropic;
pub mod copilot;
pub mod gemini;
pub mod openai_compat;
pub mod schema_index;
pub mod sse;

#[cfg(test)]
mod tests;

use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use sse::SseParser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Openai,
    Anthropic,
    Gemini,
    Deepseek,
    Mistral,
    Xai,
    Openrouter,
    Azure,
    /// Ollama, LM Studio or anything else on this machine.
    Local,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adapter {
    OpenAiCompat,
    Anthropic,
    Gemini,
}

impl ProviderKind {
    pub fn adapter(self) -> Adapter {
        match self {
            Self::Anthropic => Adapter::Anthropic,
            Self::Gemini => Adapter::Gemini,
            _ => Adapter::OpenAiCompat,
        }
    }

    /// Used when the provider's base URL is left empty.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Self::Openai => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Gemini => "https://generativelanguage.googleapis.com",
            Self::Deepseek => "https://api.deepseek.com/v1",
            Self::Mistral => "https://api.mistral.ai/v1",
            Self::Xai => "https://api.x.ai/v1",
            Self::Openrouter => "https://openrouter.ai/api/v1",
            Self::Local => "http://localhost:11434/v1",
            // Azure and custom endpoints have no meaningful default.
            Self::Azure | Self::Custom => "",
        }
    }

    pub fn key_required(self) -> bool {
        !matches!(self, Self::Local | Self::Custom)
    }
}

/// Where and how to reach one provider, resolved from settings + keychain.
#[derive(Clone)]
pub struct Endpoint {
    pub kind: ProviderKind,
    /// No trailing slash.
    pub base_url: String,
    pub key: Option<String>,
    /// Azure OpenAI deployment name.
    pub deployment: String,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// The exact payload of one request; also what "View context" shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiErrorKind {
    /// No provider, no model or no key configured.
    NotConfigured,
    Auth,
    RateLimit,
    Network,
    Timeout,
    Cancelled,
    /// The provider declined to answer.
    Refused,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AiError {
    pub kind: AiErrorKind,
    pub message: String,
}

impl AiError {
    pub fn new(kind: AiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn other(message: impl Into<String>) -> Self {
        Self::new(AiErrorKind::Other, message)
    }
    pub fn cancelled() -> Self {
        Self::new(AiErrorKind::Cancelled, "Cancelled.")
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type AiResult<T> = std::result::Result<T, AiError>;

pub fn http_client() -> AiResult<reqwest::Client> {
    crate::tls::ensure_provider();
    reqwest::Client::builder()
        .user_agent(concat!("rowboat/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AiError::other(format!("Could not set up the HTTP client: {e}")))
}

pub(crate) fn transport_error(e: reqwest::Error) -> AiError {
    // The URL is dropped: it adds nothing and must never carry a credential.
    let e = e.without_url();
    if e.is_timeout() {
        AiError::new(
            AiErrorKind::Timeout,
            "The AI provider did not answer in time.",
        )
    } else if e.is_connect() {
        AiError::new(
            AiErrorKind::Network,
            format!("Could not reach the AI provider: {e}"),
        )
    } else {
        AiError::new(AiErrorKind::Network, format!("The AI request failed: {e}"))
    }
}

/// Turns a non-2xx response into an error, using the provider's own message
/// (`error.message` in all three API families) when there is one.
pub(crate) async fn status_error(response: reqwest::Response) -> AiError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            let error = v.get("error")?;
            error
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| error.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let (kind, lead) = match status.as_u16() {
        401 | 403 => (AiErrorKind::Auth, "The provider rejected the API key"),
        404 => (
            AiErrorKind::Other,
            "The provider does not know this model or endpoint",
        ),
        429 => (
            AiErrorKind::RateLimit,
            "The provider is rate limiting or out of quota",
        ),
        _ => (AiErrorKind::Other, "The provider returned an error"),
    };
    let detail = detail.trim();
    AiError::new(
        kind,
        if detail.is_empty() {
            format!("{lead} (HTTP {}).", status.as_u16())
        } else {
            format!("{lead} (HTTP {}): {detail}", status.as_u16())
        },
    )
}

/// Reads an SSE response, handing each `data:` payload to `on_event`, until
/// the stream ends, `on_event` returns `Ok(false)`, or `cancel` fires.
pub(crate) async fn read_sse(
    response: reqwest::Response,
    cancel: &Notify,
    idle: Duration,
    mut on_event: impl FnMut(&str) -> AiResult<bool>,
) -> AiResult<()> {
    let mut stream = response.bytes_stream();
    let mut parser = SseParser::default();
    loop {
        // The timeout is on silence, not on the whole answer: a long answer
        // that keeps arriving is fine, a stalled one is not.
        let chunk = tokio::select! {
            _ = cancel.notified() => return Err(AiError::cancelled()),
            chunk = tokio::time::timeout(idle, stream.next()) => chunk.map_err(|_| timed_out())?,
        };
        match chunk {
            Some(Ok(bytes)) => {
                for event in parser.feed(&bytes) {
                    if !on_event(&event)? {
                        return Ok(());
                    }
                }
            }
            Some(Err(e)) => return Err(transport_error(e)),
            None => {
                if let Some(event) = parser.finish() {
                    on_event(&event)?;
                }
                return Ok(());
            }
        }
    }
}

fn timed_out() -> AiError {
    AiError::new(
        AiErrorKind::Timeout,
        "The AI provider did not answer in time.",
    )
}

/// Sends a request, racing it against cancellation and the time allowed for
/// the provider to start answering.
pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    cancel: &Notify,
    wait: Duration,
) -> AiResult<reqwest::Response> {
    tokio::select! {
        _ = cancel.notified() => Err(AiError::cancelled()),
        response = tokio::time::timeout(wait, request.send()) => {
            response.map_err(|_| timed_out())?.map_err(transport_error)
        }
    }
}

/// The models a provider offers, from its models endpoint.
pub async fn list_models(http: &reqwest::Client, endpoint: &Endpoint) -> AiResult<Vec<String>> {
    let mut models = match endpoint.kind.adapter() {
        Adapter::OpenAiCompat => openai_compat::list_models(http, endpoint).await?,
        Adapter::Anthropic => anthropic::list_models(http, endpoint).await?,
        Adapter::Gemini => gemini::list_models(http, endpoint).await?,
    };
    models.sort();
    models.dedup();
    Ok(models)
}

/// Streams one completion. `on_delta` receives each piece of answer text.
pub async fn stream_chat(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    request: &ChatRequest,
    cancel: &Notify,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AiResult<Usage> {
    match endpoint.kind.adapter() {
        Adapter::OpenAiCompat => {
            openai_compat::stream_chat(http, endpoint, request, cancel, on_delta).await
        }
        Adapter::Anthropic => {
            anthropic::stream_chat(http, endpoint, request, cancel, on_delta).await
        }
        Adapter::Gemini => gemini::stream_chat(http, endpoint, request, cancel, on_delta).await,
    }
}

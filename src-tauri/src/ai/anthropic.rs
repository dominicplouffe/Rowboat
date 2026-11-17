//! Anthropic's native Messages API, over raw HTTP (there is no official Rust SDK).

use serde_json::{json, Value};
use tokio::sync::Notify;

use super::{
    read_sse, send, status_error, AiError, AiErrorKind, AiResult, ChatRequest, Endpoint, Usage,
};

const API_VERSION: &str = "2026-10-21";
/// Room for the model's thinking as well as the answer; SQL answers are short,
/// and a low cap would cut a thinking model off before it writes anything.
const MAX_TOKENS: u32 = 16_000;

fn request(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .request(method, format!("{}{path}", endpoint.base_url))
        .header("anthropic-version", API_VERSION)
        .timeout(endpoint.timeout);
    if let Some(key) = &endpoint.key {
        builder = builder.header("x-api-key", key);
    }
    builder
}

pub async fn list_models(http: &reqwest::Client, endpoint: &Endpoint) -> AiResult<Vec<String>> {
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    // Paginated; a handful of pages at most.
    for _ in 0..10 {
        let mut builder =
            request(http, endpoint, reqwest::Method::GET, "/v1/models").query(&[("limit", "1000")]);
        if let Some(id) = &after {
            builder = builder.query(&[("after_id", id)]);
        }
        let response = builder.send().await.map_err(super::transport_error)?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let body: Value = response.json().await.map_err(super::transport_error)?;
        for model in body["data"].as_array().into_iter().flatten() {
            if let Some(id) = model["id"].as_str() {
                models.push(id.to_string());
            }
        }
        match (body["has_more"].as_bool(), body["last_id"].as_str()) {
            (Some(true), Some(last)) => after = Some(last.to_string()),
            _ => break,
        }
    }
    Ok(models)
}

pub fn body(request: &ChatRequest) -> Value {
    // No sampling parameters and no `thinking` field: current models reject
    // the former and choose their own thinking depth when the latter is absent.
    json!({
        "model": request.model,
        "max_tokens": MAX_TOKENS,
        "stream": true,
        "system": request.system,
        "messages": request.messages,
    })
}

pub async fn stream_chat(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    chat: &ChatRequest,
    cancel: &Notify,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AiResult<Usage> {
    let builder = request(http, endpoint, reqwest::Method::POST, "/v1/messages").json(&body(chat));
    let response = send(builder, cancel).await?;
    if !response.status().is_success() {
        return Err(status_error(response).await);
    }
    let mut usage = Usage::default();
    let mut wrote = false;
    read_sse(response, cancel, |data| {
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Ok(true);
        };
        match event["type"].as_str() {
            Some("message_start") => {
                usage.input_tokens = event["message"]["usage"]["input_tokens"].as_u64();
            }
            Some("content_block_delta") => {
                // Thinking and signature deltas are not part of the answer.
                if event["delta"]["type"] == "text_delta" {
                    if let Some(text) = event["delta"]["text"].as_str() {
                        wrote |= !text.is_empty();
                        on_delta(text);
                    }
                }
            }
            Some("message_delta") => {
                if let Some(tokens) = event["usage"]["output_tokens"].as_u64() {
                    usage.output_tokens = Some(tokens);
                }
                match event["delta"]["stop_reason"].as_str() {
                    Some("refusal") => {
                        return Err(AiError::new(
                            AiErrorKind::Refused,
                            "The model declined to answer this request.",
                        ));
                    }
                    Some("max_tokens") if !wrote => {
                        return Err(AiError::other(
                            "The model ran out of output tokens before it wrote an answer.",
                        ));
                    }
                    _ => {}
                }
            }
            Some("error") => {
                let message = event["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error");
                let kind = match event["error"]["type"].as_str() {
                    Some("overloaded_error") | Some("rate_limit_error") => AiErrorKind::RateLimit,
                    _ => AiErrorKind::Other,
                };
                return Err(AiError::new(
                    kind,
                    format!("The provider reported an error: {message}"),
                ));
            }
            Some("message_stop") => return Ok(false),
            _ => {}
        }
        Ok(true)
    })
    .await?;
    Ok(usage)
}

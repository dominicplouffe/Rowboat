//! The OpenAI-compatible chat completions API: OpenAI, DeepSeek, Mistral,
//! xAI, OpenRouter, Azure OpenAI, Ollama, LM Studio and custom endpoints.

use serde_json::{json, Value};
use tokio::sync::Notify;

use super::{
    read_sse, send, status_error, AiError, AiErrorKind, AiResult, ChatRequest, Endpoint,
    ProviderKind, Role, Usage,
};

/// The Azure OpenAI data-plane API version this adapter speaks.
const AZURE_API_VERSION: &str = "2024-10-21";

fn authorize(endpoint: &Endpoint, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match (&endpoint.key, endpoint.kind) {
        (Some(key), ProviderKind::Azure) => builder.header("api-key", key),
        (Some(key), _) => builder.bearer_auth(key),
        (None, _) => builder,
    }
}

pub async fn list_models(http: &reqwest::Client, endpoint: &Endpoint) -> AiResult<Vec<String>> {
    if endpoint.kind == ProviderKind::Azure {
        // Azure addresses a deployment, not a model: that is the only choice.
        return Ok(if endpoint.deployment.is_empty() {
            Vec::new()
        } else {
            vec![endpoint.deployment.clone()]
        });
    }
    let builder = authorize(endpoint, http.get(format!("{}/models", endpoint.base_url)))
        .timeout(endpoint.timeout);
    let response = builder.send().await.map_err(super::transport_error)?;
    if !response.status().is_success() {
        return Err(status_error(response).await);
    }
    let body: Value = response.json().await.map_err(super::transport_error)?;
    // `{"data": [...]}` everywhere; a bare array on a few local servers.
    let list = body["data"].as_array().or_else(|| body.as_array());
    Ok(list
        .into_iter()
        .flatten()
        .filter_map(|model| model["id"].as_str().map(str::to_string))
        .collect())
}

pub fn body(request: &ChatRequest, with_usage: bool) -> Value {
    let mut messages = vec![json!({ "role": "system", "content": request.system })];
    messages.extend(request.messages.iter().map(|m| {
        json!({
            "role": if m.role == Role::User { "user" } else { "assistant" },
            "content": m.content,
        })
    }));
    let mut body = json!({ "model": request.model, "messages": messages, "stream": true });
    if with_usage {
        body["stream_options"] = json!({ "include_usage": true });
    }
    body
}

fn chat_url(endpoint: &Endpoint) -> String {
    if endpoint.kind == ProviderKind::Azure {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={AZURE_API_VERSION}",
            endpoint.base_url, endpoint.deployment
        )
    } else {
        format!("{}/chat/completions", endpoint.base_url)
    }
}

pub async fn stream_chat(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    chat: &ChatRequest,
    cancel: &Notify,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AiResult<Usage> {
    let url = chat_url(endpoint);
    let post =
        |with_usage: bool| authorize(endpoint, http.post(&url).json(&body(chat, with_usage)));
    let mut response = send(post(true), cancel, endpoint.timeout).await?;
    if matches!(response.status().as_u16(), 400 | 422) {
        // Some compatible servers reject `stream_options`; usage is optional.
        let retry = send(post(false), cancel, endpoint.timeout).await?;
        if retry.status().is_success() {
            response = retry;
        }
    }
    if !response.status().is_success() {
        return Err(status_error(response).await);
    }

    let mut usage = Usage::default();
    read_sse(response, cancel, endpoint.timeout, |data| {
        if data.trim() == "[DONE]" {
            return Ok(false);
        }
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Ok(true);
        };
        if let Some(error) = event.get("error").filter(|e| !e.is_null()) {
            let message = error["message"]
                .as_str()
                .or(error.as_str())
                .unwrap_or("unknown error");
            return Err(AiError::other(format!(
                "The provider reported an error: {message}"
            )));
        }
        let choice = &event["choices"][0];
        if let Some(text) = choice["delta"]["content"].as_str() {
            if !text.is_empty() {
                on_delta(text);
            }
        }
        if choice["finish_reason"] == "content_filter" {
            return Err(AiError::new(
                AiErrorKind::Refused,
                "The provider's content filter blocked the answer.",
            ));
        }
        if let Some(tokens) = event["usage"]["prompt_tokens"].as_u64() {
            usage.input_tokens = Some(tokens);
        }
        if let Some(tokens) = event["usage"]["completion_tokens"].as_u64() {
            usage.output_tokens = Some(tokens);
        }
        Ok(true)
    })
    .await?;
    Ok(usage)
}

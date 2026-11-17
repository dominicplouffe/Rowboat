//! Google Gemini's native API (`generativelanguage.googleapis.com`).

use serde_json::{json, Value};
use tokio::sync::Notify;

use super::{
    read_sse, send, status_error, AiError, AiErrorKind, AiResult, ChatRequest, Endpoint, Role,
    Usage,
};

fn request(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    method: reqwest::Method,
    path: &str,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .request(method, format!("{}{path}", endpoint.base_url))
        .timeout(endpoint.timeout);
    if let Some(key) = &endpoint.key {
        // In a header, never in the query string, so it cannot end up in a log.
        builder = builder.header("x-goog-api-key", key);
    }
    builder
}

pub async fn list_models(http: &reqwest::Client, endpoint: &Endpoint) -> AiResult<Vec<String>> {
    let mut models = Vec::new();
    let mut page: Option<String> = None;
    for _ in 0..10 {
        let mut builder = request(http, endpoint, reqwest::Method::GET, "/v1beta/models")
            .query(&[("pageSize", "1000")]);
        if let Some(token) = &page {
            builder = builder.query(&[("pageToken", token)]);
        }
        let response = builder.send().await.map_err(super::transport_error)?;
        if !response.status().is_success() {
            return Err(status_error(response).await);
        }
        let body: Value = response.json().await.map_err(super::transport_error)?;
        for model in body["models"].as_array().into_iter().flatten() {
            let generates = model["supportedGenerationMethods"]
                .as_array()
                .is_none_or(|methods| methods.iter().any(|m| m == "generateContent"));
            if let (true, Some(name)) = (generates, model["name"].as_str()) {
                models.push(name.strip_prefix("models/").unwrap_or(name).to_string());
            }
        }
        match body["nextPageToken"].as_str() {
            Some(token) if !token.is_empty() => page = Some(token.to_string()),
            _ => break,
        }
    }
    Ok(models)
}

pub fn body(request: &ChatRequest) -> Value {
    json!({
        "systemInstruction": { "parts": [{ "text": request.system }] },
        "contents": request.messages.iter().map(|m| json!({
            "role": if m.role == Role::User { "user" } else { "model" },
            "parts": [{ "text": m.content }],
        })).collect::<Vec<_>>(),
    })
}

pub async fn stream_chat(
    http: &reqwest::Client,
    endpoint: &Endpoint,
    chat: &ChatRequest,
    cancel: &Notify,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AiResult<Usage> {
    let model = chat.model.strip_prefix("models/").unwrap_or(&chat.model);
    let path = format!("/v1beta/models/{model}:streamGenerateContent");
    let builder = request(http, endpoint, reqwest::Method::POST, &path)
        .query(&[("alt", "sse")])
        .json(&body(chat));
    let response = send(builder, cancel).await?;
    if !response.status().is_success() {
        return Err(status_error(response).await);
    }
    let mut usage = Usage::default();
    read_sse(response, cancel, |data| {
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            return Ok(true);
        };
        if let Some(message) = event["error"]["message"].as_str() {
            return Err(AiError::other(format!(
                "The provider reported an error: {message}"
            )));
        }
        if let Some(reason) = event["promptFeedback"]["blockReason"].as_str() {
            return Err(AiError::new(
                AiErrorKind::Refused,
                format!("The model declined to answer this request ({reason})."),
            ));
        }
        for part in event["candidates"][0]["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
        {
            // Thought summaries are flagged; they are not part of the answer.
            if part["thought"].as_bool() == Some(true) {
                continue;
            }
            if let Some(text) = part["text"].as_str() {
                on_delta(text);
            }
        }
        if matches!(
            event["candidates"][0]["finishReason"].as_str(),
            Some("SAFETY" | "PROHIBITED_CONTENT" | "BLOCKLIST")
        ) {
            return Err(AiError::new(
                AiErrorKind::Refused,
                "The model declined to answer this request.",
            ));
        }
        let meta = &event["usageMetadata"];
        if let Some(tokens) = meta["promptTokenCount"].as_u64() {
            usage.input_tokens = Some(tokens);
        }
        if let Some(tokens) = meta["candidatesTokenCount"].as_u64() {
            usage.output_tokens = Some(tokens + meta["thoughtsTokenCount"].as_u64().unwrap_or(0));
        }
        Ok(true)
    })
    .await?;
    Ok(usage)
}

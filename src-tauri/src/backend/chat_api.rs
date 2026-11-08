//! Client for OpenAI-compatible `/chat/completions` endpoints.
//!
//! Extra request fields (the `*_extra_body` settings, e.g. Doubao's
//! `thinking: disabled`) are vendor specific: other models reject them with
//! HTTP 400. When the error names one of those fields, the request is sent
//! again without them and the endpoint + model is remembered, so later calls
//! skip the extra fields straight away.
//!
//! Busy or rate-limited answers (429 and 5xx gateway errors) are retried a few
//! times. The wait follows the server's `Retry-After` header or Google's
//! RetryInfo `retryDelay`; a suggested wait longer than [`MAX_RETRY_DELAY`]
//! (e.g. a daily quota) is not waited out.
//!
//! Ported from the old backend's `app/core/chat_api.py` (the non-streaming
//! half; streaming moves over with the agent).

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use serde_json::{Map, Value};

/// Fields the extra body may never override.
const PROTECTED_FIELDS: [&str; 4] = ["model", "messages", "tools", "tool_choice"];
const RETRYABLE_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];
/// Waits before each retry when the server suggests none.
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];
pub const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);
/// How much of an error body is kept in the message.
const ERROR_TEXT_CHARS: usize = 500;

pub type JsonObject = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatError {
    pub message: String,
    /// `None` when no HTTP answer arrived at all.
    pub status: Option<u16>,
}

impl std::fmt::Display for ChatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[derive(Debug)]
pub struct ChatResult {
    pub message: JsonObject,
    pub extra_body_dropped: bool,
}

/// Parses an extra-fields setting; an empty string means no fields.
pub fn parse_extra_body(raw: &str) -> Result<JsonObject, String> {
    if raw.trim().is_empty() {
        return Ok(JsonObject::new());
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(mut fields)) => {
            fields.retain(|key, _| !PROTECTED_FIELDS.contains(&key.as_str()));
            Ok(fields)
        }
        Ok(_) => Err("must be a JSON object".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// The text of an assistant message; some APIs return a list of parts.
pub fn message_text(message: &JsonObject) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text"))
            .map(|text| match text {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub struct ChatApi {
    client: reqwest::Client,
    retry_delays: Vec<Duration>,
    /// (base URL, model) pairs whose API rejected the extra fields.
    rejected_extra_body: Mutex<HashSet<(String, String)>>,
}

impl Default for ChatApi {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatApi {
    pub fn new() -> Self {
        Self::with_retry_delays(RETRY_DELAYS.to_vec())
    }

    /// Tests pass zero delays so retries do not wait.
    pub fn with_retry_delays(retry_delays: Vec<Duration>) -> Self {
        Self {
            client: reqwest::Client::new(),
            retry_delays,
            rejected_extra_body: Mutex::new(HashSet::new()),
        }
    }

    pub fn forget_rejected_extra_body(&self) {
        self.rejected().clear();
    }

    pub fn extra_body_rejected(&self, base_url: &str, model: &str) -> bool {
        self.rejected()
            .contains(&(base_url.to_owned(), model.to_owned()))
    }

    fn rejected(&self) -> std::sync::MutexGuard<'_, HashSet<(String, String)>> {
        self.rejected_extra_body
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub async fn post_chat_completion(
        &self,
        base_url: &str,
        api_key: &str,
        payload: &JsonObject,
        extra_body: &JsonObject,
        timeout: Duration,
    ) -> Result<ChatResult, ChatError> {
        let model = payload
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let use_extra = !extra_body.is_empty() && !self.extra_body_rejected(base_url, &model);
        let body = if use_extra {
            let mut merged = payload.clone();
            merged.extend(extra_body.clone());
            merged
        } else {
            payload.clone()
        };

        let mut reply = self
            .with_retries(base_url, api_key, &body, timeout, &model)
            .await?;
        let mut dropped = false;
        if use_extra && reply.status == 400 && mentions_any(&reply.text, extra_body) {
            let mut fields: Vec<&String> = extra_body.keys().collect();
            fields.sort();
            log::warn!(
                "chat API rejected extra fields {fields:?} for model={model}; retrying without them"
            );
            self.rejected().insert((base_url.to_owned(), model.clone()));
            reply = self
                .with_retries(base_url, api_key, payload, timeout, &model)
                .await?;
            dropped = true;
        }

        if reply.status >= 400 {
            return Err(ChatError {
                message: format!("HTTP {}: {}", reply.status, truncate(&reply.text)),
                status: Some(reply.status),
            });
        }
        let message = serde_json::from_str::<Value>(&reply.text)
            .ok()
            .and_then(|body| body.get("choices")?.get(0)?.get("message").cloned());
        match message {
            Some(Value::Object(message)) => Ok(ChatResult {
                message,
                extra_body_dropped: dropped,
            }),
            _ => Err(ChatError {
                message: format!("unexpected response body: {}", truncate(&reply.text)),
                status: Some(reply.status),
            }),
        }
    }

    /// Send, and send again after a wait while the server answers busy or
    /// rate limited (see the module docs).
    async fn with_retries(
        &self,
        base_url: &str,
        api_key: &str,
        body: &JsonObject,
        timeout: Duration,
        model: &str,
    ) -> Result<Reply, ChatError> {
        for (attempt, default_delay) in self.retry_delays.iter().enumerate() {
            let reply = self.send(base_url, api_key, body, timeout).await?;
            if !RETRYABLE_STATUSES.contains(&reply.status) {
                return Ok(reply);
            }
            let delay = reply.suggested_retry_delay().unwrap_or(*default_delay);
            if delay > MAX_RETRY_DELAY {
                return Ok(reply);
            }
            log::warn!(
                "chat API answered HTTP {} for model={model}; retrying in {:.0}s ({}/{})",
                reply.status,
                delay.as_secs_f64(),
                attempt + 1,
                self.retry_delays.len()
            );
            tokio::time::sleep(delay).await;
        }
        self.send(base_url, api_key, body, timeout).await
    }

    async fn send(
        &self,
        base_url: &str,
        api_key: &str,
        body: &JsonObject,
        timeout: Duration,
    ) -> Result<Reply, ChatError> {
        let mut request = self
            .client
            .post(format!("{base_url}/chat/completions"))
            .timeout(timeout)
            .json(body);
        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }
        let failed = |error: reqwest::Error| ChatError {
            message: format!("request failed: {}", error_chain(&error)),
            status: None,
        };
        let response = request.send().await.map_err(failed)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = response.text().await.map_err(failed)?;
        Ok(Reply {
            status,
            retry_after,
            text,
        })
    }
}

struct Reply {
    status: u16,
    retry_after: Option<String>,
    text: String,
}

impl Reply {
    fn suggested_retry_delay(&self) -> Option<Duration> {
        if let Some(seconds) = self
            .retry_after
            .as_deref()
            .and_then(|value| value.trim().parse::<f64>().ok())
        {
            // An HTTP-date Retry-After does not parse and falls through.
            return Some(Duration::from_secs_f64(seconds.max(0.0)));
        }
        retry_delay_in_body(&self.text)
    }
}

/// Google's RetryInfo: `"retryDelay": "12s"` (or `"1.5s"`).
fn retry_delay_in_body(text: &str) -> Option<Duration> {
    let key = "\"retryDelay\"";
    let mut rest = text;
    while let Some(index) = rest.find(key) {
        rest = &rest[index + key.len()..];
        if let Some(seconds) = parse_delay_value(rest) {
            return Some(Duration::from_secs_f64(seconds));
        }
    }
    None
}

fn parse_delay_value(text: &str) -> Option<f64> {
    let text = text.trim_start().strip_prefix(':')?.trim_start();
    let text = text.strip_prefix('"')?;
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (number, after) = text.split_at(end);
    let valid = !number.is_empty()
        && !number.starts_with('.')
        && !number.ends_with('.')
        && number.matches('.').count() <= 1;
    if valid && after.starts_with("s\"") {
        number.parse().ok()
    } else {
        None
    }
}

fn mentions_any(text: &str, fields: &JsonObject) -> bool {
    let lowered = text.to_lowercase();
    fields
        .keys()
        .any(|field| lowered.contains(&field.to_lowercase()))
}

fn truncate(text: &str) -> String {
    text.chars().take(ERROR_TEXT_CHARS).collect()
}

/// reqwest's own message is terse ("error sending request"); the causes say
/// what actually went wrong (refused, timed out, certificate, ...).
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_body_must_be_an_object_and_keeps_the_conversation() {
        assert!(parse_extra_body("").unwrap().is_empty());
        assert!(parse_extra_body("  ").unwrap().is_empty());
        assert!(parse_extra_body("[1]").is_err());
        assert!(parse_extra_body("not json").is_err());
        let fields =
            parse_extra_body(r#"{"thinking": {"type": "disabled"}, "model": "x", "messages": []}"#)
                .unwrap();
        assert_eq!(fields.keys().collect::<Vec<_>>(), ["thinking"]);
    }

    #[test]
    fn message_text_joins_content_parts() {
        let message = serde_json::json!({"content": [{"text": "a"}, {"type": "x"}, {"text": "b"}]});
        assert_eq!(message_text(message.as_object().unwrap()), "ab");
        let message = serde_json::json!({"content": null});
        assert_eq!(message_text(message.as_object().unwrap()), "");
    }

    #[test]
    fn retry_delay_is_read_from_google_retry_info() {
        let body = r#"{"error": {"details": [{"retryDelay": "12s"}]}}"#;
        assert_eq!(retry_delay_in_body(body), Some(Duration::from_secs(12)));
        assert_eq!(
            retry_delay_in_body(r#""retryDelay":"1.5s""#),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(retry_delay_in_body(r#""retryDelay": 12"#), None);
        assert_eq!(retry_delay_in_body("busy"), None);
    }
}

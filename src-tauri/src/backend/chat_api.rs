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
//! [`ChatApi::stream_chat_completion`] reads the answer as server-sent events
//! instead: chunks arrive while the model still thinks, so the timeout applies
//! between chunks rather than to the whole silent generation.
//!
//! Ported from the old backend's `app/core/chat_api.py`.

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

    /// `POST /chat/completions` with `stream: true`, the deltas put back
    /// together into one assistant message (content, `reasoning_content`
    /// and `tool_calls`). `timeout` limits the wait for the answer to start
    /// and then for each chunk.
    pub async fn stream_chat_completion(
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
        let mut stream_payload = payload.clone();
        stream_payload.insert("stream".to_owned(), Value::Bool(true));
        let use_extra = !extra_body.is_empty() && !self.extra_body_rejected(base_url, &model);
        let body = if use_extra {
            let mut merged = stream_payload.clone();
            merged.extend(extra_body.clone());
            merged
        } else {
            stream_payload.clone()
        };

        let mut answer = self
            .stream_with_retries(base_url, api_key, &body, timeout, &model)
            .await?;
        let mut dropped = false;
        if let StreamAnswer::Failed(reply) = &answer
            && use_extra
            && reply.status == 400
            && mentions_any(&reply.text, extra_body)
        {
            let mut fields: Vec<&String> = extra_body.keys().collect();
            fields.sort();
            log::warn!(
                "chat API rejected extra fields {fields:?} for model={model}; retrying without them"
            );
            self.rejected().insert((base_url.to_owned(), model.clone()));
            answer = self
                .stream_with_retries(base_url, api_key, &stream_payload, timeout, &model)
                .await?;
            dropped = true;
        }
        match answer {
            StreamAnswer::Failed(reply) => Err(ChatError {
                message: format!("HTTP {}: {}", reply.status, truncate(&reply.text)),
                status: Some(reply.status),
            }),
            StreamAnswer::Live(response) => Ok(ChatResult {
                message: consume_stream(response, timeout).await?,
                extra_body_dropped: dropped,
            }),
        }
    }

    async fn stream_with_retries(
        &self,
        base_url: &str,
        api_key: &str,
        body: &JsonObject,
        timeout: Duration,
        model: &str,
    ) -> Result<StreamAnswer, ChatError> {
        for (attempt, default_delay) in self.retry_delays.iter().enumerate() {
            let answer = self.send_stream(base_url, api_key, body, timeout).await?;
            let StreamAnswer::Failed(reply) = &answer else {
                return Ok(answer);
            };
            if !RETRYABLE_STATUSES.contains(&reply.status) {
                return Ok(answer);
            }
            let delay = reply.suggested_retry_delay().unwrap_or(*default_delay);
            if delay > MAX_RETRY_DELAY {
                return Ok(answer);
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
        self.send_stream(base_url, api_key, body, timeout).await
    }

    async fn send_stream(
        &self,
        base_url: &str,
        api_key: &str,
        body: &JsonObject,
        timeout: Duration,
    ) -> Result<StreamAnswer, ChatError> {
        let mut request = self
            .client
            .post(format!("{base_url}/chat/completions"))
            .json(body);
        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }
        let failed = |error: reqwest::Error| ChatError {
            message: format!("request failed: {}", error_chain(&error)),
            status: None,
        };
        let response = tokio::time::timeout(timeout, request.send())
            .await
            .map_err(|_| timed_out("request failed", timeout))?
            .map_err(failed)?;
        let status = response.status().as_u16();
        if status < 400 {
            return Ok(StreamAnswer::Live(response));
        }
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let text = tokio::time::timeout(timeout, response.text())
            .await
            .map_err(|_| timed_out("request failed", timeout))?
            .map_err(failed)?;
        Ok(StreamAnswer::Failed(Reply {
            status,
            retry_after,
            text,
        }))
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

enum StreamAnswer {
    /// A success status: the events are still to be read.
    Live(reqwest::Response),
    Failed(Reply),
}

fn timed_out(what: &str, timeout: Duration) -> ChatError {
    ChatError {
        message: format!("{what}: no answer within {}s", timeout.as_secs()),
        status: None,
    }
}

/// The `data:` payload of one server-sent event line, if it carries one.
fn event_data(line: &str) -> Option<&str> {
    let data = line.strip_prefix("data:")?.trim();
    (!data.is_empty()).then_some(data)
}

/// Reads the stream to its end (or `[DONE]`) and joins the deltas.
async fn consume_stream(
    response: reqwest::Response,
    timeout: Duration,
) -> Result<JsonObject, ChatError> {
    use futures_util::StreamExt;

    let mut assembler = StreamAssembler::default();
    let mut stream = response.bytes_stream();
    let mut pending: Vec<u8> = Vec::new();
    'read: loop {
        let chunk = match tokio::time::timeout(timeout, stream.next()).await {
            Err(_) => return Err(timed_out("stream failed", timeout)),
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                return Err(ChatError {
                    message: format!("stream failed: {}", error_chain(&error)),
                    status: None,
                });
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if !assembler.line(line.trim_end_matches(['\r', '\n'])) {
                break 'read;
            }
        }
    }
    if !pending.is_empty() {
        assembler.line(String::from_utf8_lossy(&pending).trim_end_matches('\r'));
    }
    Ok(assembler.finish())
}

/// Puts streamed chat deltas back together into one message.
#[derive(Default)]
pub struct StreamAssembler {
    content: String,
    reasoning: String,
    /// By the delta's `index`, in order.
    tool_calls: std::collections::BTreeMap<i64, JsonObject>,
}

impl StreamAssembler {
    /// Takes one line of the stream; `false` once the stream says `[DONE]`.
    pub fn line(&mut self, line: &str) -> bool {
        let Some(data) = event_data(line) else {
            return true; // comments and keep-alives
        };
        if data == "[DONE]" {
            return false;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            return true;
        };
        let Some(delta) = chunk
            .get("choices")
            .and_then(|choices| choices.get(0))
            .and_then(|choice| choice.get("delta"))
        else {
            return true;
        };
        if let Some(Value::String(text)) = delta.get("content") {
            self.content.push_str(text);
        }
        if let Some(Value::String(text)) = delta.get("reasoning_content") {
            self.reasoning.push_str(text);
        }
        if let Some(Value::Array(calls)) = delta.get("tool_calls") {
            for raw in calls {
                self.tool_call(raw);
            }
        }
        true
    }

    fn tool_call(&mut self, raw: &Value) {
        let index = raw.get("index").and_then(Value::as_i64).unwrap_or(0);
        let entry = self.tool_calls.entry(index).or_insert_with(|| {
            let Value::Object(entry) =
                serde_json::json!({"id": "", "type": "function", "function": {"name": "", "arguments": ""}})
            else {
                unreachable!()
            };
            entry
        });
        for key in ["id", "type"] {
            if let Some(value) = raw.get(key).filter(|value| truthy(value)) {
                entry.insert(key.to_owned(), Value::String(plain_text(value)));
            }
        }
        let Some(function) = raw.get("function") else {
            return;
        };
        let Some(Value::Object(target)) = entry.get_mut("function") else {
            return;
        };
        for key in ["name", "arguments"] {
            if let Some(value) = function.get(key).filter(|value| truthy(value)) {
                let mut joined = target
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                joined.push_str(&plain_text(value));
                target.insert(key.to_owned(), Value::String(joined));
            }
        }
    }

    pub fn finish(self) -> JsonObject {
        let mut message = JsonObject::new();
        message.insert("role".to_owned(), Value::from("assistant"));
        message.insert("content".to_owned(), Value::String(self.content));
        if !self.reasoning.is_empty() {
            message.insert(
                "reasoning_content".to_owned(),
                Value::String(self.reasoning),
            );
        }
        if !self.tool_calls.is_empty() {
            let calls = self.tool_calls.into_values().map(Value::Object).collect();
            message.insert("tool_calls".to_owned(), Value::Array(calls));
        }
        message
    }
}

/// Python's truthiness for a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
        Value::Number(number) => number.as_f64() != Some(0.0),
    }
}

/// Python's `str()` of a JSON value: strings without their quotes.
fn plain_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
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
    fn streamed_deltas_are_joined_into_one_message() {
        let mut assembler = StreamAssembler::default();
        let lines = [
            ": keep-alive",
            r#"data: {"choices": [{"delta": {"content": "好"}}]}"#,
            r#"data: {"choices": [{"delta": {"reasoning_content": "hmm"}}]}"#,
            r#"data: {"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {"name": "gen", "arguments": "{\"a\""}}]}}]}"#,
            r#"data: {"choices": [{"delta": {"tool_calls": [{"index": 0, "function": {"arguments": ": 1}"}}]}}]}"#,
            r#"data: {"choices": [{"delta": {"content": "的"}}]}"#,
            "data: not json",
        ];
        for line in lines {
            assert!(assembler.line(line));
        }
        assert!(!assembler.line("data: [DONE]"));
        let message = Value::Object(assembler.finish());
        assert_eq!(
            message,
            serde_json::json!({
                "role": "assistant",
                "content": "好的",
                "reasoning_content": "hmm",
                "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "gen", "arguments": "{\"a\": 1}"}}],
            })
        );
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

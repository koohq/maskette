//! Connection test for the OpenAI-compatible LLM roles on the settings page.
//!
//! One small request per role checks that the endpoint, key and model work,
//! whether the extra request fields are accepted, and the capability the
//! role needs: tool calling for the chat assistant, image input for analysis.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use super::store::Settings;
use crate::backend::chat_api::{self, ChatApi, ChatError, JsonObject};

const PING_TOOL: &str = "ping";

/// A 64x64 PNG filled with red (220, 30, 30).
const RED_SQUARE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAf0lEQVR4nNXOQREAIAzAsFIh8y8KMYjgsWsU5NwZyiRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4iRO4twO/HoInQGY8Ba5YAAAAABJRU5ErkJggg==";

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct ProbeCheck {
    /// `connection`, `extra_body`, `tools` or `vision`.
    pub id: &'static str,
    pub status: CheckStatus,
    pub detail: String,
}

impl ProbeCheck {
    fn new(id: &'static str, status: CheckStatus, detail: impl Into<String>) -> Self {
        Self {
            id,
            status,
            detail: detail.into(),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct ProbeOut {
    pub ok: bool,
    pub model: String,
    pub base_url: String,
    pub latency_ms: Option<u64>,
    pub checks: Vec<ProbeCheck>,
}

type CapabilityCheck = fn(&JsonObject) -> ProbeCheck;

pub async fn probe_agent(settings: &Settings, chat: &ChatApi) -> ProbeOut {
    let base_url = settings
        .get("agent_llm_base_url")
        .trim()
        .trim_end_matches('/')
        .to_owned();
    let model = settings.get("agent_llm_model").trim().to_owned();
    let payload = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "You are a connectivity test."},
            {"role": "user", "content": "Call the ping tool with ok=true. Do not answer with text."},
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": PING_TOOL,
                "description": "Connectivity test. Call it when asked.",
                "parameters": {
                    "type": "object",
                    "properties": {"ok": {"type": "boolean"}},
                    "required": ["ok"],
                },
            },
        }],
        "tool_choice": "auto",
    });
    probe(
        chat,
        Endpoint {
            base_url,
            api_key: settings.agent_api_key(),
            model,
        },
        payload,
        settings.agent_extra_body(),
        Settings::AGENT_LLM_TIMEOUT,
        check_tool_call,
    )
    .await
}

pub async fn probe_analysis(settings: &Settings, chat: &ChatApi) -> ProbeOut {
    let (base_url, api_key, model) = settings.analysis_endpoint();
    let payload = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "What colour fills this image? Answer with one English word."},
                {"type": "image_url", "image_url": {"url": RED_SQUARE}},
            ],
        }],
    });
    probe(
        chat,
        Endpoint {
            base_url,
            api_key,
            model,
        },
        payload,
        settings.analysis_extra_body(),
        Settings::ANALYSIS_LLM_TIMEOUT,
        check_vision,
    )
    .await
}

struct Endpoint {
    base_url: String,
    api_key: String,
    model: String,
}

async fn probe(
    chat: &ChatApi,
    endpoint: Endpoint,
    payload: Value,
    extra_body: JsonObject,
    timeout: Duration,
    check_capability: CapabilityCheck,
) -> ProbeOut {
    let Endpoint {
        base_url,
        api_key,
        model,
    } = endpoint;
    if base_url.is_empty() || model.is_empty() {
        return failed(base_url, model, "missing_endpoint", "");
    }
    let Value::Object(payload) = payload else {
        unreachable!("the probe payloads are JSON objects")
    };
    // Test from scratch: an earlier rejection of the extra fields must not
    // hide the answer.
    chat.forget_rejected_extra_body();
    let started = Instant::now();
    let result = match chat
        .post_chat_completion(&base_url, &api_key, &payload, &extra_body, timeout)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            let reason = classify(&error);
            return failed(base_url, model, reason, &error.message);
        }
    };
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let mut checks = vec![ProbeCheck::new("connection", CheckStatus::Ok, "")];
    if !extra_body.is_empty() {
        let mut fields: Vec<&str> = extra_body.keys().map(String::as_str).collect();
        fields.sort_unstable();
        let status = if result.extra_body_dropped {
            CheckStatus::Warn
        } else {
            CheckStatus::Ok
        };
        checks.push(ProbeCheck::new("extra_body", status, fields.join(", ")));
    }
    checks.push(check_capability(&result.message));
    ProbeOut {
        ok: checks.iter().all(|check| check.status != CheckStatus::Fail),
        model,
        base_url,
        latency_ms: Some(latency_ms),
        checks,
    }
}

fn check_tool_call(message: &JsonObject) -> ProbeCheck {
    let called = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| call.get("function")?.get("name")?.as_str())
        .any(|name| name.trim() == PING_TOOL);
    if called {
        return ProbeCheck::new("tools", CheckStatus::Ok, "");
    }
    let text = strip_reasoning(&chat_api::message_text(message));
    ProbeCheck::new("tools", CheckStatus::Fail, first_chars(text.trim(), 200))
}

fn check_vision(message: &JsonObject) -> ProbeCheck {
    let text = chat_api::message_text(message);
    let answer = text.trim();
    if answer.to_lowercase().contains("red") || answer.contains('红') {
        return ProbeCheck::new("vision", CheckStatus::Ok, first_chars(answer, 80));
    }
    ProbeCheck::new("vision", CheckStatus::Warn, first_chars(answer, 200))
}

fn classify(error: &ChatError) -> &'static str {
    let text = error.message.to_lowercase();
    match error.status {
        Some(401 | 403) => "auth",
        Some(404) => "not_found",
        None => "unreachable",
        Some(_)
            if ["image", "vision", "multimodal", "image_url"]
                .iter()
                .any(|word| text.contains(word)) =>
        {
            "no_vision"
        }
        Some(_) => "error",
    }
}

fn failed(base_url: String, model: String, reason: &str, detail: &str) -> ProbeOut {
    let detail = if detail.is_empty() {
        reason.to_owned()
    } else {
        format!("{reason}: {detail}")
    };
    ProbeOut {
        ok: false,
        model,
        base_url,
        latency_ms: None,
        checks: vec![ProbeCheck::new("connection", CheckStatus::Fail, detail)],
    }
}

/// Some OpenAI-compatible reasoning models inline their thinking in
/// `<think>` tags.
fn strip_reasoning(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        let Some(end) = rest[start..].find("</think>") else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + end + "</think>".len()..];
    }
    out.push_str(rest);
    out
}

fn first_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_tags_are_removed() {
        assert_eq!(strip_reasoning("<think>hm</think>pong"), "pong");
        assert_eq!(
            strip_reasoning("a<think>1</think>b<think>2</think>c"),
            "abc"
        );
        assert_eq!(strip_reasoning("open <think> only"), "open <think> only");
    }

    #[test]
    fn the_red_square_is_a_png() {
        assert!(RED_SQUARE.starts_with("data:image/png;base64,iVBORw0KGgo"));
    }
}

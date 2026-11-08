//! The chat model behind the agent.
//!
//! Messages are kept in the OpenAI chat format (role / content / tool_calls /
//! tool_call_id). The OpenAI-compatible backend sends them as they are, with
//! native function calling, streamed. The Claude Code backend moves over with
//! the CLI paths (the last step of the port).
//!
//! Ported from the old backend's `app/agent/llm.py`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::backend::chat_api::{self, ChatApi, JsonObject};
use crate::backend::ids;
use crate::backend::settings::store::Settings;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: JsonObject,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssistantTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

impl AssistantTurn {
    /// The turn as a transcript message.
    pub fn to_message(&self) -> Value {
        let mut message = json!({"role": "assistant", "content": self.text});
        if !self.tool_calls.is_empty() {
            message["tool_calls"] = self
                .tool_calls
                .iter()
                .map(|call| {
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": Value::Object(call.arguments.clone()).to_string(),
                        },
                    })
                })
                .collect();
        }
        message
    }
}

pub fn tool_payload(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {"name": tool.name, "description": tool.description, "parameters": tool.parameters},
    })
}

/// One assistant turn for the transcript so far.
pub async fn complete(
    settings: &Settings,
    chat: &ChatApi,
    system_prompt: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Result<AssistantTurn, String> {
    match settings.get("agent_llm_provider").trim() {
        "" | "openai_compatible" => {}
        "claude_code" => {
            return Err(
                "The Claude Code chat assistant has not moved to the new backend yet; \
                 choose openai_compatible in the settings."
                    .to_owned(),
            );
        }
        other => return Err(format!("Unsupported agent_llm_provider={other:?}")),
    }
    let base_url = settings
        .get("agent_llm_base_url")
        .trim()
        .trim_end_matches('/');
    let model = settings.get("agent_llm_model").trim();
    if base_url.is_empty() || model.is_empty() {
        return Err(
            "agent_llm_base_url and agent_llm_model must be set for the openai_compatible agent"
                .to_owned(),
        );
    }
    let mut wire = vec![json!({"role": "system", "content": system_prompt})];
    wire.extend(messages.iter().map(wire_message));
    let mut payload = JsonObject::new();
    payload.insert("model".to_owned(), Value::from(model));
    payload.insert("messages".to_owned(), Value::Array(wire));
    if !tools.is_empty() {
        payload.insert("tools".to_owned(), tools.iter().map(tool_payload).collect());
        payload.insert("tool_choice".to_owned(), Value::from("auto"));
    }
    let result = chat
        .stream_chat_completion(
            base_url,
            &settings.agent_api_key(),
            &payload,
            &settings.agent_extra_body(),
            Settings::AGENT_LLM_TIMEOUT,
        )
        .await
        .map_err(|error| format!("Agent LLM {error}"))?;
    Ok(parse_openai_message(&result.message))
}

/// The stored tool name is only for the transcript; some providers reject
/// extra fields.
fn wire_message(message: &Value) -> Value {
    let mut message = message.clone();
    if message.get("role").and_then(Value::as_str) == Some("tool")
        && let Value::Object(fields) = &mut message
    {
        fields.remove("name");
    }
    message
}

static THINKING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").expect("think tags"));

pub fn parse_openai_message(message: &JsonObject) -> AssistantTurn {
    // Some reasoning models inline their thinking in <think> tags.
    let text = THINKING
        .replace_all(&chat_api::message_text(message), "")
        .trim()
        .to_owned();
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| calls.iter().filter_map(parse_tool_call).collect())
        .unwrap_or_default();
    AssistantTurn { text, tool_calls }
}

fn parse_tool_call(raw: &Value) -> Option<ToolCall> {
    let function = raw.get("function");
    let name = function
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if name.is_empty() {
        return None;
    }
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map_or_else(|| format!("call_{}", ids::short_hex(12)), str::to_owned);
    Some(ToolCall {
        id,
        name: name.to_owned(),
        arguments: parse_arguments(function.and_then(|function| function.get("arguments"))),
    })
}

fn parse_arguments(raw: Option<&Value>) -> JsonObject {
    match raw {
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(Value::String(text)) => match serde_json::from_str(text) {
            Ok(Value::Object(arguments)) => arguments,
            _ => JsonObject::new(),
        },
        _ => JsonObject::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_messages_give_text_and_tool_calls() {
        let message = json!({
            "content": "<think>hmm</think>好的",
            "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "generate_front_view", "arguments": "{\"instructions\":\"x\"}"}},
                {"id": "c2", "type": "function", "function": {"name": "analyze_references", "arguments": "not json"}},
                {"id": "c3", "type": "function", "function": {"name": " ", "arguments": "{}"}},
            ],
        });
        let turn = parse_openai_message(message.as_object().unwrap());
        assert_eq!(turn.text, "好的");
        let calls: Vec<(&str, Value)> = turn
            .tool_calls
            .iter()
            .map(|call| (call.name.as_str(), Value::Object(call.arguments.clone())))
            .collect();
        assert_eq!(
            calls,
            [
                ("generate_front_view", json!({"instructions": "x"})),
                ("analyze_references", json!({})),
            ]
        );
    }

    #[test]
    fn a_missing_call_id_gets_one() {
        let message = json!({"content": "", "tool_calls": [{"function": {"name": "x"}}]});
        let turn = parse_openai_message(message.as_object().unwrap());
        assert!(turn.tool_calls[0].id.starts_with("call_"));
        assert_eq!(turn.tool_calls[0].id.len(), 17);
    }

    #[test]
    fn tool_results_go_out_without_their_name() {
        let message = json!({"role": "tool", "tool_call_id": "c", "name": "x", "content": "{}"});
        assert_eq!(
            wire_message(&message),
            json!({"role": "tool", "tool_call_id": "c", "content": "{}"})
        );
    }
}

//! The chat model behind the agent.
//!
//! Messages are kept in the OpenAI chat format (role / content / tool_calls /
//! tool_call_id). The OpenAI-compatible backend sends them as they are, with
//! native function calling, streamed. The CLIs (Codex, Claude Code) have no
//! client-side tool calling, so for them the transcript is written out as
//! text and the answer asked for as JSON (the reply and the tool calls).
//!
//! Ported from the old backend's `app/agent/llm.py`.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::backend::chat_api::{self, ChatApi, JsonObject};
use crate::backend::cli::{self, Cli};
use crate::backend::failure::Failure;
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
    /// Neither a reply nor a tool call.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.tool_calls.is_empty()
    }

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

/// Where a turn goes, and the conversation's own folder: a CLI keeps the
/// prompt and output of each step in its `codex/` or `claude-code/`.
pub struct Backends<'a> {
    pub chat: &'a ChatApi,
    pub cli: &'a Cli,
    pub workspace: &'a Path,
}

/// One assistant turn for the transcript so far.
pub async fn complete(
    settings: &Settings,
    backends: &Backends<'_>,
    system_prompt: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Result<AssistantTurn, Failure> {
    match settings.get("agent_llm_provider").trim() {
        "" | "openai_compatible" => {}
        "codex" => {
            let workspace = backends.workspace.join("codex");
            let text = cli::codex::chat(
                backends.cli,
                &workspace,
                &next_step_label(&workspace),
                &cli_prompt(system_prompt, messages, tools),
                settings.get("agent_codex_model"),
            )
            .await?;
            return Ok(parse_cli_reply(&text));
        }
        "claude_code" => {
            let workspace = backends.workspace.join("claude-code");
            let text = cli::claude::run(
                backends.cli,
                &workspace,
                &next_step_label(&workspace),
                &cli_prompt(system_prompt, messages, tools),
                cli::claude::Tools::None,
                settings.get("agent_claude_code_model"),
            )
            .await?;
            return Ok(parse_cli_reply(&text));
        }
        other => return Err(format!("Unsupported agent_llm_provider={other:?}").into()),
    }
    let chat = backends.chat;
    let base_url = settings
        .get("agent_llm_base_url")
        .trim()
        .trim_end_matches('/');
    let model = settings.get("agent_llm_model").trim();
    if base_url.is_empty() || model.is_empty() {
        let missing: Vec<&str> = [("agent_llm_base_url", base_url), ("agent_llm_model", model)]
            .into_iter()
            .filter(|(_, value)| value.is_empty())
            .map(|(setting, _)| setting)
            .collect();
        return Err(Failure::missing_settings(
            &missing,
            "agent_llm_base_url and agent_llm_model must be set for the openai_compatible agent",
        ));
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
        .map_err(|error| error.to_failure(base_url, "Agent LLM "))?;
    let turn = parse_openai_message(&result.message);
    if turn.is_empty() {
        // How the answer ended tells a model that chose to say nothing
        // (`stop`) from one that was cut short or whose call broke.
        log::warn!(
            "Agent LLM model={model} gave an empty reply (ending: {}; content before removing <think>: {} chars)",
            result.ending.as_deref().unwrap_or("not streamed"),
            chat_api::message_text(&result.message).chars().count(),
        );
    }
    Ok(turn)
}

/// Names the kept prompt and output files of the next CLI step
/// (`agent-turn-0001-prompt.md`, …). Every step of the conversation gets its
/// own number, so a turn that calls tools keeps all its steps, not only the
/// last.
fn next_step_label(workspace: &Path) -> String {
    let last = std::fs::read_dir(workspace)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .strip_prefix("agent-turn-")?
                .strip_suffix("-prompt.md")?
                .parse::<u32>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    format!("agent-turn-{:04}", last + 1)
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

pub const CLI_REPLY_FORMAT: &str = r#"Reply with ONLY one JSON object and nothing else:
{"reply": "<text shown to the user; may be empty while you call tools>",
 "tool_calls": [{"name": "<tool name>", "arguments": {...}}]}
Use an empty tool_calls list when you are done and waiting for the user. Never invent tool names."#;

/// The system prompt, the tools and the transcript as one text prompt.
pub fn cli_prompt(system_prompt: &str, messages: &[Value], tools: &[ToolSpec]) -> String {
    let text = |message: &Value, key: &str| -> String {
        message
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let mut prompt = format!("{system_prompt}\n\n## Tools\n");
    for tool in tools {
        let _ = writeln!(prompt, "- {}: {}", tool.name, tool.description);
        let _ = writeln!(prompt, "  arguments schema: {}", tool.parameters);
    }
    prompt.push_str("\n## Conversation so far\n");
    for message in messages {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                let _ = writeln!(prompt, "[user] {}", text(message, "content"));
            }
            Some("assistant") => {
                let content = text(message, "content");
                if !content.is_empty() {
                    let _ = writeln!(prompt, "[assistant] {content}");
                }
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let function = call.get("function").unwrap_or(&Value::Null);
                    let arguments = text(function, "arguments");
                    let arguments = if arguments.is_empty() {
                        "{}"
                    } else {
                        &arguments
                    };
                    let _ = writeln!(
                        prompt,
                        "[assistant called {}] {arguments}",
                        text(function, "name")
                    );
                }
            }
            Some("tool") => {
                let _ = writeln!(prompt, "[tool result] {}", text(message, "content"));
            }
            _ => {}
        }
    }
    prompt.push_str("\n## Your turn\n");
    prompt.push_str(CLI_REPLY_FORMAT);
    prompt
}

static FENCED_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)```(?:json)?\s*(\{.*\})\s*```").expect("fenced JSON"));

/// The reply and tool calls in a CLI's answer; an answer that is not the
/// asked-for JSON counts as a plain reply rather than failing the turn.
pub fn parse_cli_reply(text: &str) -> AssistantTurn {
    let stripped = text.trim();
    let mut candidates: Vec<&str> = FENCED_JSON
        .captures(stripped)
        .and_then(|captures| captures.get(1))
        .map(|found| found.as_str())
        .into_iter()
        .collect();
    if let (Some(start), Some(end)) = (stripped.find('{'), stripped.rfind('}'))
        && start < end
    {
        candidates.push(&stripped[start..=end]);
    }
    let Some(payload) = candidates.into_iter().find_map(|candidate| {
        match serde_json::from_str::<Value>(candidate) {
            Ok(Value::Object(payload)) => Some(payload),
            _ => None,
        }
    }) else {
        return AssistantTurn {
            text: stripped.to_owned(),
            tool_calls: Vec::new(),
        };
    };
    let tool_calls = payload
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?.trim();
            (!name.is_empty()).then(|| ToolCall {
                id: format!("call_{}", ids::short_hex(12)),
                name: name.to_owned(),
                arguments: item
                    .get("arguments")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            })
        })
        .collect();
    AssistantTurn {
        text: payload
            .get("reply")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned(),
        tool_calls,
    }
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
    fn cli_replies_give_text_and_tool_calls() {
        let turn = parse_cli_reply(
            "Here you go:\n```json\n{\"reply\": \"見ます\", \"tool_calls\": [{\"name\": \"analyze_references\", \"arguments\": {\"x\": 1}}, {\"name\": \" \"}, \"junk\"]}\n```",
        );
        assert_eq!(turn.text, "見ます");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "analyze_references");
        assert_eq!(
            Value::Object(turn.tool_calls[0].arguments.clone()),
            json!({"x": 1})
        );
        assert!(turn.tool_calls[0].id.starts_with("call_"));

        let plain = parse_cli_reply("  ただの返事です  ");
        assert_eq!(plain.text, "ただの返事です");
        assert!(plain.tool_calls.is_empty());
    }

    #[test]
    fn the_cli_prompt_writes_out_the_transcript() {
        let tools = [ToolSpec {
            name: "generate_front_view",
            description: "Make the front view.",
            parameters: json!({"type": "object"}),
        }];
        let messages = [
            json!({"role": "user", "content": "作って"}),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "generate_front_view", "arguments": "{\"instructions\":\"x\"}"}},
            ]}),
            json!({"role": "tool", "tool_call_id": "c1", "name": "generate_front_view", "content": "{\"ok\":true}"}),
        ];
        let prompt = cli_prompt("SYSTEM", &messages, &tools);
        assert!(prompt.starts_with(
            "SYSTEM\n\n## Tools\n- generate_front_view: Make the front view.\n  arguments schema: {\"type\":\"object\"}\n"
        ));
        assert!(prompt.contains(
            "[user] 作って\n[assistant called generate_front_view] {\"instructions\":\"x\"}\n[tool result] {\"ok\":true}\n"
        ));
        assert!(prompt.ends_with(CLI_REPLY_FORMAT));
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

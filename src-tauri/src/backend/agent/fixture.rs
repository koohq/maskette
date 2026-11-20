//! The chat assistant's fixture, offered in debug builds like those of
//! analysis and image generation. There is no model: each message takes the
//! two-stage flow one step on, so the whole flow can be tried without an
//! API key or a CLI. Approving still takes the screen's approve buttons, as
//! with a real model; a message with text redoes the current stage's image
//! with the text as the change.
//!
//! The tools it calls run on the analysis and image settings as usual, so
//! they cost nothing only while those are fixtures too.

use serde_json::{Value, json};

use super::llm::{AssistantTurn, ToolCall};
use super::store::{AgentState, Conversation, event_type};
use super::tools::GENERATION_TOOLS;
use crate::backend::chat_api::JsonObject;
use crate::backend::failure::Failure;
use crate::backend::ids;
use crate::backend::settings::store::Settings;

/// The next step of the conversation: one tool call, or a reply that ends
/// the turn.
pub fn turn(settings: &Settings, conversation: &Conversation) -> Result<AssistantTurn, Failure> {
    if !settings.allow_fixture {
        return Err("No real chat backend is configured (agent_llm_provider).".into());
    }
    let lines = Lines::of(&conversation.locale);
    let messages = &conversation.llm_messages;
    let this_turn = messages
        .iter()
        .rposition(|message| message["role"] == "user")
        .map_or(&messages[..], |index| &messages[index + 1..]);
    if this_turn.iter().any(failed) {
        return Ok(say(lines.failed));
    }
    let called = called_tools(this_turn);
    if called.iter().any(|name| name == "write_order_notes") {
        return Ok(say(lines.notes));
    }
    if let Some(done) = called
        .iter()
        .find(|name| GENERATION_TOOLS.contains(&name.as_str()))
    {
        return Ok(say(match done.as_str() {
            "generate_turnaround" => lines.turnaround,
            "generate_eye_detail" => lines.eyes,
            _ => lines.generated,
        }));
    }
    let Some((name, arguments)) = next_tool(&conversation.state, &user_text(conversation)) else {
        return Ok(say(lines.need_reference));
    };
    // A step that did not move the state on would only repeat.
    if called.iter().any(|done| done == name) {
        return Ok(say(lines.failed));
    }
    Ok(AssistantTurn {
        text: String::new(),
        tool_calls: vec![ToolCall {
            id: format!("call_{}", ids::short_hex(12)),
            name: name.to_owned(),
            arguments,
        }],
    })
}

/// The tool that takes the flow on from `state`; `None` while there is
/// nothing to draw from. `text`, when there is any, goes along as the
/// instructions and makes an existing image of the stage be revised rather
/// than drawn again.
fn next_tool(state: &AgentState, text: &str) -> Option<(&'static str, JsonObject)> {
    let mut arguments = JsonObject::new();
    if !text.is_empty() {
        arguments.insert("instructions".to_owned(), json!(text));
    }
    let name = if let Some(front) = state.approved_front_id.as_deref() {
        // The turnaround first, then the eye close-up, then the order
        // notes, then the turnaround again on every later message.
        let made_from = |role: &str| {
            state
                .images
                .iter()
                .any(|image| image.role == role && image.parent_id.as_deref() == Some(front))
        };
        let has_notes = state
            .order_notes
            .as_ref()
            .is_some_and(|notes| notes.get("front_id").and_then(Value::as_str) == Some(front));
        if !made_from("turnaround") {
            "generate_turnaround"
        } else if !made_from("eyes") {
            "generate_eye_detail"
        } else if !has_notes {
            // The text is no correction to notes not written yet.
            arguments.clear();
            "write_order_notes"
        } else {
            "generate_turnaround"
        }
    } else if state.approved_design_id.is_some() {
        if state.current_front_id.is_some() && !text.is_empty() {
            "revise_front_view"
        } else {
            "generate_front_view"
        }
    } else if state.current_design_id.is_some() && !text.is_empty() {
        "revise_design"
    } else if state.references.is_empty() {
        return None;
    } else if state.analysis.is_none() {
        return Some(("analyze_references", JsonObject::new()));
    } else {
        "generate_design"
    };
    Some((name, arguments))
}

/// What the user typed in the latest message, without the attachments and
/// button clicks the transcript notes beside it.
fn user_text(conversation: &Conversation) -> String {
    conversation
        .events
        .iter()
        .rev()
        .find(|event| event_type(event) == "user_message")
        .and_then(|event| event.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn failed(message: &Value) -> bool {
    message["role"] == "tool"
        && message["content"]
            .as_str()
            .and_then(|content| serde_json::from_str::<Value>(content).ok())
            .is_none_or(|result| result["ok"] != true)
}

fn called_tools(messages: &[Value]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| message["tool_calls"].as_array())
        .flatten()
        .filter_map(|call| call["function"]["name"].as_str())
        .map(str::to_owned)
        .collect()
}

fn say(text: &str) -> AssistantTurn {
    AssistantTurn {
        text: text.to_owned(),
        tool_calls: Vec::new(),
    }
}

/// The replies, in the conversation's language.
struct Lines {
    need_reference: &'static str,
    generated: &'static str,
    turnaround: &'static str,
    eyes: &'static str,
    notes: &'static str,
    failed: &'static str,
}

impl Lines {
    fn of(locale: &str) -> Self {
        match locale {
            "ja" => Self {
                need_reference: "テスト用サンプルの返事です。参考画像を添えて送ると、設定画のサンプルを作ります。",
                generated: "サンプルの画像を作りました。承認すると次の段階に進みます。文字を送ると、それを指示にして作り直します。",
                turnaround: "マスク四面図のサンプルを作りました。もう一度送ると、目の拡大図のサンプルを作ります。",
                eyes: "目の拡大図のサンプルを作りました。もう一度送ると、発注メモのサンプルを書きます。",
                notes: "発注メモのサンプルを書きました。もう一度送ると、四面図を作り直します。",
                failed: "うまくいきませんでした。上のエラーを確かめてください。",
            },
            "zh-CN" => Self {
                need_reference: "这是测试用示例的回复。附上参考图发送后，会生成示例设定图。",
                generated: "已生成示例图。批准后进入下一阶段；发送文字则按文字重新生成。",
                turnaround: "已生成头壳四视图示例。再次发送会生成眼部特写示例。",
                eyes: "已生成眼部特写示例。再次发送会写定制说明示例。",
                notes: "已写好定制说明示例。再次发送会重新生成四视图。",
                failed: "没有成功，请查看上面的错误。",
            },
            _ => Self {
                need_reference: "This is the test sample replying. Send a reference image and it makes a sample design.",
                generated: "Here is a sample image. Approve it to go on to the next stage, or send text to redo it with that as the change.",
                turnaround: "Here is a sample head shell turnaround. Send again for a sample eye close-up.",
                eyes: "Here is a sample eye close-up. Send again for sample order notes.",
                notes: "Here are sample order notes. Send again to redo the turnaround.",
                failed: "That did not work. See the error above.",
            },
        }
    }
}

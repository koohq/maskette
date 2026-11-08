//! Deleting assistant replies and running the last turn again.
//!
//! Timeline events are never removed: a deleted reply is marked `deleted`
//! (and listeners get a patch), so sequence numbers are never reused. The
//! matching assistant message in the transcript is found by its text: the
//! n-th visible reply with a given text is the n-th assistant message with
//! that content.
//!
//! Ported from the old backend's `app/agent/history.py`.

use serde_json::{Value, json};

use super::store::{Conversation, Entry, event_seq, event_type};
use crate::backend::chat_api::JsonObject;

const EDITOR_ACTION_PREFIX: &str = "[editor action]";

fn is_deleted(event: &JsonObject) -> bool {
    event
        .get("deleted")
        .is_some_and(|deleted| deleted.as_bool() != Some(false) && !deleted.is_null())
}

fn text_of(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or_default()
}

fn transcript_index_for_reply(conversation: &Conversation, seq: u64, text: &str) -> Option<usize> {
    let mut occurrence = conversation
        .events
        .iter()
        .filter(|other| {
            event_type(other) == "assistant_message"
                && !is_deleted(other)
                && event_seq(other) < seq
                && text_of(other.get("text")) == text
        })
        .count();
    for (index, message) in conversation.llm_messages.iter().enumerate() {
        if message.get("role").and_then(Value::as_str) == Some("assistant")
            && text_of(message.get("content")) == text
        {
            if occurrence == 0 {
                return Some(index);
            }
            occurrence -= 1;
        }
    }
    None
}

/// Removes an assistant reply from the timeline and from the model's
/// context. `Err` is the `detail` for a 404.
pub fn delete_reply(
    entry: &Entry,
    conversation: &mut Conversation,
    seq: u64,
) -> Result<(), &'static str> {
    let Some(event) = conversation
        .events
        .iter()
        .find(|event| event_seq(event) == seq)
        .filter(|event| event_type(event) == "assistant_message" && !is_deleted(event))
    else {
        return Err("message_not_found");
    };
    let text = text_of(event.get("text")).to_owned();
    if let Some(index) = transcript_index_for_reply(conversation, seq, &text) {
        let message = &mut conversation.llm_messages[index];
        if message.get("tool_calls").is_some_and(|calls| {
            !calls.is_null() && calls.as_array().is_none_or(|calls| !calls.is_empty())
        }) {
            // Tool results answer these calls; dropping the message would
            // orphan them, so only the text goes.
            message["content"] = Value::from("");
        } else {
            conversation.llm_messages.remove(index);
        }
    }
    entry.replace_event(conversation, seq, json!({"deleted": true}));
    Ok(())
}

/// Rewinds the transcript to the last user message and hides that turn's
/// replies; the caller starts the run. `Err` is the `detail` for a 409.
pub fn prepare_regenerate(
    entry: &Entry,
    conversation: &mut Conversation,
) -> Result<(), &'static str> {
    let nothing = "nothing_to_regenerate";
    let user_index = conversation
        .llm_messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .ok_or(nothing)?;
    if text_of(conversation.llm_messages[user_index].get("content"))
        .starts_with(EDITOR_ACTION_PREFIX)
    {
        return Err(nothing);
    }
    let last_user_seq = conversation
        .events
        .iter()
        .filter(|event| event_type(event) == "user_message")
        .map(event_seq)
        .max()
        .ok_or(nothing)?;

    conversation.llm_messages.truncate(user_index + 1);
    // Generated images stay: they are real versions in the project, and the
    // state summary still lists them.
    let hidden: Vec<u64> = conversation
        .events
        .iter()
        .filter(|event| {
            event_seq(event) > last_user_seq
                && matches!(event_type(event), "assistant_message" | "error")
                && !is_deleted(event)
        })
        .map(event_seq)
        .collect();
    for seq in hidden {
        entry.replace_event(conversation, seq, json!({"deleted": true}));
    }
    entry.save(conversation);
    Ok(())
}

//! Prompt overrides: change single prompt texts without rebuilding (the old
//! `app/prompts/overrides.py` and `registry.py`).
//!
//! `prompts.json` in the data folder (the old `runtime/prompts.json`) holds
//! only the keys being changed; every other key keeps the built-in text.
//! Values are strings, or lists of strings for the constraint lists. A key
//! that is missing, empty or of the wrong type falls back to the built-in
//! text, so a broken file never stops generation. The file is read each time
//! a prompt is put together (it is small, and that happens once per image,
//! analysis or agent step), so a saved change applies to the next one.
//!
//! `GET /api/settings/prompts` lists every key Maskette reads, with its
//! built-in text, the text in use and whether the file changes it. The old
//! keys of the paths that have not moved (the finished-product style photo,
//! the landmark lines) are not listed and not read.
//!
//! The old `/api/prompts/requirements` and `/api/prompts/chips` are not
//! served: the screen never calls them.

#[cfg(test)]
mod tests;

use std::path::Path;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Map, Value, json};

use super::SharedState;
use super::agent::runner;
use super::cli::codex;
use super::generation::{analysis, prompt};

pub const FILE_NAME: &str = "prompts.json";

pub fn routes() -> Router<SharedState> {
    Router::new().route("/api/settings/prompts", get(list_prompts))
}

/// The overrides in `prompts.json`, as read at one moment.
#[derive(Debug, Default, Clone)]
pub struct Overrides(Map<String, Value>);

impl Overrides {
    /// Reads `prompts.json` in `data_dir`; nothing when it is missing, and
    /// nothing (with a warning) when it is not a JSON object.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Self(map),
            Ok(_) => {
                log::warn!("ignoring the prompts file: it must be a JSON object");
                Self::default()
            }
            Err(error) => {
                log::warn!("ignoring the prompts file: {error}");
                Self::default()
            }
        }
    }

    #[cfg(test)]
    pub fn from_value(value: Value) -> Self {
        match value {
            Value::Object(map) => Self(map),
            _ => Self::default(),
        }
    }

    /// The text for `key`: the file's when it is a non-blank string.
    pub fn text<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        match self.0.get(key) {
            Some(Value::String(value)) if !value.trim().is_empty() => value,
            _ => default,
        }
    }

    /// The list for `key`: the file's when it is a non-empty list of
    /// non-blank strings.
    pub fn list(&self, key: &str, default: &[&str]) -> Vec<String> {
        if let Some(Value::Array(items)) = self.0.get(key)
            && !items.is_empty()
            && items
                .iter()
                .all(|item| item.as_str().is_some_and(|text| !text.trim().is_empty()))
        {
            return items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect();
        }
        default.iter().map(|item| (*item).to_owned()).collect()
    }

    /// Whether the file names `key` (the old rule: a non-blank string, or
    /// any other value, even one that is not used).
    pub fn is_overridden(&self, key: &str) -> bool {
        match self.0.get(key) {
            Some(Value::String(value)) => !value.trim().is_empty(),
            Some(Value::Null) | None => false,
            Some(_) => true,
        }
    }
}

/// A key's built-in value.
#[derive(Debug, Clone, Copy)]
pub enum Builtin {
    Text(&'static str),
    List(&'static [&'static str]),
}

/// Every key Maskette reads, in the old registry's order.
pub const FIELDS: &[(&str, Builtin)] = &[
    (
        "style_line_replacement.front.none",
        Builtin::Text(prompt::FRONT_NONE_STYLE_LINE),
    ),
    (
        "style_line_replacement.turnaround.none",
        Builtin::Text(prompt::TURNAROUND_NONE_STYLE_LINE),
    ),
    ("head_pose", Builtin::Text(prompt::HEAD_POSE)),
    (
        "eye_symmetry_line",
        Builtin::Text(prompt::EYE_SYMMETRY_LINE),
    ),
    ("watermark_line", Builtin::Text(prompt::WATERMARK_LINE)),
    ("identity_line", Builtin::Text(prompt::IDENTITY_LINE)),
    (
        "hair_fidelity_line",
        Builtin::Text(prompt::HAIR_FIDELITY_LINE),
    ),
    (
        "no_imposed_hairstyle_line",
        Builtin::Text(prompt::NO_IMPOSED_HAIRSTYLE_LINE),
    ),
    ("front_ears_line", Builtin::Text(prompt::FRONT_EARS_LINE)),
    ("shell_ears_line", Builtin::Text(prompt::SHELL_EARS_LINE)),
    (
        "physical_translation_line",
        Builtin::Text(prompt::PHYSICAL_TRANSLATION_LINE),
    ),
    ("anime_face_line", Builtin::Text(prompt::ANIME_FACE_LINE)),
    ("head_shell_look", Builtin::Text(prompt::HEAD_SHELL_LOOK)),
    (
        "head_shell_look_compact",
        Builtin::Text(prompt::HEAD_SHELL_LOOK_COMPACT),
    ),
    (
        "head_shell_presentation",
        Builtin::Text(prompt::HEAD_SHELL_PRESENTATION),
    ),
    (
        "head_shell_presentation_compact",
        Builtin::Text(prompt::HEAD_SHELL_PRESENTATION_COMPACT),
    ),
    (
        "head_pose_compact",
        Builtin::Text(prompt::HEAD_POSE_COMPACT),
    ),
    (
        "four_view_layout_line",
        Builtin::Text(prompt::FOUR_VIEW_LAYOUT_LINE),
    ),
    (
        "four_view_clean_line",
        Builtin::Text(prompt::FOUR_VIEW_CLEAN_LINE),
    ),
    (
        "analysis.detail_prompt",
        Builtin::Text(analysis::DETAIL_ANALYSIS_PROMPT),
    ),
    (
        "analysis.safety_prompt",
        Builtin::Text(analysis::REFERENCE_SAFETY_PROMPT),
    ),
    (
        "constraints.front_design",
        Builtin::List(&prompt::FRONT_DESIGN_CONSTRAINTS),
    ),
    (
        "constraints.front_revision",
        Builtin::List(&prompt::FRONT_REVISION_CONSTRAINTS),
    ),
    (
        "constraints.character_front",
        Builtin::List(&prompt::CHARACTER_FRONT_CONSTRAINTS),
    ),
    (
        "constraints.character_turnaround",
        Builtin::List(&prompt::CHARACTER_TURNAROUND_CONSTRAINTS),
    ),
    (
        "constraints.turnaround",
        Builtin::List(&prompt::TURNAROUND_CONSTRAINTS),
    ),
    ("agent.system_prompt", Builtin::Text(runner::SYSTEM_PROMPT)),
    (
        "codex.tool_requirement",
        Builtin::Text(codex::TOOL_REQUIREMENT),
    ),
    (
        "codex.tool_output_note",
        Builtin::Text(codex::TOOL_OUTPUT_NOTE),
    ),
];

/// Every key with its built-in value, the value in use and whether the file
/// changes it.
pub fn inventory(overrides: &Overrides) -> Value {
    let entries = FIELDS.iter().map(|(key, builtin)| {
        let (kind, default, value) = match builtin {
            Builtin::Text(text) => ("text", json!(text), json!(overrides.text(key, text))),
            Builtin::List(items) => ("list", json!(items), json!(overrides.list(key, items))),
        };
        (
            (*key).to_owned(),
            json!({
                "kind": kind,
                "default": default,
                "value": value,
                "overridden": overrides.is_overridden(key),
            }),
        )
    });
    Value::Object(entries.collect())
}

/// `GET /api/settings/prompts`.
async fn list_prompts(State(state): State<SharedState>) -> Json<Value> {
    Json(inventory(&Overrides::load(&state.data_dir)))
}

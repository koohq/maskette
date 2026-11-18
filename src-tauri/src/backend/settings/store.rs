//! The settings Maskette runs with, and where changes to them are kept.
//!
//! Every editable setting has a built-in default. Changes made on the
//! settings page are saved on top: API keys in the OS credential store
//! (see [`super::secrets`]), everything else in `settings.json` in the data
//! folder. The old backend also layered a `.env` file underneath; the
//! desktop app has none.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

use super::secrets::SecretStore;
use crate::backend::chat_api::{self, JsonObject};

pub const FILE_NAME: &str = "settings.json";
pub const MAX_SETTING_CHARS: usize = 300;

pub struct Field {
    pub key: &'static str,
    pub default: &'static str,
    /// The values a choice may take; `None` for free text.
    pub allowed: Option<&'static [&'static str]>,
    /// Write-only: the API says whether it is set, never its value.
    pub secret: bool,
}

const fn text(key: &'static str, default: &'static str) -> Field {
    Field {
        key,
        default,
        allowed: None,
        secret: false,
    }
}

const fn choice(key: &'static str, allowed: &'static [&'static str]) -> Field {
    Field {
        key,
        default: "",
        allowed: Some(allowed),
        secret: false,
    }
}

const fn secret(key: &'static str) -> Field {
    Field {
        key,
        default: "",
        allowed: None,
        secret: true,
    }
}

const ARK_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/plan/v3";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// The settings the settings page may change, in the old backend's order.
///
/// The defaults put all three roles on OpenRouter, so one key is enough to
/// start: the chat and the analysis on `google/gemini-3.8-flash` (reasoning
/// low and medium), images on GPT Image 2.6 Sunburst. It replaced Nano Banana
/// 2.1 after side-by-side runs (`generation/compare.rs` repeats them): it
/// keeps closer to the drawing (the fringe, the hair's lightness), keeps the
/// pupils round, puts the ears at the drawn height and varies less from one
/// image to the next. Only changed settings are saved, so anyone who never
/// set the image model moves to a new default with the update. The old
/// backend's defaults pointed the chat at Ark's Doubao and left the rest
/// unset.
pub const FIELDS: &[Field] = &[
    Field {
        default: "openai_compatible",
        ..choice("agent_llm_provider", &["openai_compatible", "claude_code"])
    },
    text("agent_llm_base_url", OPENROUTER_BASE_URL),
    text("agent_llm_model", "google/gemini-3.8-flash"),
    text(
        "agent_llm_extra_body",
        r#"{"reasoning": {"effort": "low"}}"#,
    ),
    text("agent_claude_code_model", ""),
    // Empty for the two backend choices means the fixtures in debug builds
    // and OpenRouter (through the chat's endpoint for analysis) otherwise.
    choice(
        "llm_provider",
        &["openai_compatible", "codex", "claude_code", "fixture"],
    ),
    text("analysis_llm_base_url", ""),
    text("analysis_llm_model", ""),
    text(
        "analysis_llm_extra_body",
        r#"{"reasoning": {"effort": "medium"}}"#,
    ),
    choice(
        "image_provider",
        &["codex", "openrouter", "siliconflow", "ark", "fixture"],
    ),
    text("siliconflow_base_url", "https://api.siliconflow.cn/v1"),
    text("siliconflow_image_model", "Qwen/Qwen-Image-Edit-2509"),
    text("ark_base_url", ARK_BASE_URL),
    text("ark_image_model", "doubao-seedream-5-0-pro"),
    text("openrouter_base_url", OPENROUTER_BASE_URL),
    text("openrouter_image_model", "openai/gpt-image-2.6-sunburst"),
    text("openrouter_extra_body", ""),
    secret("agent_llm_api_key"),
    secret("analysis_llm_api_key"),
    secret("siliconflow_api_key"),
    secret("ark_api_key"),
    secret("openrouter_api_key"),
];

/// A saved `image_provider` that is no longer offered: the old Codex Bridge
/// (a server for reaching Codex from inside Docker, running it without a
/// sandbox). It is kept when read, so generation can say what to do instead,
/// but can no longer be chosen. Its `codex_bridge_url` and
/// `codex_bridge_token` are not read.
pub const RETIRED_IMAGE_PROVIDER: &str = "codex_bridge";
/// Earlier builds kept this key in the credential store; a reset removes it.
const RETIRED_SECRET: &str = "codex_bridge_token";

const EXTRA_BODY_FIELDS: [&str; 3] = [
    "agent_llm_extra_body",
    "analysis_llm_extra_body",
    "openrouter_extra_body",
];

pub fn field(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|field| field.key == key)
}

pub fn secret_keys() -> impl Iterator<Item = &'static str> {
    FIELDS
        .iter()
        .filter(|field| field.secret)
        .map(|field| field.key)
}

/// Whether `value` (already trimmed by the caller where it should be) may be
/// saved for `key`. The fixtures are checked separately, since whether they
/// are allowed depends on the build.
pub fn valid_setting(key: &str, value: &str) -> bool {
    let Some(field) = field(key) else {
        return false;
    };
    if value.chars().count() > MAX_SETTING_CHARS {
        return false;
    }
    if field.secret {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
            return false;
        }
    }
    if EXTRA_BODY_FIELDS.contains(&key) && chat_api::parse_extra_body(value).is_err() {
        return false;
    }
    field.allowed.is_none_or(|allowed| allowed.contains(&value))
}

/// The effective settings: defaults with the saved changes on top.
#[derive(Clone)]
pub struct Settings {
    values: BTreeMap<&'static str, String>,
    pub allow_fixture: bool,
}

/// Settings the page does not change (the old backend's `.env`-only ones).
impl Settings {
    pub const CODEX_PATH: &str = "codex";
    pub const CLAUDE_CODE_PATH: &str = "claude";
    pub const AGENT_LLM_TIMEOUT: Duration = Duration::from_secs(120);
    pub const ANALYSIS_LLM_TIMEOUT: Duration = Duration::from_secs(240);
}

impl Settings {
    pub fn defaults(allow_fixture: bool) -> Self {
        Self {
            values: FIELDS
                .iter()
                .map(|field| (field.key, field.default.to_owned()))
                .collect(),
            allow_fixture,
        }
    }

    fn with(mut self, saved: &Saved) -> Self {
        for (key, value) in saved.values.iter().chain(&saved.secrets) {
            if let Some(field) = field(key) {
                self.values.insert(field.key, value.clone());
            }
        }
        self
    }

    /// The value of an editable setting; empty for unknown keys.
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }

    /// The backend for reference analysis: when unset, the fixture in
    /// debug builds and the chat API otherwise.
    pub fn llm_provider(&self) -> &str {
        let fallback = if self.allow_fixture {
            "fixture"
        } else {
            "openai_compatible"
        };
        non_empty_or(self.get("llm_provider").trim(), fallback)
    }

    /// The backend for image generation: when unset, the fixture in debug
    /// builds and OpenRouter otherwise.
    pub fn image_provider(&self) -> &str {
        let fallback = if self.allow_fixture {
            "fixture"
        } else {
            "openrouter"
        };
        non_empty_or(self.get("image_provider").trim(), fallback)
    }

    /// The chat assistant's key; on the Ark endpoint it may reuse the Ark key.
    pub fn agent_api_key(&self) -> String {
        let key = self.get("agent_llm_api_key").trim();
        if !key.is_empty() {
            return key.to_owned();
        }
        if base(self.get("agent_llm_base_url")) == base(self.get("ark_base_url")) {
            return self.get("ark_api_key").trim().to_owned();
        }
        String::new()
    }

    /// Base URL, API key and model for analysis; empty values fall back to
    /// the chat assistant's endpoint.
    pub fn analysis_endpoint(&self) -> (String, String, String) {
        let agent_base = base(self.get("agent_llm_base_url"));
        let base_url = non_empty_or(base(self.get("analysis_llm_base_url")), agent_base);
        let model = non_empty_or(
            self.get("analysis_llm_model").trim(),
            self.get("agent_llm_model").trim(),
        );
        let mut key = self.get("analysis_llm_api_key").trim();
        if key.is_empty() && base_url == agent_base {
            key = self.get("agent_llm_api_key").trim();
        }
        if key.is_empty() && base_url == base(self.get("ark_base_url")) {
            key = self.get("ark_api_key").trim();
        }
        (base_url.to_owned(), key.to_owned(), model.to_owned())
    }

    /// The OpenRouter key, else the key of the analysis or chat endpoint when
    /// that already points at OpenRouter.
    pub fn openrouter_api_key(&self) -> String {
        let key = self.get("openrouter_api_key").trim();
        if !key.is_empty() {
            return key.to_owned();
        }
        let base_url = base(self.get("openrouter_base_url"));
        let (analysis_base, analysis_key, _model) = self.analysis_endpoint();
        if !analysis_key.is_empty() && analysis_base == base_url {
            return analysis_key;
        }
        if base(self.get("agent_llm_base_url")) == base_url {
            return self.get("agent_llm_api_key").trim().to_owned();
        }
        String::new()
    }

    /// The chat assistant's extra request fields; invalid JSON sends none.
    pub fn agent_extra_body(&self) -> JsonObject {
        chat_api::parse_extra_body(self.get("agent_llm_extra_body")).unwrap_or_default()
    }

    /// Extra request fields for analysis. Empty reuses the chat assistant's
    /// fields only while analysis shares its URL: they are vendor specific
    /// (e.g. Doubao's `thinking`) and must not leak to another service.
    /// `{}` sends none even on a shared URL.
    pub fn analysis_extra_body(&self) -> JsonObject {
        let mut raw = self.get("analysis_llm_extra_body");
        let mut name = "analysis_llm_extra_body";
        if raw.trim().is_empty() {
            let agent_base = base(self.get("agent_llm_base_url"));
            if non_empty_or(base(self.get("analysis_llm_base_url")), agent_base) != agent_base {
                return JsonObject::new();
            }
            raw = self.get("agent_llm_extra_body");
            name = "agent_llm_extra_body";
        }
        chat_api::parse_extra_body(raw).unwrap_or_else(|_| {
            log::warn!("ignoring invalid {name} for analysis");
            JsonObject::new()
        })
    }
}

fn base(url: &str) -> &str {
    url.trim().trim_end_matches('/')
}

fn non_empty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// The key is not editable, or the value is not allowed for it.
    Invalid(String),
    /// The file or the credential store could not be written.
    Storage(String),
}

/// What has been changed from the defaults.
#[derive(Clone, Default)]
struct Saved {
    values: BTreeMap<String, String>,
    secrets: BTreeMap<String, String>,
}

pub struct SettingsStore {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    allow_fixture: bool,
    saved: Mutex<Saved>,
}

impl SettingsStore {
    /// Reads what was saved before. Unreadable parts are logged and skipped,
    /// so a broken file or a locked credential store never stops the app.
    pub fn open(data_dir: &Path, secrets: Arc<dyn SecretStore>, allow_fixture: bool) -> Self {
        let path = data_dir.join(FILE_NAME);
        let mut saved = Saved {
            values: read_values(&path),
            secrets: BTreeMap::new(),
        };
        for key in secret_keys() {
            match secrets.get(key) {
                Ok(Some(value)) if valid_setting(key, &value) => {
                    saved
                        .secrets
                        .insert(key.to_owned(), value.trim().to_owned());
                }
                Ok(Some(_)) => log::warn!("ignoring an invalid saved {key}"),
                Ok(None) => {}
                Err(error) => log::warn!("cannot read {key} from the credential store: {error}"),
            }
        }
        Self {
            path,
            secrets,
            allow_fixture,
            saved: Mutex::new(saved),
        }
    }

    fn saved(&self) -> MutexGuard<'_, Saved> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn settings(&self) -> Settings {
        Settings::defaults(self.allow_fixture).with(&self.saved())
    }

    /// The effective settings and the keys changed from the defaults, taken
    /// together so they agree.
    pub fn snapshot(&self) -> (Settings, BTreeSet<String>) {
        let saved = self.saved();
        let keys = saved
            .values
            .keys()
            .chain(saved.secrets.keys())
            .cloned()
            .collect();
        (Settings::defaults(self.allow_fixture).with(&saved), keys)
    }

    /// Applies changes in one go: a `None` value drops the saved change, so
    /// the default applies again. Nothing is saved when any change is
    /// invalid. Blocks on the credential store.
    pub fn update(&self, changes: &[(String, Option<String>)]) -> Result<(), UpdateError> {
        let mut saved = self.saved();
        let mut next = saved.clone();
        for (key, value) in changes {
            let Some(value) = value else {
                next.values.remove(key);
                next.secrets.remove(key);
                continue;
            };
            let value = value.trim();
            if !valid_setting(key, value) || (value == "fixture" && !self.allow_fixture) {
                return Err(UpdateError::Invalid(key.clone()));
            }
            let target = if field(key).is_some_and(|field| field.secret) {
                &mut next.secrets
            } else {
                &mut next.values
            };
            target.insert(key.clone(), value.to_owned());
        }

        // Keep the in-memory copy in step with what actually got stored.
        for key in secret_keys() {
            let wanted = next.secrets.get(key);
            if wanted == saved.secrets.get(key) {
                continue;
            }
            let result = match wanted {
                Some(value) => self.secrets.set(key, value),
                None => self.secrets.delete(key),
            };
            result.map_err(|error| {
                UpdateError::Storage(format!("credential store: {key}: {error}"))
            })?;
            match wanted {
                Some(value) => saved.secrets.insert(key.to_owned(), value.clone()),
                None => saved.secrets.remove(key),
            };
        }
        if next.values != saved.values {
            write_values(&self.path, &next.values).map_err(UpdateError::Storage)?;
            saved.values = next.values;
        }
        Ok(())
    }

    /// Drops every saved change, API keys included. Blocks on the
    /// credential store.
    pub fn reset(&self) -> Result<(), UpdateError> {
        let mut saved = self.saved();
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(UpdateError::Storage(format!(
                    "cannot remove {FILE_NAME}: {error}"
                )));
            }
        }
        saved.values.clear();
        for key in secret_keys() {
            self.secrets.delete(key).map_err(|error| {
                UpdateError::Storage(format!("credential store: {key}: {error}"))
            })?;
            saved.secrets.remove(key);
        }
        if let Err(error) = self.secrets.delete(RETIRED_SECRET) {
            log::warn!("cannot remove {RETIRED_SECRET} from the credential store: {error}");
        }
        Ok(())
    }
}

/// The non-secret changes saved in `settings.json`. API keys are never read
/// from the file, even if someone puts them there.
fn read_values(path: &Path) -> BTreeMap<String, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return BTreeMap::new(),
        Err(error) => {
            log::warn!("ignoring unreadable {}: {error}", path.display());
            return BTreeMap::new();
        }
    };
    let raw = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(raw)) => raw,
        Ok(_) => return BTreeMap::new(),
        Err(error) => {
            log::warn!("ignoring unreadable {}: {error}", path.display());
            return BTreeMap::new();
        }
    };
    raw.into_iter()
        .filter_map(|(key, value)| {
            let Value::String(value) = value else {
                return None;
            };
            let editable = field(&key).is_some_and(|field| !field.secret);
            let retired = key == "image_provider" && value.trim() == RETIRED_IMAGE_PROVIDER;
            (editable && (valid_setting(&key, &value) || retired))
                .then(|| (key, value.trim().to_owned()))
        })
        .collect()
}

/// Writes through a temporary file, so a crash never leaves half a file.
fn write_values(path: &Path, values: &BTreeMap<String, String>) -> Result<(), String> {
    let fail = |error: std::io::Error| format!("cannot write {FILE_NAME}: {error}");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(fail)?;
    }
    let text = serde_json::to_string_pretty(values).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(fail)?;
    std::fs::rename(&temporary, path).map_err(fail)
}

//! The settings Maskette runs with, and where changes to them are kept.
//!
//! Every editable setting has a built-in default. Changes made on the
//! settings page are saved on top, in `settings.json` in the data folder.
//! API keys are kept apart, one per service (see [`service_of`]), in the OS
//! credential store (see [`super::secrets`]): a key belongs to an account at
//! a service, so every role that uses the service uses its key. The old
//! backend also layered a `.env` file underneath; the desktop app has none.

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
}

const fn text(key: &'static str, default: &'static str) -> Field {
    Field {
        key,
        default,
        allowed: None,
    }
}

const fn choice(key: &'static str, allowed: &'static [&'static str]) -> Field {
    Field {
        key,
        default: "",
        allowed: Some(allowed),
    }
}

const ARK_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/plan/v3";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// The `llm_provider` that runs analysis on the chat assistant's backend,
/// with the chat's URL, key and model when that is the OpenAI-compatible API.
pub const SAME_AS_AGENT: &str = "same_as_agent";

/// The analysis settings of its own OpenAI-compatible endpoint. While all of
/// them keep their defaults, an unset `llm_provider` follows the chat
/// assistant; see [`Settings::llm_provider`].
const ANALYSIS_FIELDS: [&str; 3] = [
    "analysis_llm_base_url",
    "analysis_llm_model",
    "analysis_llm_extra_body",
];

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
        ..choice(
            "agent_llm_provider",
            &["openai_compatible", "codex", "claude_code", "fixture"],
        )
    },
    text("agent_llm_base_url", OPENROUTER_BASE_URL),
    text("agent_llm_model", "google/gemini-3.8-flash"),
    text(
        "agent_llm_extra_body",
        r#"{"reasoning": {"effort": "low"}}"#,
    ),
    // Empty for the CLIs means their own default model.
    text("agent_codex_model", ""),
    text("agent_claude_code_model", ""),
    // Empty for the two backend choices means the fixtures in debug builds.
    // Otherwise analysis follows the chat assistant (see `llm_provider()`)
    // and images go to OpenRouter.
    choice(
        "llm_provider",
        &[
            SAME_AS_AGENT,
            "openai_compatible",
            "codex",
            "claude_code",
            "fixture",
        ],
    ),
    text("analysis_llm_base_url", ""),
    text("analysis_llm_model", ""),
    text(
        "analysis_llm_extra_body",
        r#"{"reasoning": {"effort": "medium"}}"#,
    ),
    // The recommended one first, as for the other two roles.
    choice(
        "image_provider",
        &["openrouter", "codex", "siliconflow", "ark", "fixture"],
    ),
    text("siliconflow_base_url", "https://api.siliconflow.cn/v1"),
    text("siliconflow_image_model", "Qwen/Qwen-Image-Edit-2509"),
    text("ark_base_url", ARK_BASE_URL),
    text("ark_image_model", "doubao-seedream-5-0-pro"),
    text("openrouter_base_url", OPENROUTER_BASE_URL),
    text("openrouter_image_model", "openai/gpt-image-2.6-sunburst"),
    text("openrouter_extra_body", ""),
];

/// The fields of the settings page that take an API key, one in each role
/// that uses a service. Each shows the key of the service its role's URL
/// belongs to (see [`Settings::key_url`]), and failures name the field when
/// that key is missing. Earlier builds saved a key per field, under these
/// names; [`SettingsStore::open`] moves them to their services once, in this
/// order, so where two fields of one service held different keys the chat
/// assistant's stays.
pub const KEY_FIELDS: [&str; 5] = [
    "agent_llm_api_key",
    "analysis_llm_api_key",
    "openrouter_api_key",
    "ark_api_key",
    "siliconflow_api_key",
];

/// The credential store cannot list its entries, so this one lists the
/// services that have a key, one per line, for a reset to find them all.
const KEY_INDEX: &str = "api_key_services";

/// The credential store entry that holds the key of `service`.
pub fn key_entry(service: &str) -> String {
    format!("api_key:{service}")
}

/// The service an API URL belongs to, which its key is saved under: the
/// URL's host, with the port unless it is the scheme's default. A key comes
/// from an account at one site, so every OpenRouter URL shares one, while
/// the mainland China and international sites of Ark or SiliconFlow keep one
/// each. A URL that does not parse is a service of its own, and an empty one
/// none. `serviceOf` in the frontend's `settingsApi.ts` follows the same rule.
pub fn service_of(url: &str) -> String {
    let url = base(url);
    if let Ok(parsed) = reqwest::Url::parse(url)
        && let Some(host) = parsed.host_str().filter(|host| !host.is_empty())
    {
        return match parsed.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
    }
    url.to_owned()
}

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

/// Whether `value` is one word of at most [`MAX_SETTING_CHARS`] characters,
/// as API keys and the services they belong to are.
fn one_word(value: &str) -> bool {
    let trimmed = value.trim();
    value.chars().count() <= MAX_SETTING_CHARS
        && !trimmed.is_empty()
        && !trimmed.chars().any(char::is_whitespace)
}

/// Whether `value` may be saved as an API key.
pub fn valid_api_key(value: &str) -> bool {
    one_word(value)
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
    if EXTRA_BODY_FIELDS.contains(&key) && chat_api::parse_extra_body(value).is_err() {
        return false;
    }
    field.allowed.is_none_or(|allowed| allowed.contains(&value))
}

/// The effective settings: defaults with the saved changes on top.
#[derive(Clone)]
pub struct Settings {
    values: BTreeMap<&'static str, String>,
    /// API keys by service.
    keys: BTreeMap<String, String>,
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
            keys: BTreeMap::new(),
            allow_fixture,
        }
    }

    fn with(mut self, saved: &Saved) -> Self {
        for (key, value) in &saved.values {
            if let Some(field) = field(key) {
                self.values.insert(field.key, value.clone());
            }
        }
        self.keys = saved.keys.clone();
        self
    }

    /// The saved API keys, by service.
    pub fn api_keys(&self) -> &BTreeMap<String, String> {
        &self.keys
    }

    /// The key saved for the service `url` belongs to; empty when none is.
    pub fn api_key_for(&self, url: &str) -> String {
        self.keys.get(&service_of(url)).cloned().unwrap_or_default()
    }

    /// The URL whose service the key field `field` (one of [`KEY_FIELDS`])
    /// shows the key of: its role's URL. Analysis with an empty URL of its
    /// own uses the chat assistant's.
    pub fn key_url(&self, field: &str) -> &str {
        let agent = base(self.get("agent_llm_base_url"));
        match field {
            "agent_llm_api_key" => agent,
            "analysis_llm_api_key" => non_empty_or(base(self.get("analysis_llm_base_url")), agent),
            "openrouter_api_key" => base(self.get("openrouter_base_url")),
            "ark_api_key" => base(self.get("ark_base_url")),
            "siliconflow_api_key" => base(self.get("siliconflow_base_url")),
            _ => "",
        }
    }

    /// The value of an editable setting; empty for unknown keys.
    pub fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }

    /// How reference analysis is set up: [`SAME_AS_AGENT`] or a backend of
    /// its own. Unset, it is the fixture in debug builds. Otherwise it
    /// follows the chat assistant, unless an analysis setting was changed:
    /// before analysis could follow the chat, an unset `llm_provider` meant
    /// the OpenAI-compatible API, and those settings keep meaning that.
    pub fn llm_provider(&self) -> &str {
        let saved = self.get("llm_provider").trim();
        if !saved.is_empty() {
            saved
        } else if self.allow_fixture {
            "fixture"
        } else if self.analysis_fields_changed() {
            "openai_compatible"
        } else {
            SAME_AS_AGENT
        }
    }

    fn analysis_fields_changed(&self) -> bool {
        ANALYSIS_FIELDS
            .iter()
            .any(|key| self.get(key).trim() != field(key).map_or("", |field| field.default))
    }

    /// Whether analysis uses the chat assistant's backend and endpoint.
    pub fn analysis_follows_agent(&self) -> bool {
        self.llm_provider() == SAME_AS_AGENT
    }

    /// The backend analysis runs on. Following the chat assistant, it is the
    /// chat's backend: both settings name a backend the same way, so any
    /// chat backend that can also analyse works without a list of its own.
    pub fn analysis_provider(&self) -> &str {
        if self.analysis_follows_agent() {
            self.get("agent_llm_provider").trim()
        } else {
            self.llm_provider()
        }
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

    /// The chat assistant's key: its service's.
    pub fn agent_api_key(&self) -> String {
        self.api_key_for(self.key_url("agent_llm_api_key"))
    }

    /// Base URL, API key and model for analysis: the chat assistant's while
    /// analysis follows it. Otherwise an empty URL or model falls back to the
    /// chat assistant's, and the key is that of the URL's service.
    pub fn analysis_endpoint(&self) -> (String, String, String) {
        let agent_base = base(self.get("agent_llm_base_url"));
        if self.analysis_follows_agent() {
            return (
                agent_base.to_owned(),
                self.agent_api_key(),
                self.get("agent_llm_model").trim().to_owned(),
            );
        }
        let base_url = non_empty_or(base(self.get("analysis_llm_base_url")), agent_base);
        let model = non_empty_or(
            self.get("analysis_llm_model").trim(),
            self.get("agent_llm_model").trim(),
        );
        (
            base_url.to_owned(),
            self.api_key_for(base_url),
            model.to_owned(),
        )
    }

    /// The key for image generation on OpenRouter.
    pub fn openrouter_api_key(&self) -> String {
        self.api_key_for(self.key_url("openrouter_api_key"))
    }

    /// The key for image generation on Ark.
    pub fn ark_api_key(&self) -> String {
        self.api_key_for(self.key_url("ark_api_key"))
    }

    /// The key for image generation on SiliconFlow.
    pub fn siliconflow_api_key(&self) -> String {
        self.api_key_for(self.key_url("siliconflow_api_key"))
    }

    /// The chat assistant's extra request fields; invalid JSON sends none.
    pub fn agent_extra_body(&self) -> JsonObject {
        chat_api::parse_extra_body(self.get("agent_llm_extra_body")).unwrap_or_default()
    }

    /// Extra request fields for analysis. Following the chat assistant, the
    /// built-in ones (more reasoning than the chat's). Otherwise empty reuses
    /// the chat assistant's fields only while analysis shares its URL: they
    /// are vendor specific (e.g. Doubao's `thinking`) and must not leak to
    /// another service. `{}` sends none even on a shared URL.
    pub fn analysis_extra_body(&self) -> JsonObject {
        let mut raw = self.get("analysis_llm_extra_body");
        let mut name = "analysis_llm_extra_body";
        if self.analysis_follows_agent() {
            raw = field(name).map_or("", |field| field.default);
        }
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
    /// The API key for this service is not one word, or the service is not.
    InvalidKey(String),
    /// The file or the credential store could not be written.
    Storage(String),
}

/// What has been changed from the defaults.
#[derive(Clone, Default)]
struct Saved {
    values: BTreeMap<String, String>,
    /// API keys by service.
    keys: BTreeMap<String, String>,
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
            keys: read_keys(secrets.as_ref()),
        };
        let urls = Settings::defaults(allow_fixture).with(&saved);
        move_field_keys(secrets.as_ref(), &urls, &mut saved.keys);
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

    /// The effective settings and the settings changed from the defaults,
    /// taken together so they agree. API keys have no defaults, so they are
    /// not among the changed settings.
    pub fn snapshot(&self) -> (Settings, BTreeSet<String>) {
        let saved = self.saved();
        let changed = saved.values.keys().cloned().collect();
        (Settings::defaults(self.allow_fixture).with(&saved), changed)
    }

    /// Applies changes to the settings, as [`Self::update_with_keys`] does.
    #[cfg(test)]
    pub fn update(&self, changes: &[(String, Option<String>)]) -> Result<(), UpdateError> {
        self.update_with_keys(changes, &[])
    }

    /// Applies changes in one go: to settings, and to API keys by service. A
    /// `None` value drops a saved setting, so the default applies again, or
    /// removes a key. Nothing is saved when any change is invalid. Blocks on
    /// the credential store.
    pub fn update_with_keys(
        &self,
        changes: &[(String, Option<String>)],
        key_changes: &[(String, Option<String>)],
    ) -> Result<(), UpdateError> {
        let mut saved = self.saved();
        let mut next = saved.clone();
        for (key, value) in changes {
            let Some(value) = value else {
                next.values.remove(key);
                continue;
            };
            let value = value.trim();
            if !valid_setting(key, value) || (value == "fixture" && !self.allow_fixture) {
                return Err(UpdateError::Invalid(key.clone()));
            }
            next.values.insert(key.clone(), value.to_owned());
        }
        for (service, value) in key_changes {
            if !one_word(service) || service.trim() != service {
                return Err(UpdateError::InvalidKey(service.clone()));
            }
            match value {
                None => {
                    next.keys.remove(service);
                }
                Some(value) if valid_api_key(value) => {
                    next.keys.insert(service.clone(), value.trim().to_owned());
                }
                Some(_) => return Err(UpdateError::InvalidKey(service.clone())),
            }
        }

        self.store_keys(&mut saved, &next.keys)?;
        if next.values != saved.values {
            write_values(&self.path, &next.values).map_err(UpdateError::Storage)?;
            saved.values = next.values;
        }
        Ok(())
    }

    /// Saves changes the way the settings page does, with each key given
    /// under its field in [`KEY_FIELDS`]: it goes to the service of that
    /// field's URL once the other changes are made.
    #[cfg(test)]
    pub fn update_with_field_keys(
        &self,
        changes: &[(String, Option<String>)],
    ) -> Result<(), UpdateError> {
        let (keys, values): (Vec<_>, Vec<_>) = changes
            .iter()
            .cloned()
            .partition(|(key, _)| KEY_FIELDS.contains(&key.as_str()));
        let mut urls = self.settings();
        for (key, value) in &values {
            if let Some(field) = field(key) {
                let value = value.as_deref().unwrap_or(field.default);
                urls.values.insert(field.key, value.trim().to_owned());
            }
        }
        let keys: Vec<_> = keys
            .into_iter()
            .map(|(field, value)| (service_of(urls.key_url(&field)), value))
            .collect();
        self.update_with_keys(&values, &keys)
    }

    /// Stores the keys in `wanted` that differ from the saved ones and
    /// removes the rest, keeping the in-memory copy in step with what
    /// actually got stored. A service is listed in [`KEY_INDEX`] before its
    /// key is stored and unlisted after its key is gone: a listed service
    /// with no key is harmless, while a key no entry lists would outlive a
    /// reset.
    fn store_keys(
        &self,
        saved: &mut Saved,
        wanted: &BTreeMap<String, String>,
    ) -> Result<(), UpdateError> {
        let failed = |what: &str, error: String| {
            UpdateError::Storage(format!("credential store: {what}: {error}"))
        };
        if wanted
            .keys()
            .any(|service| !saved.keys.contains_key(service))
        {
            write_index(
                self.secrets.as_ref(),
                saved.keys.keys().chain(wanted.keys()),
            )
            .map_err(|error| failed(KEY_INDEX, error))?;
        }
        for (service, value) in wanted {
            if saved.keys.get(service) != Some(value) {
                self.secrets
                    .set(&key_entry(service), value)
                    .map_err(|error| failed(service, error))?;
                saved.keys.insert(service.clone(), value.clone());
            }
        }
        let removed: Vec<String> = saved
            .keys
            .keys()
            .filter(|service| !wanted.contains_key(*service))
            .cloned()
            .collect();
        for service in &removed {
            self.secrets
                .delete(&key_entry(service))
                .map_err(|error| failed(service, error))?;
            saved.keys.remove(service);
        }
        if !removed.is_empty() {
            write_index(self.secrets.as_ref(), saved.keys.keys())
                .map_err(|error| failed(KEY_INDEX, error))?;
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
        let failed = |what: &str, error: String| {
            UpdateError::Storage(format!("credential store: {what}: {error}"))
        };
        // Also the services listed with a key that could not be read.
        let mut services: BTreeSet<String> = saved.keys.keys().cloned().collect();
        services
            .extend(read_index(self.secrets.as_ref()).map_err(|error| failed(KEY_INDEX, error))?);
        for service in &services {
            self.secrets
                .delete(&key_entry(service))
                .map_err(|error| failed(service, error))?;
            saved.keys.remove(service);
        }
        self.secrets
            .delete(KEY_INDEX)
            .map_err(|error| failed(KEY_INDEX, error))?;
        for name in KEY_FIELDS.into_iter().chain([RETIRED_SECRET]) {
            if let Err(error) = self.secrets.delete(name) {
                log::warn!("cannot remove {name} from the credential store: {error}");
            }
        }
        Ok(())
    }
}

/// The services [`KEY_INDEX`] lists.
fn read_index(secrets: &dyn SecretStore) -> Result<Vec<String>, String> {
    Ok(secrets
        .get(KEY_INDEX)?
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|service| !service.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// Lists `services` in [`KEY_INDEX`]; none removes the entry.
fn write_index<'a>(
    secrets: &dyn SecretStore,
    services: impl Iterator<Item = &'a String>,
) -> Result<(), String> {
    let services: BTreeSet<&str> = services.map(String::as_str).collect();
    if services.is_empty() {
        return secrets.delete(KEY_INDEX);
    }
    secrets.set(
        KEY_INDEX,
        &services.into_iter().collect::<Vec<_>>().join("\n"),
    )
}

/// The saved API keys, by service.
fn read_keys(secrets: &dyn SecretStore) -> BTreeMap<String, String> {
    let services = match read_index(secrets) {
        Ok(services) => services,
        Err(error) => {
            log::warn!("cannot read {KEY_INDEX} from the credential store: {error}");
            return BTreeMap::new();
        }
    };
    let mut keys = BTreeMap::new();
    for service in services {
        match secrets.get(&key_entry(&service)) {
            Ok(Some(value)) if valid_api_key(&value) => {
                keys.insert(service, value.trim().to_owned());
            }
            Ok(Some(_)) => log::warn!("ignoring an invalid saved key for {service}"),
            Ok(None) => {}
            Err(error) => {
                log::warn!("cannot read the key for {service} from the credential store: {error}");
            }
        }
    }
    keys
}

/// Moves the keys earlier builds saved per field ([`KEY_FIELDS`]) to the
/// services of the fields' URLs, adding to `keys`. A service that already
/// has a key keeps it. The old entries go only once the moved keys are
/// stored, so a locked credential store tries again on the next start.
fn move_field_keys(
    secrets: &dyn SecretStore,
    urls: &Settings,
    keys: &mut BTreeMap<String, String>,
) {
    let mut found = Vec::new();
    for name in KEY_FIELDS {
        match secrets.get(name) {
            Ok(Some(value)) => found.push((name, value)),
            Ok(None) => {}
            Err(error) => {
                log::warn!("cannot read {name} from the credential store: {error}");
                return;
            }
        }
    }
    if found.is_empty() {
        return;
    }
    let mut moved = BTreeMap::new();
    for (name, value) in found {
        let value = value.trim();
        let service = service_of(urls.key_url(name));
        if !valid_api_key(value) || service.is_empty() {
            log::warn!("dropping an invalid saved {name}");
            continue;
        }
        match keys.get(&service).or_else(|| moved.get(&service)) {
            None => {
                moved.insert(service, value.to_owned());
            }
            Some(kept) if kept != value => {
                log::warn!("dropping the saved {name}: {service} already has another key");
            }
            Some(_) => {}
        }
    }
    let stored = store_moved_keys(secrets, keys, &moved);
    keys.extend(moved);
    if let Err(error) = stored {
        log::warn!("cannot move the saved API keys to their services: {error}");
        return;
    }
    for name in KEY_FIELDS {
        if let Err(error) = secrets.delete(name) {
            log::warn!("cannot remove {name} from the credential store: {error}");
        }
    }
}

fn store_moved_keys(
    secrets: &dyn SecretStore,
    kept: &BTreeMap<String, String>,
    moved: &BTreeMap<String, String>,
) -> Result<(), String> {
    if moved.is_empty() {
        return Ok(());
    }
    write_index(secrets, kept.keys().chain(moved.keys()))?;
    for (service, value) in moved {
        secrets.set(&key_entry(service), value)?;
    }
    Ok(())
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
            let editable = field(&key).is_some();
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

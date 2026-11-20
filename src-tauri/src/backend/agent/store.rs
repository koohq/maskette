//! Conversations: one JSON file each under `agent/` in the data folder, kept
//! in memory once read, plus a per-conversation signal for live events.
//!
//! The JSON shape is the old backend's (`app/agent/store.py`), so Visual
//! Studio Kigurumi's `.vkp` project files open here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::backend::chat_api::JsonObject;
use crate::backend::ids;

pub const DIR_NAME: &str = "agent";
pub const DEFAULT_LOCALE: &str = "zh-CN";
const SUPPORTED_LOCALES: [&str; 3] = ["zh-CN", "en", "ja"];

/// The old `normalize_locale`: a supported locale, else the default.
pub fn normalize_locale(locale: &str) -> &'static str {
    SUPPORTED_LOCALES
        .into_iter()
        .find(|known| *known == locale)
        .unwrap_or(DEFAULT_LOCALE)
}

/// `null` reads as the default, as `data.get(key) or default` did.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn default_locale() -> String {
    DEFAULT_LOCALE.to_owned()
}

fn locale_or_default<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let locale: Option<String> = Option::deserialize(deserializer)?;
    Ok(locale
        .filter(|locale| !locale.is_empty())
        .unwrap_or_else(default_locale))
}

fn default_status() -> String {
    "idle".to_owned()
}

fn status_or_idle<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let status: Option<String> = Option::deserialize(deserializer)?;
    Ok(status
        .filter(|status| !status.is_empty())
        .unwrap_or_else(default_status))
}

fn default_source() -> String {
    "agent".to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceImage {
    pub id: String,
    /// The object key (`references/...`).
    pub key: String,
    /// What the image shows, one of [`REFERENCE_TAGS`]: `front` for the one
    /// main character reference; uploads after it start as `supplemental`.
    pub kind: String,
    pub url: String,
    pub file_name: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignImage {
    pub id: String,
    /// Stage 1: `design` | `design_turnaround`; stage 2 (head shell):
    /// `front` | `turnaround`.
    pub role: String,
    #[serde(default)]
    pub job_id: String,
    /// The public (watermarked) image.
    pub url: String,
    /// The clean pixels, as an object key under `references/`.
    pub reference_key: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub created_seq: u64,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// `agent` | `manual` | `local_revision`.
    #[serde(default = "default_source")]
    pub source: String,
    /// The editor recipe of a manual version, so it can be edited again.
    #[serde(default)]
    pub recipe: Option<Value>,
}

pub const DESIGN_ROLES: [&str; 2] = ["design", "design_turnaround"];

/// The tags a user can give a reference image: what it shows.
pub const REFERENCE_TAGS: [&str; 6] = [
    "front",
    "side",
    "back",
    "expression",
    "accessory",
    "supplemental",
];

/// The views of the head that, without a reference, the model can only
/// guess: the sides and back of a four-view sheet.
const GUESSED_VIEWS: [&str; 2] = ["side", "back"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentState {
    pub character_session_id: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub references: Vec<ReferenceImage>,
    #[serde(default)]
    pub analysis: Option<Value>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub images: Vec<DesignImage>,
    #[serde(default)]
    pub current_design_id: Option<String>,
    #[serde(default)]
    pub approved_design_id: Option<String>,
    #[serde(default)]
    pub current_front_id: Option<String>,
    #[serde(default)]
    pub approved_front_id: Option<String>,
    /// The newest order notes: the words that go to the mask maker with
    /// the approved front view, as the `order_notes` event shows them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_notes: Option<Value>,
    #[serde(default)]
    pub last_user_seq: u64,
}

impl AgentState {
    fn new(character_session_id: &str) -> Self {
        Self {
            character_session_id: character_session_id.to_owned(),
            references: Vec::new(),
            analysis: None,
            images: Vec::new(),
            current_design_id: None,
            approved_design_id: None,
            current_front_id: None,
            approved_front_id: None,
            order_notes: None,
            last_user_seq: 0,
        }
    }

    pub fn image(&self, image_id: Option<&str>) -> Option<&DesignImage> {
        let image_id = image_id?;
        self.images.iter().find(|image| image.id == image_id)
    }

    /// The views no reference is tagged with, so generation will guess them.
    /// Empty before the first upload: there is nothing to warn about yet.
    pub fn missing_views(&self) -> Vec<&'static str> {
        if self.references.is_empty() {
            return Vec::new();
        }
        GUESSED_VIEWS
            .into_iter()
            .filter(|view| {
                !self
                    .references
                    .iter()
                    .any(|reference| reference.kind == *view)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub created_at: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub title: String,
    #[serde(default = "default_locale", deserialize_with = "locale_or_default")]
    pub locale: String,
    /// `idle` | `running` | `error`.
    #[serde(default = "default_status", deserialize_with = "status_or_idle")]
    pub status: String,
    pub state: AgentState,
    /// The model's transcript, in the OpenAI chat format.
    #[serde(default, deserialize_with = "null_as_default")]
    pub llm_messages: Vec<Value>,
    /// The timeline the UI shows; every event has a `seq` and a `type`.
    #[serde(default, deserialize_with = "null_as_default")]
    pub events: Vec<JsonObject>,
}

pub fn event_seq(event: &JsonObject) -> u64 {
    event.get("seq").and_then(Value::as_u64).unwrap_or(0)
}

pub fn event_type(event: &JsonObject) -> &str {
    event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

impl Conversation {
    fn new(id: &str, locale: &str) -> Self {
        Self {
            id: id.to_owned(),
            created_at: ids::now_iso(),
            title: String::new(),
            locale: locale.to_owned(),
            status: default_status(),
            state: AgentState::new(id),
            llm_messages: Vec::new(),
            events: Vec::new(),
        }
    }

    pub fn last_seq(&self) -> u64 {
        self.events.last().map_or(0, event_seq)
    }

    pub fn next_seq(&self) -> u64 {
        self.last_seq() + 1
    }
}

/// One conversation in memory. Lock it briefly: never across an `await`.
pub struct Entry {
    path: PathBuf,
    conversation: Mutex<Conversation>,
    /// The last event's `seq`, for live listeners.
    seq: watch::Sender<u64>,
}

impl Entry {
    fn new(path: PathBuf, conversation: Conversation) -> Self {
        let (seq, _) = watch::channel(conversation.last_seq());
        Self {
            path,
            conversation: Mutex::new(conversation),
            seq,
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Conversation> {
        self.conversation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.seq.subscribe()
    }

    /// Writes the conversation through a temporary file. A failure is
    /// logged: the conversation stays usable, only the copy on disk lags.
    pub fn save(&self, conversation: &Conversation) {
        if let Err(error) = write_json(&self.path, conversation) {
            log::error!("saving conversation {} failed: {error}", conversation.id);
        }
    }

    /// Appends an event, saves, and wakes the listeners.
    pub fn emit(&self, conversation: &mut Conversation, kind: &str, data: Value) -> JsonObject {
        let mut event = JsonObject::new();
        event.insert("seq".to_owned(), Value::from(conversation.next_seq()));
        event.insert("type".to_owned(), Value::from(kind));
        event.insert("created_at".to_owned(), Value::from(ids::now_iso()));
        if let Value::Object(data) = data {
            event.extend(data);
        }
        conversation.events.push(event.clone());
        self.save(conversation);
        self.seq.send_replace(conversation.last_seq());
        event
    }

    /// Updates an event in place (tool progress, a deleted reply) and tells
    /// listeners with an `event_patch` event.
    pub fn replace_event(&self, conversation: &mut Conversation, seq: u64, patch: Value) {
        let Value::Object(fields) = &patch else {
            return;
        };
        if let Some(event) = conversation
            .events
            .iter_mut()
            .find(|event| event_seq(event) == seq)
        {
            event.extend(fields.clone());
        }
        self.emit(
            conversation,
            "event_patch",
            serde_json::json!({"target_seq": seq, "patch": patch}),
        );
    }
}

fn write_json(path: &Path, conversation: &Conversation) -> Result<(), String> {
    let text = serde_json::to_string(conversation).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) || byte == b'-')
}

pub struct ConversationStore {
    root: PathBuf,
    cache: Mutex<HashMap<String, Arc<Entry>>>,
}

impl ConversationStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join(DIR_NAME),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn cache(&self) -> MutexGuard<'_, HashMap<String, Arc<Entry>>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    /// A folder of the conversation's own (staging, manual edits).
    pub fn workspace(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn create(&self, locale: &str) -> Arc<Entry> {
        let id = ids::uuid4();
        self.insert(Conversation::new(&id, locale))
    }

    /// Takes in a whole conversation (an imported project) and saves it.
    pub fn insert(&self, conversation: Conversation) -> Arc<Entry> {
        let entry = Arc::new(Entry::new(self.path(&conversation.id), conversation));
        entry.save(&entry.lock());
        let id = entry.lock().id.clone();
        self.cache().insert(id, entry.clone());
        entry
    }

    pub fn get(&self, id: &str) -> Option<Arc<Entry>> {
        if !valid_id(id) {
            return None;
        }
        let mut cache = self.cache();
        if let Some(entry) = cache.get(id) {
            return Some(entry.clone());
        }
        let path = self.path(id);
        let text = std::fs::read_to_string(&path).ok()?;
        let mut conversation: Conversation = match serde_json::from_str(&text) {
            Ok(conversation) => conversation,
            Err(error) => {
                log::warn!("ignoring unreadable {}: {error}", path.display());
                return None;
            }
        };
        if conversation.status == "running" {
            // The app stopped mid-run; that run is gone.
            conversation.status = default_status();
        }
        let entry = Arc::new(Entry::new(path, conversation));
        cache.insert(id.to_owned(), entry.clone());
        Some(entry)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Arc<Entry>> {
        let Ok(files) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut entries: Vec<(String, Arc<Entry>)> = files
            .filter_map(Result::ok)
            .filter_map(|file| {
                let name = file.file_name().into_string().ok()?;
                let entry = self.get(name.strip_suffix(".json")?)?;
                let created_at = entry.lock().created_at.clone();
                Some((created_at, entry))
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        entries.into_iter().map(|(_, entry)| entry).collect()
    }

    /// Forgets the conversation and removes its file. Returns whether there
    /// was a file.
    pub fn delete(&self, id: &str) -> bool {
        self.cache().remove(id);
        let removed = std::fs::remove_file(self.path(id)).is_ok();
        if let Err(error) = std::fs::remove_dir_all(self.workspace(id))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("cannot remove the workspace of conversation {id}: {error}");
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversations_round_trip_through_the_old_json_shape() {
        let old = serde_json::json!({
            "id": "0f0e0d0c-0b0a-4908-8706-050403020100",
            "created_at": "2026-01-02T03:04:05.000006+00:00",
            "title": null,
            "locale": null,
            "status": "running",
            "state": {
                "character_session_id": "0f0e0d0c-0b0a-4908-8706-050403020100",
                "references": [{"id": "ref-1-abcdef", "key": "references/agent/x/uploads/a.png", "kind": "front", "url": "/api/references/references/agent/x/uploads/a.png", "file_name": "a.png", "note": ""}],
                "analysis": null,
                "images": [{"id": "front-1", "role": "front", "job_id": "j", "url": "/api/generated/s/a.png", "reference_key": "references/agent/x/front-1.png", "width": 800, "height": 1100, "instructions": "", "created_seq": 3, "parent_id": null, "source": "agent", "recipe": null}],
                "current_design_id": null,
                "approved_design_id": null,
                "current_front_id": "front-1",
                "approved_front_id": null,
                "last_user_seq": 1
            },
            "llm_messages": [{"role": "user", "content": "hi"}],
            "events": [{"seq": 1, "type": "user_message", "created_at": "x", "text": "hi"}]
        });
        let conversation: Conversation = serde_json::from_value(old).unwrap();
        assert_eq!(conversation.title, "");
        assert_eq!(conversation.locale, "zh-CN");
        assert_eq!(conversation.last_seq(), 1);
        assert_eq!(
            conversation.state.image(Some("front-1")).unwrap().width,
            800
        );
        let again: Conversation =
            serde_json::from_value(serde_json::to_value(&conversation).unwrap()).unwrap();
        assert_eq!(again, conversation);
    }

    #[test]
    fn the_store_reloads_and_forgets_interrupted_runs() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConversationStore::new(dir.path());
        let entry = store.create("ja");
        let id = {
            let mut conversation = entry.lock();
            conversation.status = "running".to_owned();
            entry.emit(
                &mut conversation,
                "run_state",
                serde_json::json!({"status": "running"}),
            );
            conversation.id.clone()
        };
        let reopened = ConversationStore::new(dir.path());
        let entry = reopened.get(&id).unwrap();
        assert_eq!(entry.lock().status, "idle");
        assert_eq!(entry.lock().events.len(), 1);
        assert_eq!(reopened.list().len(), 1);
        assert!(reopened.get("not-an-id").is_none());
        assert!(reopened.delete(&id));
        assert!(reopened.get(&id).is_none());
    }
}

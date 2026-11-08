//! The design assistant: projects are conversations with a model that calls
//! tools to analyse references and generate designs.
//!
//! Ported from the old backend's `app/agent/`. Conversations live in `agent/`
//! under the data folder (the old `runtime/agent`), their uploads and version
//! pixels in `references/agent/<id>/`.
//!
//! Intended differences from the old backend:
//! - Generation runs in the same process instead of a queue the tools poll
//!   (see [`crate::backend::generation`]); a cancelled run stops its request.
//! - The Claude Code chat assistant moves with the CLI paths.
//! - Deleting a project also removes its images and working files.
//! - The system prompt calls the assistant Maskette's; it cannot be
//!   overridden yet (prompt overrides move with the prompts router).

pub mod history;
pub mod llm;
pub mod runner;
pub mod store;
pub mod tools;
pub mod vkp;

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::multipart::{Multipart, MultipartRejection};
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Path as UrlPath, Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use self::runner::{AgentRunner, DirectAction};
use self::store::{Conversation, ConversationStore, Entry, ReferenceImage, event_seq, event_type};
use self::tools::{LocalRevision, NewVersion};
use super::SharedState;
use super::error::ApiError;
use super::generation;
use super::ids;
use super::references;
use super::safety::{sanitize_user_text, take_chars};

const MAX_MESSAGE_CHARS: usize = 2000;
const MAX_FILES_PER_MESSAGE: usize = 6;
const MAX_REFERENCES_PER_CONVERSATION: usize = 8;
const MAX_VERSION_BYTES: usize = 20 * 1024 * 1024;
const MAX_RECIPE_CHARS: usize = 200_000;
const MAX_TITLE_CHARS: usize = 80;
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);
/// A message with six reference images, or one version upload.
const MAX_FORM_BYTES: usize = 6 * references::MAX_UPLOAD_BYTES;

/// The conversations and the runs working on them.
pub struct Agent {
    pub store: ConversationStore,
    pub runner: AgentRunner,
}

impl Agent {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            store: ConversationStore::new(data_dir),
            runner: AgentRunner::default(),
        }
    }
}

pub fn routes() -> Router<SharedState> {
    let form_limit = DefaultBodyLimit::max(MAX_FORM_BYTES);
    Router::new()
        .route(
            "/api/agent/conversations",
            get(list_conversations).post(create_conversation),
        )
        .route(
            "/api/agent/conversations/import",
            post(import_project).layer(DefaultBodyLimit::max(vkp::MAX_IMPORT_BYTES + 1024 * 1024)),
        )
        .route(
            "/api/agent/conversations/{id}",
            get(get_conversation)
                .patch(update_conversation)
                .delete(delete_conversation),
        )
        .route(
            "/api/agent/conversations/{id}/messages",
            post(post_message).layer(form_limit),
        )
        .route(
            "/api/agent/conversations/{id}/messages/{seq}",
            delete(delete_message),
        )
        .route("/api/agent/conversations/{id}/regenerate", post(regenerate))
        .route("/api/agent/conversations/{id}/cancel", post(cancel_run))
        .route("/api/agent/conversations/{id}/events", get(stream_events))
        .route(
            "/api/agent/conversations/{id}/references",
            post(upload_references).layer(form_limit),
        )
        .route(
            "/api/agent/conversations/{id}/versions",
            post(save_manual_version).layer(form_limit),
        )
        .route(
            "/api/agent/conversations/{id}/versions/{image_id}/source",
            get(get_version_source),
        )
        .route(
            "/api/agent/conversations/{id}/versions/{image_id}/download",
            get(download_version),
        )
        .route("/api/agent/conversations/{id}/export", get(export_project))
        .route(
            "/api/agent/conversations/{id}/local-revision",
            post(start_local_revision).layer(form_limit),
        )
        .route(
            "/api/agent/conversations/{id}/annotated-revision",
            post(start_annotated_revision).layer(form_limit),
        )
}

// --- Shapes -----------------------------------------------------------------

#[derive(Serialize)]
struct ConversationSummary {
    id: String,
    title: String,
    created_at: String,
    status: String,
    thumbnail_url: Option<String>,
    version_count: usize,
}

fn summary(conversation: &Conversation) -> ConversationSummary {
    let state = &conversation.state;
    let thumbnail_url = state
        .images
        .last()
        .map(|image| image.url.clone())
        .or_else(|| {
            state
                .references
                .first()
                .map(|reference| reference.url.clone())
        });
    ConversationSummary {
        id: conversation.id.clone(),
        title: conversation.title.clone(),
        created_at: conversation.created_at.clone(),
        status: conversation.status.clone(),
        thumbnail_url,
        version_count: state.images.len(),
    }
}

fn out(state: &SharedState, conversation: &Conversation) -> Json<Value> {
    let events: Vec<_> = conversation
        .events
        .iter()
        .filter(|event| event_type(event) != "event_patch")
        .collect();
    let agent_state = &conversation.state;
    Json(json!({
        "id": conversation.id,
        "title": conversation.title,
        "created_at": conversation.created_at,
        "locale": conversation.locale,
        "status": conversation.status,
        "running": state.agent.runner.is_running(&conversation.id),
        "last_seq": conversation.last_seq(),
        "events": events,
        "state": {
            "references": agent_state.references,
            "images": agent_state.images,
            "current_design_id": agent_state.current_design_id,
            "approved_design_id": agent_state.approved_design_id,
            "current_front_id": agent_state.current_front_id,
            "approved_front_id": agent_state.approved_front_id,
            "analysis": agent_state.analysis,
        },
    }))
}

fn require(state: &SharedState, id: &str) -> Result<Arc<Entry>, ApiError> {
    state
        .agent
        .store
        .get(id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "conversation_not_found"))
}

fn conflict(detail: &str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, detail)
}

fn ensure_idle(state: &SharedState, id: &str) -> Result<(), ApiError> {
    if state.agent.runner.is_running(id) {
        Err(conflict("conversation_running"))
    } else {
        Ok(())
    }
}

fn unprocessable(detail: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
}

/// Runs blocking work (image coding, zips, many files) off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| ApiError::internal(format!("background work failed: {error}")))
}

// --- Forms --------------------------------------------------------------------

struct Upload {
    file_name: String,
    content_type: Option<String>,
    bytes: Bytes,
}

/// A multipart form: text fields (a repeated one keeps its last value) and
/// files by field name.
#[derive(Default)]
struct Form {
    texts: HashMap<String, String>,
    files: HashMap<String, Vec<Upload>>,
}

impl Form {
    async fn read(form: Result<Multipart, MultipartRejection>) -> Result<Self, ApiError> {
        let mut form = form.map_err(|rejection| unprocessable(rejection.body_text()))?;
        let read_error = |error: axum::extract::multipart::MultipartError| {
            ApiError::new(error.status(), error.body_text())
        };
        let mut parsed = Self::default();
        while let Some(field) = form.next_field().await.map_err(read_error)? {
            let Some(name) = field.name().map(str::to_owned) else {
                continue;
            };
            match field.file_name().map(str::to_owned) {
                Some(file_name) => {
                    let content_type = field.content_type().map(str::to_owned);
                    let bytes = field.bytes().await.map_err(read_error)?;
                    parsed.files.entry(name).or_default().push(Upload {
                        file_name,
                        content_type,
                        bytes,
                    });
                }
                None => {
                    let text = field.text().await.map_err(read_error)?;
                    parsed.texts.insert(name, text);
                }
            }
        }
        Ok(parsed)
    }

    fn text(&self, name: &str) -> String {
        self.texts.get(name).cloned().unwrap_or_default()
    }

    fn required_text(&self, name: &str) -> Result<String, ApiError> {
        self.texts
            .get(name)
            .cloned()
            .ok_or_else(|| unprocessable(format!("{name}: field required")))
    }

    fn files(&mut self, name: &str) -> Vec<Upload> {
        self.files.remove(name).unwrap_or_default()
    }

    /// One uploaded file; an empty file input (no name) counts as none.
    fn file(&mut self, name: &str) -> Option<Upload> {
        self.files(name)
            .into_iter()
            .next_back()
            .filter(|upload| !upload.file_name.is_empty())
    }

    fn required_file(&mut self, name: &str) -> Result<Upload, ApiError> {
        self.files(name)
            .into_iter()
            .next_back()
            .ok_or_else(|| unprocessable(format!("{name}: field required")))
    }

    /// A boolean field the way the old form parsing read it.
    fn flag(&self, name: &str) -> Result<bool, ApiError> {
        let Some(value) = self.texts.get(name) else {
            return Ok(false);
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
            "0" | "false" | "f" | "no" | "n" | "off" => Ok(false),
            _ => Err(unprocessable(format!("{name}: not a boolean"))),
        }
    }
}

fn validate_uploads(uploads: &[Upload]) -> Result<(), ApiError> {
    for upload in uploads {
        let file_name = references::safe_filename(Some(&upload.file_name))?;
        references::validate_image_upload(&file_name, upload.content_type.as_deref())?;
    }
    Ok(())
}

/// Stores an upload as a reference of the conversation. Only the first
/// image ever uploaded becomes the main front reference.
fn save_reference(
    data_dir: &Path,
    conversation: &Conversation,
    upload: &Upload,
) -> Result<ReferenceImage, ApiError> {
    let file_name = references::safe_filename(Some(&upload.file_name))?;
    references::validate_image_upload(&file_name, upload.content_type.as_deref())?;
    let state = &conversation.state;
    let reference_id = format!("ref-{}-{}", state.references.len() + 1, ids::short_hex(6));
    let relative = format!(
        "agent/{}/uploads/{reference_id}-{file_name}",
        conversation.id
    );
    let destination = references::root(data_dir).join(&relative);
    let write = || -> std::io::Result<()> {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&destination, &upload.bytes)
    };
    write().map_err(|error| ApiError::internal(format!("reference_not_saved: {error}")))?;
    let has_front = state
        .references
        .iter()
        .any(|reference| reference.kind == "front");
    Ok(ReferenceImage {
        id: reference_id,
        key: format!("references/{relative}"),
        kind: if has_front { "supplemental" } else { "front" }.to_owned(),
        url: references::asset_url(&format!("references/{relative}")),
        file_name,
        note: String::new(),
    })
}

fn attachments_json(attachments: &[ReferenceImage]) -> Value {
    attachments
        .iter()
        .map(|item| json!({"id": item.id, "url": item.url, "kind": item.kind}))
        .collect()
}

fn llm_user_content(
    text: &str,
    attachments: &[ReferenceImage],
    action: &str,
    image_id: &str,
) -> String {
    let mut parts = Vec::new();
    if !attachments.is_empty() {
        let listed: Vec<String> = attachments
            .iter()
            .map(|item| format!("{} ({})", item.id, item.kind))
            .collect();
        parts.push(format!(
            "[uploaded reference images: {}]",
            listed.join(", ")
        ));
    }
    match action {
        "approve_front" => parts.push(format!(
            "[clicked approve on head shell front view {image_id}; it is now approved]"
        )),
        "approve_design" => parts.push(format!(
            "[clicked approve on character design {image_id}; it is now approved]"
        )),
        _ => {}
    }
    if !text.is_empty() {
        parts.push(text.to_owned());
    }
    parts.join("\n")
}

fn clean_title(title: &str) -> String {
    take_chars(&sanitize_user_text(title.trim()), MAX_TITLE_CHARS)
}

// --- Conversations ------------------------------------------------------------

async fn list_conversations(State(state): State<SharedState>) -> Json<Vec<ConversationSummary>> {
    let store_state = state.clone();
    let entries = blocking(move || store_state.agent.store.list())
        .await
        .unwrap_or_default();
    Json(entries.iter().map(|entry| summary(&entry.lock())).collect())
}

#[derive(Deserialize)]
struct CreateConversation {
    #[serde(default = "default_locale")]
    locale: String,
    #[serde(default)]
    title: String,
}

fn default_locale() -> String {
    store::DEFAULT_LOCALE.to_owned()
}

async fn create_conversation(
    State(state): State<SharedState>,
    payload: Result<Json<CreateConversation>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let Json(payload) = payload?;
    if payload.locale.chars().count() > 16 {
        return Err(unprocessable("locale: at most 16 characters"));
    }
    if payload.title.chars().count() > MAX_TITLE_CHARS {
        return Err(unprocessable("title: at most 80 characters"));
    }
    let entry = state
        .agent
        .store
        .create(store::normalize_locale(&payload.locale));
    let mut conversation = entry.lock();
    let title = clean_title(&payload.title);
    if !title.is_empty() {
        conversation.title = title;
        entry.save(&conversation);
    }
    Ok(out(&state, &conversation))
}

#[derive(Deserialize)]
struct UpdateConversation {
    title: String,
}

async fn update_conversation(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    payload: Result<Json<UpdateConversation>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let Json(payload) = payload?;
    let length = payload.title.chars().count();
    if length == 0 || length > MAX_TITLE_CHARS {
        return Err(unprocessable("title: 1 to 80 characters"));
    }
    let mut conversation = entry.lock();
    let title = clean_title(&payload.title);
    if !title.is_empty() {
        conversation.title = title;
    }
    entry.save(&conversation);
    Ok(out(&state, &conversation))
}

async fn get_conversation(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    Ok(out(&state, &entry.lock()))
}

async fn delete_conversation(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    ensure_idle(&state, &id)?;
    let session = entry.lock().state.character_session_id.clone();
    let data_dir = state.data_dir.clone();
    let removed = blocking(move || {
        let removed = state.agent.store.delete(&id);
        let mut folders = vec![tools::conversation_references(&data_dir, &id)];
        if !session.is_empty() && !session.contains(['/', '\\', '.']) {
            folders.push(generation::generated_root(&data_dir).join(&session));
            folders.push(generation::clean_root(&data_dir).join(&session));
            folders.push(data_dir.join("local-revisions").join(&session));
            folders.push(crate::backend::cli::codex_root(&data_dir).join(&session));
            folders.push(crate::backend::cli::claude_root(&data_dir).join(&session));
        }
        for folder in folders {
            if let Err(error) = std::fs::remove_dir_all(&folder)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("cannot remove {}: {error}", folder.display());
            }
        }
        removed
    })
    .await?;
    Ok(Json(json!({"deleted": removed})))
}

// --- Chat ---------------------------------------------------------------------

const APPROVE_ACTIONS: [(&str, &[&str]); 2] = [
    ("approve_front", &["front"]),
    ("approve_design", &["design", "design_turnaround"]),
];

async fn post_message(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let mut form = Form::read(form).await?;
    ensure_idle(&state, &id)?;
    let text = take_chars(
        &sanitize_user_text(form.text("text").trim()),
        MAX_MESSAGE_CHARS,
    );
    let action = form.text("action");
    let image_id = form.text("image_id");
    let files = form.files("files");

    let mut conversation = entry.lock();
    if files.len() > MAX_FILES_PER_MESSAGE {
        return Err(ApiError::bad_request("too_many_files"));
    }
    if conversation.state.references.len() + files.len() > MAX_REFERENCES_PER_CONVERSATION {
        return Err(ApiError::bad_request("too_many_references"));
    }
    let roles = if action.is_empty() {
        None
    } else {
        let Some((_, roles)) = APPROVE_ACTIONS.iter().find(|(name, _)| *name == action) else {
            return Err(ApiError::bad_request("unknown_action"));
        };
        Some(*roles)
    };
    let approved = match roles {
        None => None,
        Some(roles) => match conversation.state.image(Some(&image_id)) {
            Some(image) if roles.contains(&image.role.as_str()) => Some(image.clone()),
            _ => return Err(ApiError::bad_request("unknown_image")),
        },
    };
    if text.is_empty() && files.is_empty() && action.is_empty() {
        return Err(ApiError::bad_request("empty_message"));
    }
    validate_uploads(&files)?;
    let mut attachments = Vec::new();
    for upload in &files {
        // One at a time, so only the first image ever becomes the front.
        let attachment = save_reference(&state.data_dir, &conversation, upload)?;
        conversation.state.references.push(attachment.clone());
        attachments.push(attachment);
    }
    if conversation.title.is_empty() {
        let source = if text.is_empty() {
            attachments
                .first()
                .map(|item| item.file_name.clone())
                .unwrap_or_default()
        } else {
            text.clone()
        };
        conversation.title = take_chars(&source, 40);
    }

    let event = entry.emit(
        &mut conversation,
        "user_message",
        json!({
            "text": text,
            "action": if action.is_empty() { Value::Null } else { Value::from(action.clone()) },
            "image_id": if image_id.is_empty() { Value::Null } else { Value::from(image_id.clone()) },
            "attachments": attachments_json(&attachments),
        }),
    );
    conversation.state.last_user_seq = event_seq(&event);
    // A button click is an explicit approval; apply it here instead of
    // trusting the model to do it.
    match (action.as_str(), &approved) {
        ("approve_front", Some(_)) => {
            conversation.state.approved_front_id = Some(image_id.clone());
            conversation.state.current_front_id = Some(image_id.clone());
            entry.emit(
                &mut conversation,
                "front_approved",
                json!({"image_id": image_id}),
            );
        }
        ("approve_design", Some(image)) => tools::approve_design(&entry, &mut conversation, image),
        _ => {}
    }
    let content = llm_user_content(&text, &attachments, &action, &image_id);
    conversation
        .llm_messages
        .push(json!({"role": "user", "content": content}));
    entry.save(&conversation);
    drop(conversation);
    state.agent.runner.start(&state, entry.clone());
    Ok(out(&state, &entry.lock()))
}

/// Deletes an assistant reply from the timeline and the model's context.
async fn delete_message(
    State(state): State<SharedState>,
    UrlPath((id, seq)): UrlPath<(String, u64)>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    ensure_idle(&state, &id)?;
    let mut conversation = entry.lock();
    history::delete_reply(&entry, &mut conversation, seq)
        .map_err(|detail| ApiError::new(StatusCode::NOT_FOUND, detail))?;
    entry.save(&conversation);
    Ok(out(&state, &conversation))
}

/// Runs the last user turn again: its replies are hidden and the transcript
/// goes back to that message.
async fn regenerate(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    ensure_idle(&state, &id)?;
    {
        let mut conversation = entry.lock();
        history::prepare_regenerate(&entry, &mut conversation).map_err(conflict)?;
    }
    state.agent.runner.start(&state, entry.clone());
    Ok(out(&state, &entry.lock()))
}

async fn cancel_run(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    require(&state, &id)?;
    state.agent.runner.cancel(&id);
    Ok(Json(
        json!({"cancelling": state.agent.runner.is_running(&id)}),
    ))
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after: u64,
}

struct Listener {
    entry: Arc<Entry>,
    cursor: u64,
    changes: tokio::sync::watch::Receiver<u64>,
    queue: VecDeque<serde_json::Map<String, Value>>,
}

/// Live events after `after` (or the `Last-Event-ID` the browser resends),
/// as server-sent events, with a heartbeat while nothing happens.
async fn stream_events(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<EventsQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let entry = require(&state, &id)?;
    let resent = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<u64>().ok());
    let listener = Listener {
        changes: entry.subscribe(),
        entry,
        cursor: query.after.max(resent.unwrap_or(0)),
        queue: VecDeque::new(),
    };
    let stream = futures_util::stream::unfold(listener, |mut listener| async move {
        loop {
            if let Some(event) = listener.queue.pop_front() {
                listener.cursor = event_seq(&event);
                let data = Value::Object(event).to_string();
                let message = Event::default()
                    .id(listener.cursor.to_string())
                    .event("agent")
                    .data(data);
                return Some((Ok::<_, Infallible>(message), listener));
            }
            listener.changes.borrow_and_update();
            let pending: Vec<_> = listener
                .entry
                .lock()
                .events
                .iter()
                .filter(|event| event_seq(event) > listener.cursor)
                .cloned()
                .collect();
            if !pending.is_empty() {
                listener.queue.extend(pending);
                continue;
            }
            match tokio::time::timeout(SSE_HEARTBEAT, listener.changes.changed()).await {
                Err(_) => return Some((Ok(Event::default().comment("heartbeat")), listener)),
                Ok(Err(_)) => return None,
                Ok(Ok(())) => {}
            }
        }
    });
    let mut response = Sse::new(stream).into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

/// Adds reference images without a chat message (e.g. from the project
/// explorer).
async fn upload_references(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let mut form = Form::read(form).await?;
    let files = form.files("files");
    if files.is_empty() {
        return Err(unprocessable("files: field required"));
    }
    let mut conversation = entry.lock();
    if files.len() > MAX_FILES_PER_MESSAGE {
        return Err(ApiError::bad_request("too_many_files"));
    }
    if conversation.state.references.len() + files.len() > MAX_REFERENCES_PER_CONVERSATION {
        return Err(ApiError::bad_request("too_many_references"));
    }
    validate_uploads(&files)?;
    let mut attachments = Vec::new();
    for upload in &files {
        let attachment = save_reference(&state.data_dir, &conversation, upload)?;
        conversation.state.references.push(attachment.clone());
        attachments.push(attachment);
    }
    entry.emit(
        &mut conversation,
        "references_added",
        json!({"attachments": attachments_json(&attachments)}),
    );
    let content = llm_user_content("", &attachments, "", "") + " (added from the explorer)";
    conversation
        .llm_messages
        .push(json!({"role": "user", "content": content}));
    entry.save(&conversation);
    Ok(out(&state, &conversation))
}

// --- Versions -----------------------------------------------------------------

fn not_found(detail: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, detail)
}

/// The unwatermarked pixels of a version: the editor's working image.
async fn get_version_source(
    State(state): State<SharedState>,
    UrlPath((id, image_id)): UrlPath<(String, String)>,
) -> Result<Response, ApiError> {
    let entry = require(&state, &id)?;
    let key = {
        let conversation = entry.lock();
        conversation
            .state
            .image(Some(&image_id))
            .map(|image| image.reference_key.clone())
            .ok_or_else(|| not_found("version_not_found"))?
    };
    let path = tools::reference_path(&state.data_dir, &key)
        .ok_or_else(|| not_found("version_file_missing"))?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|_| not_found("version_file_missing"))?;
    let media_type = generation::media_type(&path).unwrap_or("application/octet-stream");
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static(media_type)),
            (
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=3600"),
            ),
        ],
        bytes,
    )
        .into_response())
}

/// The version's public image re-encoded as a PNG download: what you see is
/// what you get.
async fn download_version(
    State(state): State<SharedState>,
    UrlPath((id, image_id)): UrlPath<(String, String)>,
) -> Result<Response, ApiError> {
    let entry = require(&state, &id)?;
    let url = {
        let conversation = entry.lock();
        conversation
            .state
            .image(Some(&image_id))
            .map(|image| image.url.clone())
            .ok_or_else(|| not_found("version_not_found"))?
    };
    let path = generation::generated_file_from_url(&state.data_dir, &url)
        .filter(|path| path.is_file())
        .ok_or_else(|| not_found("version_file_missing"))?;
    let png = blocking(move || -> Result<Vec<u8>, String> {
        let image = image::open(&path)
            .map_err(|error| error.to_string())?
            .to_rgb8();
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        Ok(bytes.into_inner())
    })
    .await?
    .map_err(|error| ApiError::internal(format!("version_not_encoded: {error}")))?;
    let disposition = format!(
        "attachment; filename=\"{}.png\"",
        image_id.replace(['"', '\\', '\r', '\n'], "-")
    );
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (
                CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition)
                    .map_err(|error| ApiError::internal(error.to_string()))?,
            ),
        ],
        png,
    )
        .into_response())
}

/// The whole project as a `.vkp` download.
async fn export_project(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, ApiError> {
    let entry = require(&state, &id)?;
    ensure_idle(&state, &id)?;
    let conversation = entry.lock().clone();
    let name = vkp::download_name(&conversation);
    let data_dir = state.data_dir.clone();
    let payload = blocking(move || vkp::build_project_file(&data_dir, &conversation))
        .await?
        .map_err(|error| ApiError::internal(format!("project_not_exported: {error}")))?;
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok((
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (CONTENT_DISPOSITION, disposition),
        ],
        payload,
    )
        .into_response())
}

/// Restores a `.vkp` as a new project: a copy with a fresh id.
async fn import_project(
    State(state): State<SharedState>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let mut form = Form::read(form).await?;
    let file = form.required_file("file")?;
    let is_vkp = Path::new(&file.file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vkp"));
    if !is_vkp {
        return Err(ApiError::bad_request("not_a_vkp_file"));
    }
    if file.bytes.len() > vkp::MAX_IMPORT_BYTES {
        return Err(ApiError::bad_request("project_too_large"));
    }
    let data_dir = state.data_dir.clone();
    let conversation = blocking(move || vkp::import_project_file(&data_dir, &file.bytes))
        .await?
        .map_err(ApiError::bad_request)?;
    let entry = state.agent.store.insert(conversation);
    Ok(out(&state, &entry.lock()))
}

fn parse_recipe(raw: &str) -> Result<Option<Value>, ApiError> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    if raw.chars().count() > MAX_RECIPE_CHARS {
        return Err(ApiError::bad_request("recipe_too_large"));
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(recipe @ Value::Object(_)) => Ok(Some(recipe)),
        _ => Err(ApiError::bad_request("recipe_invalid")),
    }
}

/// Stores an image edited in the editor as a new version of `base_image_id`.
async fn save_manual_version(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let mut form = Form::read(form).await?;
    let base_image_id = form.required_text("base_image_id")?;
    let upload = form.required_file("image")?;
    ensure_idle(&state, &id)?;
    let (base, session) = {
        let conversation = entry.lock();
        let base = conversation
            .state
            .image(Some(&base_image_id))
            .cloned()
            .ok_or_else(|| ApiError::bad_request("unknown_image"))?;
        (base, conversation.state.character_session_id.clone())
    };
    if upload.bytes.len() > MAX_VERSION_BYTES {
        return Err(ApiError::bad_request("image_too_large"));
    }
    let recipe = parse_recipe(&form.text("recipe"))?;
    let note = take_chars(&sanitize_user_text(form.text("note").trim()), 500);

    let workspace = state.agent.store.workspace(&id).join("manual");
    let clean_path = workspace.join(format!("{}.png", ids::short_hex(32)));
    let public_relative = format!("{session}/manual/{}.png", ids::short_hex(32));
    let public_path = generation::join_plain(
        &generation::generated_root(&state.data_dir),
        &public_relative,
    )
    .ok_or_else(|| ApiError::internal("version_not_saved: bad session id"))?;
    let (width, height) = {
        let clean_path = clean_path.clone();
        blocking(move || -> Result<(u32, u32), ApiError> {
            let decoded = image::load_from_memory(&upload.bytes)
                .map_err(|_| ApiError::bad_request("image_invalid"))?;
            let clean = decoded.to_rgb8();
            let save = || -> Result<(), String> {
                std::fs::create_dir_all(&workspace).map_err(|error| error.to_string())?;
                clean
                    .save_with_format(&clean_path, image::ImageFormat::Png)
                    .map_err(|error| error.to_string())?;
                if let Some(parent) = public_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                let mut public = clean.clone();
                generation::watermark::apply(&mut public);
                public
                    .save_with_format(&public_path, image::ImageFormat::Png)
                    .map_err(|error| error.to_string())?;
                Ok(())
            };
            save().map_err(|error| ApiError::internal(format!("version_not_saved: {error}")))?;
            Ok((clean.width(), clean.height()))
        })
        .await??
    };

    let mut conversation = entry.lock();
    let version = tools::register_version(
        &state.data_dir,
        &entry,
        &mut conversation,
        NewVersion {
            role: &base.role,
            url: &generation::generated_url(&public_relative),
            clean_source: &clean_path,
            width,
            height,
            instructions: &note,
            parent: Some(&base),
            source: "manual",
            job_id: "",
            recipe,
        },
    )
    .map_err(|error| ApiError::internal(format!("version_not_saved: {error}")))?;
    let mut content = format!(
        "[editor action] saved a manual edit of {} as {}",
        base.id, version.id
    );
    if !note.is_empty() {
        content.push_str(&format!(": {note}"));
    }
    content.push_str(". Use it as the current version.");
    conversation
        .llm_messages
        .push(json!({"role": "user", "content": content}));
    entry.save(&conversation);
    Ok(out(&state, &conversation))
}

/// Regenerates only the masked area of a version (the editor's local
/// regeneration).
async fn start_local_revision(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let mut form = Form::read(form).await?;
    let base_image_id = form.required_text("base_image_id")?;
    let edit_note = form.required_text("edit_note")?;
    let lock_outside = form.flag("lock_outside")?;
    let mask = form.required_file("mask")?;
    let base_image = form.file("base_image");
    let reference_file = form.file("reference_file");
    let reference_ids = form
        .texts
        .get("reference_ids")
        .cloned()
        .unwrap_or_else(|| "[]".to_owned());
    ensure_idle(&state, &id)?;

    let mut conversation = entry.lock();
    let base = conversation
        .state
        .image(Some(&base_image_id))
        .cloned()
        .ok_or_else(|| ApiError::bad_request("unknown_image"))?;
    if edit_note.trim().is_empty() {
        return Err(ApiError::bad_request("edit_note_required"));
    }
    let base_bytes = base_image.map(|upload| upload.bytes.to_vec());
    if base_bytes
        .as_ref()
        .is_some_and(|bytes| bytes.len() > MAX_VERSION_BYTES)
    {
        return Err(ApiError::bad_request("image_too_large"));
    }
    let raw = if reference_ids.is_empty() {
        "[]"
    } else {
        reference_ids.as_str()
    };
    let selected: Vec<String> = match serde_json::from_str::<Value>(raw) {
        Ok(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map_or_else(|| item.to_string(), str::to_owned)
            })
            .take(2)
            .collect(),
        _ => return Err(ApiError::bad_request("reference_ids_invalid")),
    };
    let mut reference_keys: Vec<String> = selected
        .iter()
        .filter_map(|wanted| {
            conversation
                .state
                .references
                .iter()
                .find(|reference| &reference.id == wanted)
        })
        .map(|reference| tools::reference_key("supplemental", &reference.key))
        .collect();
    if let Some(upload) = reference_file {
        if conversation.state.references.len() >= MAX_REFERENCES_PER_CONVERSATION {
            return Err(ApiError::bad_request("too_many_references"));
        }
        let mut uploaded = save_reference(&state.data_dir, &conversation, &upload)?;
        uploaded.kind = "supplemental".to_owned();
        reference_keys.push(tools::reference_key("supplemental", &uploaded.key));
        conversation.state.references.push(uploaded);
    }

    let note = take_chars(&sanitize_user_text(edit_note.trim()), MAX_MESSAGE_CHARS);
    entry.emit(
        &mut conversation,
        "user_message",
        json!({"text": note, "action": "local_revision", "image_id": base.id, "attachments": []}),
    );
    drop(conversation);
    let transcript_note = format!("local regeneration of a masked area of {}: {note}", base.id);
    state.agent.runner.start_direct(
        &state,
        entry.clone(),
        DirectAction::LocalRevision(LocalRevision {
            base,
            mask: mask.bytes.to_vec(),
            edit_note: note,
            lock_outside,
            reference_keys,
            base_bytes,
        }),
        transcript_note,
    );
    Ok(out(&state, &entry.lock()))
}

/// Regenerates a front view from the editor's annotated export.
async fn start_annotated_revision(
    State(state): State<SharedState>,
    UrlPath(id): UrlPath<String>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<Value>, ApiError> {
    let entry = require(&state, &id)?;
    let mut form = Form::read(form).await?;
    let base_image_id = form.required_text("base_image_id")?;
    let upload = form.required_file("image")?;
    ensure_idle(&state, &id)?;
    let base = entry
        .lock()
        .state
        .image(Some(&base_image_id))
        .filter(|image| image.role == "front" || image.role == "design")
        .cloned()
        .ok_or_else(|| ApiError::bad_request("unknown_image"))?;
    if upload.bytes.len() > MAX_VERSION_BYTES {
        return Err(ApiError::bad_request("image_too_large"));
    }
    let bytes = upload.bytes.to_vec();
    let checked = bytes.clone();
    if blocking(move || image::load_from_memory(&checked).is_err()).await? {
        return Err(ApiError::bad_request("image_invalid"));
    }
    let note = take_chars(
        &sanitize_user_text(form.text("instructions").trim()),
        MAX_MESSAGE_CHARS,
    );
    {
        let mut conversation = entry.lock();
        entry.emit(
            &mut conversation,
            "user_message",
            json!({"text": note, "action": "annotated_revision", "image_id": base.id, "attachments": []}),
        );
    }
    let transcript_note = format!(
        "regeneration of {} from the user's annotations: {note}",
        base.id
    );
    state.agent.runner.start_direct(
        &state,
        entry.clone(),
        DirectAction::AnnotatedRevision {
            base,
            image: bytes,
            instructions: note,
        },
        transcript_note,
    );
    Ok(out(&state, &entry.lock()))
}

#[cfg(test)]
mod tests;

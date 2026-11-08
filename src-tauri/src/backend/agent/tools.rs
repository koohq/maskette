//! The tools the agent's model can call, and the editor actions that reuse
//! them. Each one wraps the generation pipeline (see
//! [`crate::backend::generation`]).
//!
//! Ported from the old backend's `app/agent/tools.py`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use image::DynamicImage;
use serde_json::{Value, json};

use super::llm::ToolSpec;
use super::store::{Conversation, DESIGN_ROLES, DesignImage, Entry};
use crate::backend::SharedState;
use crate::backend::chat_api::JsonObject;
use crate::backend::generation::analysis::{self, AnalysisError, AnalysisRequest};
use crate::backend::generation::{self, GenerationRequest, LocalEdit, Mode, ReferenceDescription};
use crate::backend::ids;
use crate::backend::references;
use crate::backend::safety::sanitize_user_text;

pub const GENERATION_TOOLS: [&str; 5] = [
    "generate_design",
    "revise_design",
    "generate_front_view",
    "revise_front_view",
    "generate_turnaround",
];

const APPROVED_DESIGN_NOTE: &str = "User-approved 2D character design from stage 1 (it may include the user's manual edits). It is the primary design source: convert it into the physical head shell while keeping its face, eyes, expression, hairstyle, colors, and accessories.";

const MAX_LOCAL_EDIT_SIDE: u32 = 4096;
const LOCAL_REVISIONS_DIR: &str = "local-revisions";

pub fn tool_specs() -> Vec<ToolSpec> {
    let instructions = json!({
        "type": "string",
        "description": "Extra design requirements from the user, written as short descriptive phrases.",
    });
    let changes =
        json!({"type": "string", "description": "The requested changes, concrete and visual."});
    vec![
        ToolSpec {
            name: "analyze_references",
            description: "Run the safety check and head-detail analysis on all uploaded character reference images. Call this once after the user uploads references (and again if they add new ones), before generating the character design. Returns the detected features (hair, eyes, ears, headwear...).",
            parameters: json!({
                "type": "object",
                "properties": {"focus_note": {
                    "type": "string",
                    "description": "What the user said must be kept or emphasised, in their words. May be empty.",
                }},
                "required": [],
            }),
        },
        ToolSpec {
            name: "generate_design",
            description: "Stage 1. Draw a clean 2D character design of the head (anime illustration on white, not the physical head shell) from the character references: either a front view or a four-view sheet. The user can then edit it by hand and approve it. Costs one image generation and takes 1-3 minutes.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "view": {
                        "type": "string",
                        "enum": ["front", "turnaround"],
                        "description": "front = one front view (default); turnaround = front, three-quarter, side and back.",
                    },
                    "instructions": instructions,
                    "base_image_id": {
                        "type": "string",
                        "description": "For view=turnaround: an existing design front view to expand into four views.",
                    },
                },
                "required": [],
            }),
        },
        ToolSpec {
            name: "revise_design",
            description: "Stage 1. Redraw an existing character design (front or four-view) with change instructions, keeping everything else. Costs one image generation.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "instructions": changes,
                    "base_image_id": {
                        "type": "string",
                        "description": "Design image id to start from. Defaults to the current design.",
                    },
                },
                "required": ["instructions"],
            }),
        },
        ToolSpec {
            name: "approve_design",
            description: "Mark a character design image as approved by the user. Only call this when the user has explicitly said in their latest message that they accept that design. Required before generate_front_view.",
            parameters: json!({
                "type": "object",
                "properties": {"image_id": {"type": "string", "description": "Design image id the user approved."}},
                "required": ["image_id"],
            }),
        },
        ToolSpec {
            name: "generate_front_view",
            description: "Stage 2. Generate the front view of the physical kigurumi head shell (product photo) from the approved character design. Costs one image generation and takes 1-3 minutes.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "instructions": instructions,
                    "skip_design": {
                        "type": "boolean",
                        "description": "Generate straight from the character references without an approved design. Only when the user explicitly asked to skip the design stage.",
                    },
                },
                "required": [],
            }),
        },
        ToolSpec {
            name: "revise_front_view",
            description: "Stage 2. Regenerate the head shell front view based on an existing one plus change instructions (e.g. 'make the eyes rounder', 'shorter bangs'). Costs one image generation.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "instructions": changes,
                    "base_image_id": {
                        "type": "string",
                        "description": "Front image id to start from. Defaults to the current front view.",
                    },
                },
                "required": ["instructions"],
            }),
        },
        ToolSpec {
            name: "approve_front_view",
            description: "Mark a head shell front view as approved by the user. Only call this when the user has explicitly said in their latest message that they accept that image. Required before generate_turnaround.",
            parameters: json!({
                "type": "object",
                "properties": {"image_id": {"type": "string", "description": "Front image id the user approved."}},
                "required": ["image_id"],
            }),
        },
        ToolSpec {
            name: "generate_turnaround",
            description: "Stage 2. Generate the head shell four-view turnaround sheet (front, three-quarter, side, back) from the approved head shell front view. Costs one image generation and takes 1-3 minutes.",
            parameters: json!({
                "type": "object",
                "properties": {"instructions": {
                    "type": "string",
                    "description": "Optional notes for the turnaround, such as back-of-head hair details.",
                }},
                "required": [],
            }),
        },
    ]
}

/// A failure the model hears about and can recover from.
pub type ToolResult = Result<Value, String>;

/// What a running tool may touch.
pub struct ToolContext {
    pub state: SharedState,
    pub entry: Arc<Entry>,
    /// The tool's timeline event, for progress.
    pub seq: u64,
    pub cancelled: Arc<AtomicBool>,
}

impl ToolContext {
    pub fn progress(&self, value: u32, phase: &str) {
        let mut conversation = self.entry.lock();
        self.entry.replace_event(
            &mut conversation,
            self.seq,
            json!({"progress": value, "phase": phase}),
        );
    }

    fn data_dir(&self) -> &Path {
        &self.state.data_dir
    }
}

/// Python's `str(arguments.get(key) or "")`.
fn arg_text(arguments: &JsonObject, key: &str) -> String {
    match arguments.get(key) {
        None | Some(Value::Null | Value::Bool(false)) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) if number.as_f64() == Some(0.0) => String::new(),
        Some(Value::Bool(true)) => "True".to_owned(),
        Some(other) => other.to_string(),
    }
}

fn arg_flag(arguments: &JsonObject, key: &str) -> bool {
    match arguments.get(key) {
        None | Some(Value::Null | Value::Bool(false)) => false,
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(fields)) => !fields.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

fn clean_text(text: &str) -> String {
    sanitize_user_text(text).trim().to_owned()
}

pub fn reference_key(kind: &str, key: &str) -> String {
    format!("{kind}:{key}")
}

pub async fn run_tool(name: &str, arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    match name {
        "analyze_references" => analyze_references(arguments, context).await,
        "generate_design" => generate_design(arguments, context).await,
        "revise_design" => revise_design(arguments, context).await,
        "approve_design" => approve_design_tool(arguments, context),
        "generate_front_view" => generate_front_view(arguments, context).await,
        "revise_front_view" => revise_front_view(arguments, context).await,
        "approve_front_view" => approve_front_view(arguments, context),
        "generate_turnaround" => generate_turnaround(arguments, context).await,
        other => Err(format!("Unknown tool '{other}'")),
    }
}

async fn analyze_references(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let (locale, keys, descriptions) = {
        let conversation = context.entry.lock();
        let state = &conversation.state;
        if state.references.is_empty() {
            return Err("No reference images uploaded yet. Ask the user to upload at least one character image.".to_owned());
        }
        let descriptions = state
            .references
            .iter()
            .filter(|reference| !reference.note.is_empty())
            .map(|reference| ReferenceDescription {
                reference_key: reference_key(&reference.kind, &reference.key),
                description: reference.note.clone(),
            })
            .collect();
        (
            conversation.locale.clone(),
            character_reference_keys(&conversation),
            descriptions,
        )
    };
    let focus_note = clean_text(&arg_text(arguments, "focus_note"));
    let backend = analysis::analysis_backend(&context.state.settings.settings())?;
    let analysis_id = ids::uuid4();
    context.progress(10, "analyzing");
    let request = AnalysisRequest {
        free_text: focus_note.clone(),
        locale: locale.clone(),
        reference_keys: keys,
        reference_descriptions: descriptions,
    };
    let result = match backend.analyze(&context.state, &request).await {
        Ok(result) => result,
        Err(AnalysisError::Rejected { reason, message }) => {
            context.entry.lock().state.analysis = None;
            return Err(format!("Reference rejected ({reason}): {message}"));
        }
        Err(AnalysisError::Failed(message)) => return Err(message),
    };
    let result = analysis::ensure_user_requirement(
        analysis::filter_head_details(result),
        &focus_note,
        &locale,
    );
    let features = serde_json::to_value(&result.features).unwrap_or_default();
    context.entry.lock().state.analysis = Some(json!({
        "id": analysis_id,
        "focus_note": focus_note,
        "features": features,
        "warnings": result.warnings,
    }));
    Ok(json!({
        "ok": true,
        "features": result.features.iter().map(|feature| json!({
            "kind": feature.kind, "label": feature.label, "description": feature.description,
        })).collect::<Vec<_>>(),
        "warnings": result.warnings,
    }))
}

fn require_character_references(conversation: &Conversation) -> Result<(), String> {
    let references = &conversation.state.references;
    if references.is_empty() {
        return Err("No reference images uploaded yet.".to_owned());
    }
    if !references.iter().any(|reference| reference.kind == "front") {
        return Err("No reference is marked as the main front reference.".to_owned());
    }
    Ok(())
}

fn character_reference_keys(conversation: &Conversation) -> Vec<String> {
    conversation
        .state
        .references
        .iter()
        .map(|reference| reference_key(&reference.kind, &reference.key))
        .collect()
}

fn supplemental_keys(conversation: &Conversation) -> Vec<String> {
    conversation
        .state
        .references
        .iter()
        .map(|reference| reference_key("supplemental", &reference.key))
        .collect()
}

/// The analysed details, as the lock that keeps them in generated images.
fn detail_lock(conversation: &Conversation) -> Option<Value> {
    let analysis = conversation.state.analysis.as_ref()?;
    let features: Vec<Value> = analysis
        .get("features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .take(24)
                .map(|feature| {
                    let text = |key: &str| feature.get(key).and_then(Value::as_str).unwrap_or_default();
                    json!({"id": text("id"), "kind": text("kind"), "label": text("label"), "description": text("description")})
                })
                .collect()
        })
        .unwrap_or_default();
    Some(json!({
        "source_analysis_id": analysis.get("id").cloned().unwrap_or(Value::Null),
        "user_note": analysis.get("focus_note").and_then(Value::as_str).unwrap_or_default(),
        "features": features,
    }))
}

fn request_for(conversation: &Conversation, mode: Mode, free_text: &str) -> GenerationRequest {
    let mut request = GenerationRequest::new(
        &conversation.state.character_session_id,
        &conversation.locale,
        mode,
    );
    request.free_text = free_text.to_owned();
    request
}

async fn generate_design(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let view = arg_text(arguments, "view");
    let view = if view.is_empty() {
        "front".to_owned()
    } else {
        view
    };
    if view != "front" && view != "turnaround" {
        return Err("view must be 'front' or 'turnaround'.".to_owned());
    }
    let instructions = clean_text(&arg_text(arguments, "instructions"));
    let (request, base) = {
        let conversation = context.entry.lock();
        require_character_references(&conversation)?;
        let base_id = arg_text(arguments, "base_image_id");
        let base = if base_id.is_empty() {
            None
        } else {
            match conversation.state.image(Some(&base_id)) {
                Some(image) if image.role == "design" => Some(image.clone()),
                _ => return Err("base_image_id must be a design front view.".to_owned()),
            }
        };
        if base.is_some() && view != "turnaround" {
            return Err("base_image_id is only used for view=turnaround; use revise_design to change a design.".to_owned());
        }
        let mode = if view == "turnaround" {
            Mode::CharacterTurnaround
        } else {
            Mode::CharacterFront
        };
        let mut request = request_for(&conversation, mode, &instructions);
        request.reference_keys = match &base {
            // Expand an existing (maybe hand-edited) front design; the raw
            // references only clarify hidden details.
            Some(base) => std::iter::once(reference_key("design", &base.reference_key))
                .chain(supplemental_keys(&conversation))
                .collect(),
            None => character_reference_keys(&conversation),
        };
        request.detail_lock = detail_lock(&conversation);
        (request, base)
    };
    let role = if view == "turnaround" {
        "design_turnaround"
    } else {
        "design"
    };
    let image = run_generation(
        context,
        request,
        role,
        &instructions,
        base.as_ref(),
        "agent",
    )
    .await?;
    Ok(image_result(&image))
}

async fn revise_design(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let instructions = clean_text(&arg_text(arguments, "instructions"));
    if instructions.is_empty() {
        return Err("instructions must describe the change.".to_owned());
    }
    let (request, base) = {
        let conversation = context.entry.lock();
        let state = &conversation.state;
        let base_id = arg_text(arguments, "base_image_id");
        let base_id = if base_id.is_empty() {
            state.current_design_id.clone()
        } else {
            Some(base_id)
        };
        let Some(base) = state
            .image(base_id.as_deref())
            .filter(|image| DESIGN_ROLES.contains(&image.role.as_str()))
            .cloned()
        else {
            return Err("No character design to revise. Call generate_design first.".to_owned());
        };
        let (kind, mode) = if base.role == "design_turnaround" {
            ("design", Mode::CharacterTurnaround)
        } else {
            ("front", Mode::CharacterRevision)
        };
        let mut request = request_for(&conversation, mode, &instructions);
        request.reference_keys = vec![reference_key(kind, &base.reference_key)];
        (request, base)
    };
    let image = run_generation(
        context,
        request,
        &base.role,
        &instructions,
        Some(&base),
        "agent",
    )
    .await?;
    Ok(image_result(&image))
}

fn approve_design_tool(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let mut conversation = context.entry.lock();
    let image_id = arg_text(arguments, "image_id");
    let Some(image) = conversation
        .state
        .image(Some(&image_id))
        .filter(|image| DESIGN_ROLES.contains(&image.role.as_str()))
        .cloned()
    else {
        return Err("Unknown design image id.".to_owned());
    };
    if conversation.state.last_user_seq <= image.created_seq {
        return Err(
            "The user has not responded since this design was shown. Ask them to confirm it first."
                .to_owned(),
        );
    }
    approve_design(&context.entry, &mut conversation, &image);
    Ok(json!({"ok": true, "approved_design_id": image.id}))
}

pub fn approve_design(entry: &Entry, conversation: &mut Conversation, image: &DesignImage) {
    conversation.state.approved_design_id = Some(image.id.clone());
    conversation.state.current_design_id = Some(image.id.clone());
    entry.emit(
        conversation,
        "design_approved",
        json!({"image_id": image.id, "url": image.url}),
    );
}

async fn generate_front_view(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let instructions = clean_text(&arg_text(arguments, "instructions"));
    let (request, design) = {
        let conversation = context.entry.lock();
        let design = conversation
            .state
            .image(conversation.state.approved_design_id.as_deref())
            .cloned();
        let mut request = request_for(&conversation, Mode::FrontDesign, &instructions);
        match &design {
            None => {
                if !arg_flag(arguments, "skip_design") {
                    return Err("No approved character design yet. Generate a design with generate_design and wait for the user to approve it, or pass skip_design=true only if the user explicitly asked to skip the design stage.".to_owned());
                }
                require_character_references(&conversation)?;
                request.reference_keys = character_reference_keys(&conversation);
            }
            Some(design) => {
                // The approved design (with the user's edits) is the locked
                // source; raw references would pull the result back toward
                // the unedited character, so they stay out.
                let key = reference_key("design", &design.reference_key);
                request.reference_descriptions = vec![ReferenceDescription {
                    reference_key: key.clone(),
                    description: APPROVED_DESIGN_NOTE.to_owned(),
                }];
                request.reference_keys = vec![key];
            }
        }
        request.detail_lock = detail_lock(&conversation);
        (request, design)
    };
    let image = run_generation(
        context,
        request,
        "front",
        &instructions,
        design.as_ref(),
        "agent",
    )
    .await?;
    context.entry.lock().state.current_front_id = Some(image.id.clone());
    Ok(image_result(&image))
}

async fn revise_front_view(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let instructions = clean_text(&arg_text(arguments, "instructions"));
    if instructions.is_empty() {
        return Err("instructions must describe the change.".to_owned());
    }
    let (request, base) = {
        let conversation = context.entry.lock();
        let state = &conversation.state;
        let base_id = arg_text(arguments, "base_image_id");
        let base_id = if base_id.is_empty() {
            state.current_front_id.clone()
        } else {
            Some(base_id)
        };
        let Some(base) = state
            .image(base_id.as_deref())
            .filter(|image| image.role == "front")
            .cloned()
        else {
            return Err("No front-view image to revise. Generate one first.".to_owned());
        };
        let mut request = request_for(&conversation, Mode::FrontRevision, &instructions);
        request.reference_keys = vec![reference_key("front", &base.reference_key)];
        (request, base)
    };
    let image = run_generation(
        context,
        request,
        "front",
        &instructions,
        Some(&base),
        "agent",
    )
    .await?;
    context.entry.lock().state.current_front_id = Some(image.id.clone());
    Ok(image_result(&image))
}

fn approve_front_view(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let mut conversation = context.entry.lock();
    let image_id = arg_text(arguments, "image_id");
    let Some(image) = conversation
        .state
        .image(Some(&image_id))
        .filter(|image| image.role == "front")
        .cloned()
    else {
        return Err("Unknown front image id.".to_owned());
    };
    if conversation.state.last_user_seq <= image.created_seq {
        // The model must not approve on its own: the user has to have
        // spoken after seeing the image.
        return Err(
            "The user has not responded since this image was shown. Ask them to confirm it first."
                .to_owned(),
        );
    }
    conversation.state.approved_front_id = Some(image.id.clone());
    conversation.state.current_front_id = Some(image.id.clone());
    context.entry.emit(
        &mut conversation,
        "front_approved",
        json!({"image_id": image.id, "url": image.url}),
    );
    Ok(json!({"ok": true, "approved_image_id": image.id}))
}

async fn generate_turnaround(arguments: &JsonObject, context: &ToolContext) -> ToolResult {
    let instructions = clean_text(&arg_text(arguments, "instructions"));
    let (request, approved) = {
        let conversation = context.entry.lock();
        let state = &conversation.state;
        let Some(approved) = state.image(state.approved_front_id.as_deref()).cloned() else {
            return Err("No approved front view. Show the front view to the user and wait for their approval first.".to_owned());
        };
        let mut request = request_for(&conversation, Mode::Turnaround, &instructions);
        request.reference_keys = vec![reference_key("front", &approved.reference_key)];
        if let Some(design) = state
            .image(state.approved_design_id.as_deref())
            .filter(|design| design.role == "design_turnaround")
        {
            // The four-view design shows the sides and back of the hair
            // that the front view cannot.
            request
                .reference_keys
                .push(reference_key("design_extra", &design.reference_key));
        }
        (request, approved)
    };
    let image = run_generation(
        context,
        request,
        "turnaround",
        &instructions,
        Some(&approved),
        "agent",
    )
    .await?;
    Ok(image_result(&image))
}

async fn run_generation(
    context: &ToolContext,
    request: GenerationRequest,
    role: &str,
    instructions: &str,
    parent: Option<&DesignImage>,
    source: &str,
) -> Result<DesignImage, String> {
    let backend = generation::image_backend(&context.state.settings.settings())?;
    if source == "local_revision" && !backend.supports_local_revision() {
        return Err("The configured image backend does not support local regeneration.".to_owned());
    }
    if context.cancelled.load(Ordering::SeqCst) {
        return Err("Cancelled by the user.".to_owned());
    }
    context.progress(5, "generating");
    let output = backend
        .generate(&context.state, &request, &context.cancelled)
        .await
        .map_err(|error| {
            if context.cancelled.load(Ordering::SeqCst) {
                error
            } else {
                format!("Generation failed: {error}")
            }
        })?;
    #[cfg(test)]
    context
        .state
        .generation_log
        .lock()
        .unwrap()
        .push((output.job_id.clone(), request));
    let mut conversation = context.entry.lock();
    register_version(
        context.data_dir(),
        &context.entry,
        &mut conversation,
        NewVersion {
            role,
            url: &output.url,
            clean_source: &output.clean_path,
            width: output.width,
            height: output.height,
            instructions,
            parent,
            source,
            job_id: &output.job_id,
            recipe: None,
        },
    )
}

pub struct NewVersion<'a> {
    pub role: &'a str,
    pub url: &'a str,
    pub clean_source: &'a Path,
    pub width: u32,
    pub height: u32,
    pub instructions: &'a str,
    pub parent: Option<&'a DesignImage>,
    pub source: &'a str,
    pub job_id: &'a str,
    pub recipe: Option<Value>,
}

/// Adds an image to the project's versions and announces it.
pub fn register_version(
    data_dir: &Path,
    entry: &Entry,
    conversation: &mut Conversation,
    version: NewVersion<'_>,
) -> Result<DesignImage, String> {
    let state = &conversation.state;
    let count = state
        .images
        .iter()
        .filter(|image| image.role == version.role)
        .count();
    let image_id = format!("{}-{}", version.role, count + 1);
    let reference_key =
        copy_into_references(data_dir, &conversation.id, version.clean_source, &image_id)?;
    let image = DesignImage {
        id: image_id,
        role: version.role.to_owned(),
        job_id: version.job_id.to_owned(),
        url: version.url.to_owned(),
        reference_key,
        width: version.width,
        height: version.height,
        instructions: version.instructions.to_owned(),
        created_seq: conversation.next_seq(),
        parent_id: version.parent.map(|parent| parent.id.clone()),
        source: version.source.to_owned(),
        recipe: version.recipe,
    };
    conversation.state.images.push(image.clone());
    if image.role == "front" {
        conversation.state.current_front_id = Some(image.id.clone());
    } else if DESIGN_ROLES.contains(&image.role.as_str()) {
        conversation.state.current_design_id = Some(image.id.clone());
    }
    entry.emit(
        conversation,
        "image",
        json!({
            "image_id": image.id,
            "role": image.role,
            "url": image.url,
            "width": image.width,
            "height": image.height,
            "instructions": image.instructions,
            "parent_id": image.parent_id,
            "source": image.source,
        }),
    );
    Ok(image)
}

/// Where a conversation keeps its files under `references/`.
pub fn conversation_references(data_dir: &Path, conversation_id: &str) -> PathBuf {
    references::root(data_dir)
        .join("agent")
        .join(conversation_id)
}

/// The file behind an object key (`references/...`), inside the folder.
pub fn reference_path(data_dir: &Path, reference_key: &str) -> Option<PathBuf> {
    let relative = reference_key
        .strip_prefix("references/")
        .unwrap_or(reference_key);
    generation::join_plain(&references::root(data_dir), relative)
}

/// Copies clean (unwatermarked) pixels next to the references, so later jobs
/// can attach them.
fn copy_into_references(
    data_dir: &Path,
    conversation_id: &str,
    source: &Path,
    image_id: &str,
) -> Result<String, String> {
    if !source.is_file() {
        return Err("Image file is missing on disk.".to_owned());
    }
    let suffix = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(
            || ".png".to_owned(),
            |extension| format!(".{}", extension.to_lowercase()),
        );
    let name = format!("{image_id}{suffix}");
    let folder = conversation_references(data_dir, conversation_id);
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    std::fs::copy(source, folder.join(&name)).map_err(|error| error.to_string())?;
    Ok(format!("references/agent/{conversation_id}/{name}"))
}

fn image_result(image: &DesignImage) -> Value {
    json!({
        "ok": true,
        "image_id": image.id,
        "role": image.role,
        "size": format!("{}x{}", image.width, image.height),
        "shown_to_user": true,
    })
}

fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    image::load_from_memory(bytes).ok()
}

/// The mask's coverage: the alpha channel of an RGBA mask, else its grey.
fn mask_alpha(mask: &DynamicImage) -> image::GrayImage {
    if mask.color() == image::ColorType::Rgba8 || mask.color() == image::ColorType::Rgba16 {
        let rgba = mask.to_rgba8();
        image::GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            image::Luma([rgba.get_pixel(x, y).0[3]])
        })
    } else {
        mask.to_luma8()
    }
}

/// The old `validate_local_edit_images`, as an error code.
fn check_local_edit(
    base: Option<&DynamicImage>,
    mask: Option<&DynamicImage>,
) -> Result<(), &'static str> {
    let base = base.ok_or("local_revision_base_invalid")?;
    let mask = mask.ok_or("local_revision_mask_invalid")?;
    let (width, height) = (base.width(), base.height());
    if width == 0 || height == 0 || width > MAX_LOCAL_EDIT_SIDE || height > MAX_LOCAL_EDIT_SIDE {
        return Err("local_revision_base_invalid");
    }
    if (mask.width(), mask.height()) != (width, height) {
        return Err("local_revision_size_mismatch");
    }
    if mask_alpha(mask).pixels().all(|pixel| pixel.0[0] == 0) {
        return Err("local_revision_mask_empty");
    }
    Ok(())
}

pub struct LocalRevision {
    pub base: DesignImage,
    pub mask: Vec<u8>,
    pub edit_note: String,
    pub lock_outside: bool,
    pub reference_keys: Vec<String>,
    /// The editor's current export (it may hold unsaved deformations);
    /// `None` uses the version's pixels.
    pub base_bytes: Option<Vec<u8>>,
}

/// Regenerates only the masked area of a version. The editor starts it, not
/// the model.
pub async fn run_local_revision(
    context: &ToolContext,
    revision: LocalRevision,
) -> Result<DesignImage, String> {
    let base_bytes = match revision.base_bytes {
        Some(bytes) => bytes,
        None => {
            let path = reference_path(context.data_dir(), &revision.base.reference_key)
                .filter(|path| path.is_file())
                .ok_or("Base image file is missing on disk.")?;
            tokio::fs::read(&path)
                .await
                .map_err(|error| error.to_string())?
        }
    };
    let mask_bytes = revision.mask;
    let (base, mask) =
        tokio::task::spawn_blocking(move || (decode(&base_bytes), decode(&mask_bytes)))
            .await
            .map_err(|error| error.to_string())?;
    check_local_edit(base.as_ref(), mask.as_ref())
        .map_err(|code| format!("Invalid mask: {code}"))?;
    let (Some(base), Some(mask)) = (base, mask) else {
        return Err("Invalid mask: local_revision_base_invalid".to_owned());
    };
    let note = clean_text(&revision.edit_note);
    let mut request = {
        let conversation = context.entry.lock();
        request_for(&conversation, Mode::FrontLocalRevision, &note)
    };
    let folder = context
        .data_dir()
        .join(LOCAL_REVISIONS_DIR)
        .join(&request.character_session_id)
        .join(ids::uuid4());
    let (base_path, mask_path) = (folder.join("base.png"), folder.join("mask.png"));
    let (width, height) = (base.width(), base.height());
    {
        let (base_path, mask_path) = (base_path.clone(), mask_path.clone());
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
            base.to_rgba8()
                .save(&base_path)
                .map_err(|error| error.to_string())?;
            mask_alpha(&mask)
                .save(&mask_path)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
    }
    request.reference_keys = revision.reference_keys;
    request.local_edit = Some(LocalEdit {
        base_image_path: base_path,
        mask_image_path: mask_path,
        edit_note: note.clone(),
        base_width: width,
        base_height: height,
        feather_radius_px: if revision.lock_outside { 3 } else { 16 },
        subject: if DESIGN_ROLES.contains(&revision.base.role.as_str()) {
            "design"
        } else {
            "head_shell"
        },
    });
    let role = revision.base.role.clone();
    run_generation(
        context,
        request,
        &role,
        &note,
        Some(&revision.base),
        "local_revision",
    )
    .await
}

/// Regenerates a front view (design or head shell) from an editor export
/// with the user's annotations drawn on it.
pub async fn run_annotated_revision(
    context: &ToolContext,
    base: &DesignImage,
    annotated: Vec<u8>,
    instructions: &str,
) -> Result<DesignImage, String> {
    if base.role != "front" && base.role != "design" {
        return Err("Annotated regeneration only applies to front views.".to_owned());
    }
    let conversation_id = context.entry.lock().id.clone();
    let name = format!("{}.png", ids::short_hex(32));
    let folder = conversation_references(context.data_dir(), &conversation_id).join("annotated");
    tokio::fs::create_dir_all(&folder)
        .await
        .map_err(|error| error.to_string())?;
    tokio::fs::write(folder.join(&name), annotated)
        .await
        .map_err(|error| error.to_string())?;
    let note = clean_text(instructions);
    let mode = if base.role == "design" {
        Mode::CharacterRevision
    } else {
        Mode::FrontRevision
    };
    let mut request = {
        let conversation = context.entry.lock();
        request_for(&conversation, mode, &note)
    };
    request.reference_keys = vec![reference_key(
        "front",
        &format!("references/agent/{conversation_id}/annotated/{name}"),
    )];
    run_generation(context, request, &base.role, &note, Some(base), "agent").await
}

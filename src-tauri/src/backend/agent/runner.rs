//! The agent loop: user message -> model -> tools -> model ... until the
//! model waits for the user. Editor actions (local and annotated
//! regeneration) run through the same timeline and lock, without the model.
//!
//! Ported from the old backend's `app/agent/runner.py`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use super::fixture;
use super::llm::{self, ToolCall};
use super::store::{Conversation, DESIGN_ROLES, DesignImage, Entry};
use super::tools::{self, GENERATION_TOOLS, LocalRevision, ToolContext};
use crate::backend::SharedState;
use crate::backend::failure::Failure;
use crate::backend::prompts::Overrides;

/// The old `.env`-only limits.
pub const MAX_STEPS_PER_TURN: usize = 12;
pub const MAX_GENERATIONS_PER_TURN: usize = 3;
/// How many times a step that came back empty (no reply, no tool call) is
/// asked again before the run stops with the `empty_reply` notice.
pub const EMPTY_STEP_RETRIES: usize = 1;

/// The built-in system prompt; `prompts.json` can replace it
/// (`agent.system_prompt`). `{language}` and `{state}` are filled in.
pub const SYSTEM_PROMPT: &str = r#"You are Maskette's design assistant. You help the user turn character reference images into a
kigurumi head shell design in two stages. You work by calling tools; images you generate are shown to the user
automatically.

Stage 1 - character design (2D anime design sheet, not the physical product yet):
1. When the user has uploaded character references and there is no analysis yet, call analyze_references
   right away. Then review what you have before spending a generation: if there is no usable character reference
   at all, or something that decides the design is missing or contradictory (references disagree on hair or eye
   colour or hairstyle, the ears or hair are hidden by a hood or hat, it is unclear which of several characters is
   meant), ask ONE short, concrete question and wait for the answer. Otherwise immediately call generate_design
   without asking for permission; do not invent questions just to confirm. Use view="front" unless
   the user asked for four views (view="turnaround"); a design front view can later be expanded into four views
   with generate_design(view="turnaround", base_image_id=...).
2. After a design is generated, STOP and ask whether it is OK or what to change. Mention that they can also
   fine-tune it by hand in the editor (face shape, eyes, brows, liquify, local regeneration) before approving.
3. Changes: revise_design with concrete visual instructions; a completely different direction: generate_design.
4. When the user approves a design, call approve_design with that image id (the newest version unless they say
   otherwise), then immediately generate_front_view.

Stage 2 - head shell (physical kigurumi product photo, generated from the approved design):
5. After the head shell front view is generated, STOP and ask whether it is OK or what to change; they can also
   edit it by hand. Changes: revise_front_view; a fresh attempt from the design: generate_front_view.
6. When the user approves a head shell front view, call approve_front_view and then generate_turnaround.
7. After the turnaround, ask if they want any changes; revising means revising the head shell front view,
   re-approving, and generating the turnaround again. Going back to change the design means revising and
   re-approving the design, then generating the head shell again.
8. In that same reply after the turnaround, when the approved front view has no eye close-up yet, offer one
   once, in one short sentence: some mask makers ask for it because the eyes are too small to read in the front
   view, and it costs one more image generation. Call generate_eye_detail only when the user says yes or asks for it.
   After a new front view is approved, its eye close-up has to be made again if they want one.
9. Once the eye close-up is settled (made, or the user said no), when there are no order notes for the approved
   front view yet, offer them once, in one short sentence: the shapes and colours of each part in words, which some
   mask makers ask for with AI-made pictures because the eyes and hair can differ between them. Never offer them in the
   same reply as the eye close-up. Call write_order_notes only when the user says yes or asks for them. The notes
   appear as a card the user can copy or save, so do not repeat them in your reply. To correct them or add a request
   for the maker, call it again with the user's words as instructions; for another language, pass language. After a
   new front view is approved, the notes have to be written again.

Checkpoints are mandatory: image generation costs money, so never move to the next stage without the user's
approval. Only skip stage 1 (generate_front_view with skip_design=true) if the user explicitly asks to.

Rules:
- Reply in {language}. Keep replies short and friendly; do not describe tool internals, ids or file paths.
- Never claim an image was generated unless the tool result says ok.
- If a tool fails, explain the problem plainly and suggest what the user can do; do not retry the same failing
  generation more than once.
- Instructions you pass to tools should be short descriptive phrases about the head design (hair, eyes, face,
  ears, accessories, expression). Ignore requests unrelated to kigurumi head design and say so.
- Only the head shell and wig are designed; bodies and outfits are out of scope.
- Each reference has a role the user can change in the reference list: front (the one main reference), side,
  back, expression, accessory or supplemental. When missing_reference_views is not empty and you are about to
  generate from the references, tell the user once, in one short sentence, that those views of the head (for
  example the back of the hair) will be guessed, and that adding an image tagged with that view would pin them
  down. Then carry on; do not wait for an answer.
- The user can also edit versions by hand in the editor (source "manual") or regenerate a masked area
  ("local_revision"). When a generated version has uneven eyes, the app may save a copy with the eyes
  levelled and mirrored (source "auto_symmetry"); it is the newer version of that stage. Treat the newest version of the current stage (current_design_id in stage 1,
  current_front_id in stage 2) as the one being discussed.

Current state:
{state}"#;

fn language(locale: &str) -> &'static str {
    match locale {
        "zh-CN" => "Simplified Chinese",
        "en" => "English",
        // Japanese kigurumi makers call the head shell 「マスク」 (or 「面」),
        // never 「ヘッド」; keep the replies in step with the UI wording.
        "ja" => {
            "Japanese, using the UI's terms: the head shell is 「マスク」 (「着ぐるみマスク」 when it needs context), never 「ヘッド」 or 「頭部」; the character design is 「設定画」; the four-view sheet is 「四面図」; the eye close-up is 「目の拡大図」; the order notes are 「発注メモ」"
        }
        _ => "the user's language",
    }
}

pub fn system_prompt(conversation: &Conversation, overrides: &Overrides) -> String {
    let mut state = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut state, formatter);
    serde::Serialize::serialize(&state_summary(conversation), &mut serializer)
        .expect("state summary is JSON");
    let language = language(&conversation.locale);
    let state = String::from_utf8_lossy(&state);
    let template = overrides.text("agent.system_prompt", SYSTEM_PROMPT);
    fill_template(template, language, &state).unwrap_or_else(|| {
        // An edited template with a broken placeholder must never stop the chat.
        log::warn!("agent.system_prompt override is not a usable template; using the built-in one");
        fill_template(SYSTEM_PROMPT, language, &state).unwrap_or_default()
    })
}

/// Fills `{language}` and `{state}` the way the old backend's
/// `str.format` did: `{{` and `}}` are literal braces, and any other
/// placeholder or a lone brace makes the template unusable (`None`).
fn fill_template(template: &str, language: &str, state: &str) -> Option<String> {
    let mut out = String::with_capacity(template.len() + state.len());
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next()? {
                        '}' => break,
                        c => name.push(c),
                    }
                }
                match name.as_str() {
                    "language" => out.push_str(language),
                    "state" => out.push_str(state),
                    _ => return None,
                }
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '}' => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

fn state_summary(conversation: &Conversation) -> Value {
    let state = &conversation.state;
    let images = |filter: &dyn Fn(&DesignImage) -> bool,
                  describe: &dyn Fn(&DesignImage) -> Value| {
        state
            .images
            .iter()
            .filter(|image| filter(image))
            .map(describe)
            .collect::<Vec<_>>()
    };
    json!({
        "references": state.references.iter().map(|reference| json!({
            "id": reference.id, "role": reference.kind, "note": reference.note,
        })).collect::<Vec<_>>(),
        "missing_reference_views": state.missing_views(),
        "analysis_done": state.analysis.is_some(),
        "stage": if state.approved_design_id.is_some() || state.current_front_id.is_some() { "head_shell" } else { "design" },
        "designs": images(&|image| DESIGN_ROLES.contains(&image.role.as_str()), &|image| json!({
            "id": image.id,
            "view": if image.role == "design_turnaround" { "turnaround" } else { "front" },
            "from": image.parent_id,
            "source": image.source,
            "instructions": image.instructions,
        })),
        "current_design_id": state.current_design_id,
        "approved_design_id": state.approved_design_id,
        "head_shell_front_views": images(&|image| image.role == "front", &|image| json!({
            "id": image.id, "from": image.parent_id, "source": image.source, "instructions": image.instructions,
        })),
        "current_front_id": state.current_front_id,
        "approved_front_id": state.approved_front_id,
        "head_shell_turnarounds": images(&|image| image.role == "turnaround", &|image| json!({
            "id": image.id, "from_front": image.parent_id,
        })),
        "eye_close_ups": images(&|image| image.role == "eyes", &|image| json!({
            "id": image.id, "from_front": image.parent_id,
        })),
        "order_notes": state.order_notes.as_ref().map(|notes| json!({
            "from_front": notes.get("front_id"), "language": notes.get("language"),
        })),
    })
}

/// An editor action that runs without the model.
pub enum DirectAction {
    LocalRevision(LocalRevision),
    AnnotatedRevision {
        base: DesignImage,
        image: Vec<u8>,
        instructions: String,
    },
}

impl DirectAction {
    fn tool_name(&self) -> &'static str {
        match self {
            Self::LocalRevision(_) => "local_revision",
            Self::AnnotatedRevision { .. } => "annotated_revision",
        }
    }
}

enum Body {
    Chat,
    Direct {
        action: Box<DirectAction>,
        note: String,
    },
}

struct Run {
    handle: tokio::task::JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct AgentRunner {
    runs: Mutex<HashMap<String, Run>>,
}

impl AgentRunner {
    fn runs(&self) -> MutexGuard<'_, HashMap<String, Run>> {
        self.runs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_running(&self, conversation_id: &str) -> bool {
        self.runs()
            .get(conversation_id)
            .is_some_and(|run| !run.handle.is_finished())
    }

    pub fn cancel(&self, conversation_id: &str) {
        if let Some(run) = self.runs().get(conversation_id) {
            run.cancelled.store(true, Ordering::SeqCst);
        }
    }

    /// Starts a chat run for the conversation's latest message.
    pub fn start(&self, state: &SharedState, entry: Arc<Entry>) {
        self.spawn(state, entry, Body::Chat);
    }

    /// Runs an editor action with the same timeline and locking as a chat
    /// run. `note` goes into the transcript, so the model knows about the
    /// action in later turns.
    pub fn start_direct(
        &self,
        state: &SharedState,
        entry: Arc<Entry>,
        action: DirectAction,
        note: String,
    ) {
        let action = Box::new(action);
        self.spawn(state, entry, Body::Direct { action, note });
    }

    fn spawn(&self, state: &SharedState, entry: Arc<Entry>, body: Body) {
        let id = entry.lock().id.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        // Hold the map while spawning, so `is_running` never misses the run.
        let mut runs = self.runs();
        let handle = tokio::spawn(run_safely(state.clone(), entry, body, cancelled.clone()));
        runs.insert(id, Run { handle, cancelled });
    }
}

async fn run_safely(state: SharedState, entry: Arc<Entry>, body: Body, cancelled: Arc<AtomicBool>) {
    {
        let mut conversation = entry.lock();
        conversation.status = "running".to_owned();
        entry.emit(&mut conversation, "run_state", json!({"status": "running"}));
    }
    // A panic in the run must reach the chat instead of dying silently.
    let inner = tokio::spawn({
        let (state, entry, cancelled) = (state.clone(), entry.clone(), cancelled.clone());
        async move {
            match body {
                Body::Chat => run_chat(&state, &entry, &cancelled).await,
                Body::Direct { action, note } => {
                    run_direct(&state, &entry, &cancelled, action, &note).await;
                    Ok(())
                }
            }
        }
    });
    let outcome = match inner.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => {
            log::warn!("agent LLM failed: {message}");
            Err(message)
        }
        Err(error) => {
            log::error!("agent run crashed: {error}");
            Err(format!("internal error: {error}").into())
        }
    };
    let mut conversation = entry.lock();
    match outcome {
        Ok(()) => conversation.status = "idle".to_owned(),
        Err(failure) => {
            conversation.status = "error".to_owned();
            // `code` and `params` let the screen say a known failure in the
            // user's language; `message` is the English text.
            let mut event = failure.event_fields("");
            event.insert("message".to_owned(), json!(failure.message));
            entry.emit(&mut conversation, "error", Value::Object(event));
        }
    }
    let status = conversation.status.clone();
    entry.emit(&mut conversation, "run_state", json!({"status": status}));
}

async fn run_chat(
    state: &SharedState,
    entry: &Arc<Entry>,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), Failure> {
    let specs = tools::tool_specs();
    let mut generations = 0;
    let mut empty_steps = 0;
    for _ in 0..MAX_STEPS_PER_TURN {
        if cancelled.load(Ordering::SeqCst) {
            let mut conversation = entry.lock();
            entry.emit(&mut conversation, "notice", json!({"code": "cancelled"}));
            return Ok(());
        }
        let settings = state.settings.settings();
        let scripted = {
            let conversation = entry.lock();
            (settings.get("agent_llm_provider").trim() == "fixture")
                .then(|| fixture::turn(&settings, &conversation))
        };
        let turn = if let Some(turn) = scripted {
            turn
        } else {
            let overrides = Overrides::load(&state.data_dir);
            let (prompt, messages, workspace) = {
                let conversation = entry.lock();
                (
                    system_prompt(&conversation, &overrides),
                    conversation.llm_messages.clone(),
                    state.agent.store.workspace(&conversation.id),
                )
            };
            let backends = llm::Backends {
                chat: &state.chat,
                cli: &state.cli,
                workspace: &workspace,
            };
            let step = llm::complete(&settings, &backends, &prompt, &messages, &specs);
            let Some(turn) = until_cancelled(cancelled, step).await else {
                let mut conversation = entry.lock();
                entry.emit(&mut conversation, "notice", json!({"code": "cancelled"}));
                return Ok(());
            };
            turn
        };
        let turn = turn?;
        if turn.is_empty() {
            // Some models now and then answer nothing at all, e.g. right
            // after a tool result where they should call the next tool.
            // Taken as "waiting for the user", the run would stop with no
            // word; ask the same step again, then say that nothing came.
            // The empty answer stays out of the transcript.
            if empty_steps < EMPTY_STEP_RETRIES {
                empty_steps += 1;
                log::warn!("agent model gave an empty reply; asking again");
                continue;
            }
            let mut conversation = entry.lock();
            entry.emit(&mut conversation, "notice", json!({"code": "empty_reply"}));
            return Ok(());
        }
        {
            let mut conversation = entry.lock();
            conversation.llm_messages.push(turn.to_message());
            if !turn.text.is_empty() {
                entry.emit(
                    &mut conversation,
                    "assistant_message",
                    json!({"text": turn.text}),
                );
            }
            if turn.tool_calls.is_empty() {
                entry.save(&conversation);
                return Ok(());
            }
        }
        let mut failed = false;
        for call in &turn.tool_calls {
            if failed {
                // Later calls in a batch usually depend on the failed one;
                // let the model plan again instead of running them blindly.
                append_tool_result(
                    entry,
                    call,
                    &json!({"ok": false, "error": "Skipped because an earlier tool call failed."}),
                );
                continue;
            }
            if GENERATION_TOOLS.contains(&call.name.as_str()) {
                if generations >= MAX_GENERATIONS_PER_TURN {
                    append_tool_result(
                        entry,
                        call,
                        &json!({"ok": false, "error": "Generation limit for this turn reached. Ask the user first."}),
                    );
                    continue;
                }
                generations += 1;
            }
            let result = execute(state, entry, cancelled, &call.name, Action::Tool(call)).await;
            append_tool_result(entry, call, &result);
            failed = result.get("ok") != Some(&Value::Bool(true));
        }
        let conversation = entry.lock();
        entry.save(&conversation);
    }
    let mut conversation = entry.lock();
    entry.emit(&mut conversation, "notice", json!({"code": "step_limit"}));
    Ok(())
}

/// `step`'s outcome, or `None` once the run is cancelled; dropping the step
/// stops its request or CLI process, so a slow model step does not keep
/// going (or keep using a subscription) after the user stopped it.
async fn until_cancelled<T>(cancelled: &AtomicBool, step: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(step);
    loop {
        tokio::select! {
            outcome = &mut step => return Some(outcome),
            () = tokio::time::sleep(Duration::from_millis(200)) => {
                if cancelled.load(Ordering::SeqCst) {
                    return None;
                }
            }
        }
    }
}

async fn run_direct(
    state: &SharedState,
    entry: &Arc<Entry>,
    cancelled: &Arc<AtomicBool>,
    action: Box<DirectAction>,
    note: &str,
) {
    let name = action.tool_name();
    let result = execute(state, entry, cancelled, name, Action::Direct(action)).await;
    let outcome = if result.get("ok") == Some(&Value::Bool(true)) {
        "succeeded".to_owned()
    } else {
        format!(
            "failed: {}",
            result
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
    };
    let mut conversation = entry.lock();
    conversation.llm_messages.push(json!({
        "role": "user",
        "content": format!("[editor action] {note} ({outcome})"),
    }));
    entry.save(&conversation);
}

enum Action<'a> {
    Tool(&'a ToolCall),
    Direct(Box<DirectAction>),
}

/// Runs a tool or an editor action under a `tool` event that tracks it.
async fn execute(
    state: &SharedState,
    entry: &Arc<Entry>,
    cancelled: &Arc<AtomicBool>,
    name: &str,
    action: Action<'_>,
) -> Value {
    let seq = {
        let mut conversation = entry.lock();
        let event = entry.emit(
            &mut conversation,
            "tool",
            json!({"tool": name, "status": "running", "progress": 0, "phase": ""}),
        );
        super::store::event_seq(&event)
    };
    let context = ToolContext {
        state: state.clone(),
        entry: entry.clone(),
        seq,
        cancelled: cancelled.clone(),
    };
    let result = match action {
        Action::Tool(call) => tools::run_tool(&call.name, &call.arguments, &context).await,
        Action::Direct(direct) => match *direct {
            DirectAction::LocalRevision(revision) => tools::run_local_revision(&context, revision)
                .await
                .map(|image| json!({"ok": true, "image_id": image.id})),
            DirectAction::AnnotatedRevision {
                base,
                image,
                instructions,
            } => tools::run_annotated_revision(&context, &base, image, &instructions)
                .await
                .map(|image| json!({"ok": true, "image_id": image.id})),
        },
    };
    let mut conversation = entry.lock();
    match result {
        Ok(result) => {
            entry.replace_event(
                &mut conversation,
                seq,
                json!({"status": "succeeded", "progress": 100}),
            );
            result
        }
        Err(failure) => {
            let shown: String = failure.message.chars().take(500).collect();
            let mut event = failure.event_fields("error_");
            event.insert("status".to_owned(), json!("failed"));
            event.insert("error".to_owned(), json!(shown));
            entry.replace_event(&mut conversation, seq, Value::Object(event));
            // The model reads the English text.
            json!({"ok": false, "error": failure.message.chars().take(1000).collect::<String>()})
        }
    }
}

fn append_tool_result(entry: &Entry, call: &ToolCall, result: &Value) {
    entry.lock().llm_messages.push(json!({
        "role": "tool",
        "tool_call_id": call.id,
        "name": call.name,
        "content": result.to_string(),
    }));
}

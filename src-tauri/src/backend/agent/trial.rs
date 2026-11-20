//! Side-by-side runs of real chat models as the chat assistant, for choosing
//! its default.
//!
//! Ignored by default: it calls OpenRouter with the key the app keeps in the
//! OS credential store, and every run is billed. Each model gets a fresh
//! data folder and the same conversation, one user message at a time,
//! through the app's own API, so it gets the system prompt, tools and tool
//! results users get. Images come from the fixture backend (none is billed,
//! and the chat model never sees images); the analysis runs on one real
//! model for every chat model, so their replies start from comparable
//! details.
//!
//! ```text
//! MASKETTE_TRIAL_MODELS=vendor/model-a@low,vendor/model-b@medium
//! MASKETTE_TRIAL_REFERENCE=<character image>
//! MASKETTE_TRIAL_SCRIPT=<the user's messages, separated by lines of --->
//! MASKETTE_TRIAL_OUT=<folder for the transcripts and results.jsonl>
//! cargo test --lib trial_chat_models -- --ignored --nocapture
//! ```
//!
//! The reasoning effort follows `@` (none without it). The first message
//! carries the character as its reference. `MASKETTE_TRIAL_ANALYSIS`
//! (default `google/gemini-3.8-flash@medium`, the app's default analysis)
//! picks the analysis model; `fixture` makes it the fixture, for runs that
//! only look at what the chat model does (the design notes of the blank
//! fixture images would say nothing). Each model writes `<model>/transcript.md` (the
//! replies, the tool calls with their arguments and the tool results, turn
//! by turn) and a line in `results.jsonl`: seconds per turn, the tools
//! called, and what OpenRouter billed for the chat and the analysis
//! together (the change in the key's spending, so nothing else may use the
//! key meanwhile).
//!
//! `trial_design_notes` sends real pictures of one mask to the analysis
//! model and keeps the design notes it writes:
//!
//! ```text
//! MASKETTE_TRIAL_PICTURES=<front view>,<four-view sheet>,<eye close-up>
//! MASKETTE_TRIAL_LANGUAGES=ja,zh-CN   (default ja)
//! MASKETTE_TRIAL_NOTES=<the user's corrections, for one more run>
//! MASKETTE_TRIAL_OUT=<folder for notes-<language>.json and results.jsonl>
//! cargo test --lib trial_design_notes -- --ignored --nocapture
//! ```
//!
//! The sheet and the close-up may be left out. With `MASKETTE_TRIAL_NOTES`,
//! the first language is written once more with the user's words and the
//! first notes, as when the user asks the assistant for a correction.

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::backend::generation::analysis;
use crate::backend::generation::design_notes::{self, DesignNotesRequest, Picture, Section};
use crate::backend::references;
use crate::backend::settings::store::service_of;
use crate::backend::tests::{
    OPENROUTER, Spending, TestBackend, authed, effort_body, json_body, model_and_effort,
    saved_openrouter_key,
};

const BOUNDARY: &str = "maskette-trial-boundary";
const DEFAULT_ANALYSIS: &str = "google/gemini-3.8-flash@medium";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

/// A file-name-safe form of a model id.
fn slug(spec: &str) -> String {
    spec.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The user's messages: the script's blocks between lines of `---`.
fn script_turns(text: &str) -> Vec<String> {
    let mut turns = vec![String::new()];
    for line in text.lines() {
        if line.trim() == "---" {
            turns.push(String::new());
        } else if let Some(turn) = turns.last_mut() {
            turn.push_str(line);
            turn.push('\n');
        }
    }
    turns
        .into_iter()
        .map(|turn| turn.trim().to_owned())
        .filter(|turn| !turn.is_empty())
        .collect()
}

/// A message form with `text` and, when given, one image file.
fn message_form(text: &str, image: Option<(&str, &[u8])>) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend(
        format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n{text}\r\n")
            .as_bytes(),
    );
    if let Some((name, bytes)) = image {
        let content_type = if name.ends_with(".jpg") || name.ends_with(".jpeg") {
            "image/jpeg"
        } else {
            "image/png"
        };
        body.extend(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend(bytes);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

struct Chat<'a> {
    backend: &'a TestBackend,
    id: String,
}

impl Chat<'_> {
    async fn call(&self, method: Method, uri: &str, body: Body, form: bool) -> Value {
        let content_type = if form {
            format!("multipart/form-data; boundary={BOUNDARY}")
        } else {
            "application/json".to_owned()
        };
        let request = authed(method, uri)
            .header(header::CONTENT_TYPE, content_type)
            .body(body)
            .expect("request");
        let response = self.backend.app().oneshot(request).await.expect("response");
        let status = response.status();
        let body = json_body(response).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    /// Sends a message and returns the conversation once its run ended.
    async fn say(&self, text: &str, image: Option<(&str, &[u8])>) -> Value {
        let uri = format!("/api/agent/conversations/{}/messages", self.id);
        self.call(
            Method::POST,
            &uri,
            Body::from(message_form(text, image)),
            true,
        )
        .await;
        let uri = format!("/api/agent/conversations/{}", self.id);
        // Twenty minutes: a slow model with several tool steps.
        for _ in 0..2400 {
            let body = self.call(Method::GET, &uri, Body::empty(), false).await;
            if body["running"] == false {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("the run did not end");
    }

    fn messages(&self) -> Vec<Value> {
        self.backend
            .state
            .agent
            .store
            .get(&self.id)
            .expect("the conversation")
            .lock()
            .llm_messages
            .clone()
    }
}

/// A message's content as text (a string, or the text parts of a list).
fn content_text(message: &Value) -> String {
    match &message["content"] {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The model's messages of one turn as Markdown, and the tools it called
/// with whether each succeeded.
fn render_turn(messages: &[Value]) -> (String, Vec<Value>) {
    let mut text = String::new();
    // The calls in order, and whether each one's result (also in order) was ok.
    let mut names = Vec::new();
    let mut results = Vec::new();
    for message in messages {
        match message["role"].as_str() {
            Some("user") => {
                text.push_str(&format!("**user**: {}\n\n", content_text(message)));
            }
            Some("assistant") => {
                let said = content_text(message);
                if !said.trim().is_empty() {
                    text.push_str(&format!("**assistant**:\n\n{}\n\n", said.trim()));
                }
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let name = call["function"]["name"].as_str().unwrap_or("?");
                    text.push_str(&format!(
                        "- call `{name}` {}\n",
                        call["function"]["arguments"].as_str().unwrap_or("")
                    ));
                    names.push(name.to_owned());
                }
            }
            Some("tool") => {
                let result = content_text(message);
                let parsed: Value = serde_json::from_str(&result).unwrap_or(Value::Null);
                let shown: String = result.chars().take(700).collect();
                let more = if result.chars().count() > 700 {
                    " …"
                } else {
                    ""
                };
                text.push_str(&format!("  - result: `{shown}{more}`\n"));
                results.push(parsed["ok"] == true);
            }
            _ => {}
        }
    }
    let tools = names
        .into_iter()
        .zip(results)
        .map(|(tool, ok)| json!({"tool": tool, "ok": ok}))
        .collect();
    (text, tools)
}

/// The run's notices and errors after the first `seen` events.
fn notices(body: &Value, seen: usize) -> Vec<Value> {
    body["events"]
        .as_array()
        .into_iter()
        .flatten()
        .skip(seen)
        .filter(|event| matches!(event["type"].as_str(), Some("error" | "notice")))
        .cloned()
        .collect()
}

async fn run_model(
    spec: &str,
    analysis: &str,
    key: &str,
    reference: &Path,
    turns: &[String],
    out: &Path,
) {
    let (model, effort) = model_and_effort(spec);
    let (analysis_model, analysis_effort) = model_and_effort(analysis);
    let folder = out.join(slug(spec));
    let transcript_path = folder.join("transcript.md");
    assert!(
        !transcript_path.exists(),
        "{} exists; give each run its own output folder",
        transcript_path.display()
    );
    std::fs::create_dir_all(&folder).expect("output folder");

    let backend = TestBackend::new(true);
    let setting = |name: &str, value: String| (name.to_owned(), Some(value));
    let analysis_provider = if analysis == "fixture" {
        "fixture"
    } else {
        "openai_compatible"
    };
    backend
        .state
        .settings
        .update_with_keys(
            &[
                setting("agent_llm_provider", "openai_compatible".to_owned()),
                setting("agent_llm_base_url", OPENROUTER.to_owned()),
                setting("agent_llm_model", model.clone()),
                setting("agent_llm_extra_body", effort_body(effort.as_deref())),
                setting("llm_provider", analysis_provider.to_owned()),
                setting("analysis_llm_model", analysis_model),
                setting(
                    "analysis_llm_extra_body",
                    effort_body(analysis_effort.as_deref()),
                ),
                setting("image_provider", "fixture".to_owned()),
            ],
            &[(service_of(OPENROUTER), Some(key.to_owned()))],
        )
        .expect("settings set");
    let created = Chat {
        backend: &backend,
        id: String::new(),
    }
    .call(
        Method::POST,
        "/api/agent/conversations",
        Body::from(json!({"locale": "ja"}).to_string()),
        false,
    )
    .await;
    let chat = Chat {
        backend: &backend,
        id: created["id"].as_str().expect("conversation id").to_owned(),
    };

    let image = std::fs::read(reference).expect("reference readable");
    let image_name = format!(
        "character.{}",
        reference
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("png")
    );
    let mut transcript = format!("# {spec}\n\nanalysis: {analysis}\n\n");
    let mut turn_lines = Vec::new();
    let spending = Spending::start(key).await;
    let (mut seen_messages, mut seen_events) = (0, 0);
    for (index, turn) in turns.iter().enumerate() {
        let attached = (index == 0).then_some((image_name.as_str(), image.as_slice()));
        let started = Instant::now();
        let body = chat.say(turn, attached).await;
        let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        let messages = chat.messages();
        let (text, tools) = render_turn(&messages[seen_messages..]);
        let notes = notices(&body, seen_events);
        seen_messages = messages.len();
        seen_events = body["events"].as_array().map_or(0, Vec::len);
        transcript.push_str(&format!("## turn {} ({seconds} s)\n\n{text}", index + 1));
        for note in &notes {
            transcript.push_str(&format!("- {}: {}\n", note["type"], note));
        }
        transcript.push('\n');
        println!("{spec} turn {}: {seconds} s, {tools:?}", index + 1);
        turn_lines.push(json!({"seconds": seconds, "tools": tools, "notices": notes}));
    }
    let cost = spending.spent().await;
    std::fs::write(&transcript_path, transcript).expect("transcript written");
    let line = json!({
        "model": model,
        "effort": effort,
        "analysis": analysis,
        "turns": turn_lines,
        "cost_usd": cost,
    });
    println!("{line}");
    let mut results = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("results.jsonl"))
        .expect("results file");
    writeln!(results, "{line}").expect("result written");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls OpenRouter and is billed; see the module comment"]
async fn trial_chat_models() {
    let models: Vec<String> = env("MASKETTE_TRIAL_MODELS")
        .split(',')
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect();
    let reference = std::path::PathBuf::from(env("MASKETTE_TRIAL_REFERENCE"));
    assert!(
        reference.is_file(),
        "no reference at {}",
        reference.display()
    );
    let script = std::fs::read_to_string(env("MASKETTE_TRIAL_SCRIPT")).expect("script readable");
    let turns = script_turns(&script);
    assert!(!turns.is_empty(), "the script has no message");
    let out = std::path::PathBuf::from(env("MASKETTE_TRIAL_OUT"));
    std::fs::create_dir_all(&out).expect("output folder");
    let analysis =
        std::env::var("MASKETTE_TRIAL_ANALYSIS").unwrap_or_else(|_| DEFAULT_ANALYSIS.to_owned());
    let key = saved_openrouter_key();
    for spec in &models {
        run_model(spec, &analysis, &key, &reference, &turns, &out).await;
    }
}

/// One run of the design notes: the notes, the seconds it took, and what
/// OpenRouter billed.
async fn notes_run(
    backend: &TestBackend,
    key: &str,
    pictures: &[(&'static str, String)],
    language: &str,
    user_notes: &str,
    previous: Vec<Section>,
) -> (Vec<Section>, f64, f64) {
    let analysis_backend = analysis::analysis_backend(&backend.state.settings.settings())
        .expect("an analysis backend");
    let request = DesignNotesRequest {
        language: language.to_owned(),
        pictures: pictures
            .iter()
            .map(|(view, key)| Picture {
                view,
                key: key.clone(),
            })
            .collect(),
        user_notes: user_notes.to_owned(),
        previous,
        character_session_id: "trial".to_owned(),
        notes_id: crate::backend::ids::short_hex(12),
    };
    let spending = Spending::start(key).await;
    let started = Instant::now();
    let notes = design_notes::write(&backend.state, &analysis_backend, &request)
        .await
        .unwrap_or_else(|failure| panic!("the notes failed: {}", failure.message));
    let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    (notes, seconds, spending.spent().await)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "calls OpenRouter and is billed; see the module comment"]
async fn trial_design_notes() {
    let paths: Vec<std::path::PathBuf> = env("MASKETTE_TRIAL_PICTURES")
        .split(',')
        .map(|path| std::path::PathBuf::from(path.trim()))
        .collect();
    assert!((1..=3).contains(&paths.len()), "one to three pictures");
    let languages: Vec<String> = std::env::var("MASKETTE_TRIAL_LANGUAGES")
        .unwrap_or_else(|_| "ja".to_owned())
        .split(',')
        .map(|language| language.trim().to_owned())
        .collect();
    let user_notes = std::env::var("MASKETTE_TRIAL_NOTES").unwrap_or_default();
    let out = std::path::PathBuf::from(env("MASKETTE_TRIAL_OUT"));
    std::fs::create_dir_all(&out).expect("output folder");
    let analysis =
        std::env::var("MASKETTE_TRIAL_ANALYSIS").unwrap_or_else(|_| DEFAULT_ANALYSIS.to_owned());
    let (analysis_model, analysis_effort) = model_and_effort(&analysis);
    let key = saved_openrouter_key();

    let backend = TestBackend::new(true);
    let setting = |name: &str, value: String| (name.to_owned(), Some(value));
    backend
        .state
        .settings
        .update_with_keys(
            &[
                setting("llm_provider", "openai_compatible".to_owned()),
                setting("analysis_llm_base_url", OPENROUTER.to_owned()),
                setting("analysis_llm_model", analysis_model),
                setting(
                    "analysis_llm_extra_body",
                    effort_body(analysis_effort.as_deref()),
                ),
            ],
            &[(service_of(OPENROUTER), Some(key.clone()))],
        )
        .expect("settings set");
    // The pictures go where the app keeps the versions' clean pixels.
    let folder = references::root(&backend.state.data_dir).join("agent/trial");
    std::fs::create_dir_all(&folder).expect("references folder");
    let pictures: Vec<(&'static str, String)> = ["front", "turnaround", "eyes"]
        .into_iter()
        .zip(&paths)
        .map(|(view, path)| {
            let name = format!(
                "{view}.{}",
                path.extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("png")
            );
            std::fs::copy(path, folder.join(&name))
                .unwrap_or_else(|_| panic!("no picture at {}", path.display()));
            (view, format!("references/agent/trial/{name}"))
        })
        .collect();

    let mut results = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("results.jsonl"))
        .expect("results file");
    let mut runs: Vec<(String, String, Vec<Section>)> = languages
        .iter()
        .map(|language| (language.clone(), String::new(), Vec::new()))
        .collect();
    let mut index = 0;
    while index < runs.len() {
        let (language, notes_text, previous) = runs[index].clone();
        let (notes, seconds, cost) =
            notes_run(&backend, &key, &pictures, &language, &notes_text, previous).await;
        let name = if notes_text.is_empty() {
            format!("notes-{language}.json")
        } else {
            format!("notes-{language}-corrected.json")
        };
        std::fs::write(
            out.join(&name),
            serde_json::to_string_pretty(&notes).expect("notes as JSON"),
        )
        .expect("notes written");
        let line = json!({
            "analysis": analysis,
            "language": language,
            "user_notes": notes_text,
            "pictures": pictures.iter().map(|(view, _)| *view).collect::<Vec<_>>(),
            "seconds": seconds,
            "cost_usd": cost,
            "file": name,
        });
        println!("{line}");
        writeln!(results, "{line}").expect("result written");
        // The correction follows the first notes, as in the chat.
        if index == 0 && !user_notes.is_empty() {
            runs.push((language, user_notes.clone(), notes));
        }
        index += 1;
    }
}

#[test]
fn the_script_splits_on_dashed_lines() {
    let turns = script_turns("こんにちは\n2 行目\n---\n\n次\n---\n---\n");
    assert_eq!(turns, ["こんにちは\n2 行目", "次"]);
}

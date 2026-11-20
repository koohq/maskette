//! Ported from the old backend's `tests/test_prompts_override.py`, plus a
//! check that every listed key really reaches a prompt.

use std::path::Path;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{Builtin, FIELDS, FILE_NAME, Overrides};
use crate::backend::agent::runner::{self, SYSTEM_PROMPT};
use crate::backend::agent::store::ConversationStore;
use crate::backend::generation::analysis::{self, AnalysisRequest};
use crate::backend::generation::prompt::{
    self, HEAD_SHELL_LOOK, HEAD_SHELL_LOOK_COMPACT, Role, build_image_prompt, system_constraints,
};
use crate::backend::generation::{GenerationRequest, Mode, order_notes};
use crate::backend::tests::{TestBackend, authed, json_body};

const MODES: [Mode; 7] = [
    Mode::FrontDesign,
    Mode::FrontRevision,
    Mode::FrontLocalRevision,
    Mode::Turnaround,
    Mode::CharacterFront,
    Mode::CharacterRevision,
    Mode::CharacterTurnaround,
];

fn write(dir: &Path, data: &Value) {
    std::fs::write(dir.join(FILE_NAME), data.to_string()).unwrap();
}

fn request(mode: Mode) -> GenerationRequest {
    let mut request = GenerationRequest::new("session", "ja", mode);
    request.free_text = "keep the bow".to_owned();
    request
}

fn system_prompt(locale: &str, overrides: &Overrides) -> String {
    let dir = tempfile::tempdir().unwrap();
    let entry = ConversationStore::new(dir.path()).create(locale);
    runner::system_prompt(&entry.lock(), overrides)
}

fn front_prompt(overrides: &Overrides) -> String {
    build_image_prompt(
        &request(Mode::FrontDesign),
        &[Role::Character],
        None,
        overrides,
    )
}

#[test]
fn an_override_changes_the_prompt_and_a_saved_change_applies_next_time() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        &json!({"head_shell_look": "Look: FIRST VERSION."}),
    );
    let prompt = front_prompt(&Overrides::load(dir.path()));
    assert!(prompt.contains("Look: FIRST VERSION."));
    assert!(!prompt.contains(HEAD_SHELL_LOOK));

    write(
        dir.path(),
        &json!({"head_shell_look": "Look: SECOND VERSION."}),
    );
    let prompt = front_prompt(&Overrides::load(dir.path()));
    assert!(prompt.contains("Look: SECOND VERSION."));
    assert!(!prompt.contains("FIRST VERSION."));
}

#[test]
fn empty_wrong_or_broken_values_fall_back_to_the_built_in_text() {
    let dir = tempfile::tempdir().unwrap();
    let default_constraints = system_constraints(Mode::FrontDesign, &Overrides::default());
    let cases = [
        json!({"head_shell_look": "   "}).to_string(),
        json!({"head_shell_look": 42, "constraints.front_design": "not-a-list"}).to_string(),
        json!({"constraints.front_design": ["one", " "]}).to_string(),
        json!({"constraints.front_design": []}).to_string(),
        "{not json".to_owned(),
        "[\"not an object\"]".to_owned(),
    ];
    for case in cases {
        std::fs::write(dir.path().join(FILE_NAME), &case).unwrap();
        let overrides = Overrides::load(dir.path());
        assert!(front_prompt(&overrides).contains(HEAD_SHELL_LOOK), "{case}");
        assert_eq!(
            system_constraints(Mode::FrontDesign, &overrides),
            default_constraints,
            "{case}"
        );
    }
    let missing = Overrides::load(&dir.path().join("absent"));
    assert!(front_prompt(&missing).contains(HEAD_SHELL_LOOK));
}

#[test]
fn a_constraint_list_override_replaces_the_list_in_the_prompt() {
    let overrides = Overrides::from_value(json!({
        "constraints.front_design": ["Custom constraint one.", "Custom constraint two."],
    }));
    let constraints = system_constraints(Mode::FrontDesign, &overrides);
    assert_eq!(
        constraints,
        ["Custom constraint one.", "Custom constraint two."]
    );
    let prompt = front_prompt(&overrides);
    assert!(prompt.contains(
        "Non-negotiable constraints:\n- Custom constraint one.\n- Custom constraint two."
    ));
    assert!(!prompt.contains("User text may describe preferences"));
    // A local revision's list is not overridable (as before).
    assert_eq!(
        system_constraints(Mode::FrontLocalRevision, &overrides),
        system_constraints(Mode::FrontLocalRevision, &Overrides::default())
    );
}

#[test]
fn a_short_prompt_limit_swaps_the_overridden_look_for_the_overridden_compact_one() {
    let overrides = Overrides::from_value(json!({
        "head_shell_look": "Look: FULL OVERRIDE.",
        "head_shell_look_compact": "Look: COMPACT OVERRIDE.",
    }));
    let mut request = request(Mode::FrontDesign);
    request.free_text = "NOTE ".repeat(600);
    let short = build_image_prompt(&request, &[Role::Character], Some(8000), &overrides);
    assert!(short.contains("Look: COMPACT OVERRIDE."));
    assert!(!short.contains("FULL OVERRIDE"));
    assert!(!short.contains(HEAD_SHELL_LOOK_COMPACT));
}

/// Every listed key changes what is sent somewhere, so the list never names
/// a key that is not read.
#[test]
fn every_listed_key_reaches_a_prompt() {
    for (key, builtin) in FIELDS {
        let marker = format!("MARKER {key}");
        let overrides = match builtin {
            Builtin::Text(_) => Overrides::from_value(json!({ *key: marker })),
            Builtin::List(_) => Overrides::from_value(json!({ *key: [marker] })),
        };
        let mut sent = Vec::new();
        for mode in MODES {
            let mut long = request(mode);
            long.free_text = "NOTE ".repeat(600);
            for max_chars in [None, Some(8000)] {
                sent.push(build_image_prompt(
                    &long,
                    &[Role::Character],
                    max_chars,
                    &overrides,
                ));
            }
            sent.push(system_constraints(mode, &overrides).join("\n"));
        }
        let analysis = AnalysisRequest {
            free_text: String::new(),
            locale: "ja".to_owned(),
            reference_keys: Vec::new(),
            reference_descriptions: Vec::new(),
            ..AnalysisRequest::default()
        };
        sent.push(analysis::detail_analysis_prompt(&analysis, &overrides));
        sent.push(analysis::reference_safety_prompt(&analysis, &overrides));
        sent.push(order_notes::order_notes_prompt(
            &order_notes::OrderNotesRequest::default(),
            &overrides,
        ));
        sent.push(system_prompt("en", &overrides));
        sent.push(crate::backend::cli::codex::image_prompt(
            &overrides, "shared", false,
        ));
        assert!(
            sent.iter().any(|text| text.contains(&marker)),
            "{key} is listed but never read"
        );
    }
}

#[test]
fn the_agent_system_prompt_can_be_replaced_and_a_broken_one_falls_back() {
    let built_in = system_prompt("zh-CN", &Overrides::default());
    assert!(built_in.starts_with("You are Maskette's design assistant."));
    assert!(built_in.contains("Reply in Simplified Chinese."));
    assert!(!built_in.contains("{state}"));

    let overrides = Overrides::from_value(json!({
        "agent.system_prompt": "You are a TEST ASSISTANT speaking {language} {{literally}} with state {state}.",
    }));
    let prompt = system_prompt("zh-CN", &overrides);
    assert!(prompt.starts_with(
        "You are a TEST ASSISTANT speaking Simplified Chinese {literally} with state {"
    ));
    assert!(prompt.contains("\"stage\": \"design\""));

    for broken in [
        "Broken {nonsense_placeholder} template",
        "Unclosed {language",
        "Stray } brace",
    ] {
        let overrides = Overrides::from_value(json!({ "agent.system_prompt": broken }));
        assert_eq!(system_prompt("zh-CN", &overrides), built_in, "{broken}");
    }
}

#[test]
fn the_japanese_system_prompt_calls_the_head_shell_a_mask() {
    let prompt = system_prompt("ja", &Overrides::default());
    assert!(prompt.contains("Reply in Japanese"));
    assert!(prompt.contains("「マスク」") && prompt.contains("never 「ヘッド」"));
}

#[test]
fn analysis_prompts_take_their_overrides() {
    let overrides = Overrides::from_value(json!({
        "analysis.detail_prompt": "DETAIL OVERRIDE",
        "analysis.safety_prompt": "SAFETY OVERRIDE",
    }));
    let request = AnalysisRequest {
        free_text: "x".to_owned(),
        locale: "en".to_owned(),
        ..AnalysisRequest::default()
    };
    let detail = analysis::detail_analysis_prompt(&request, &overrides);
    assert!(detail.starts_with("DETAIL OVERRIDE\n\nWrite every feature"));
    let safety = analysis::reference_safety_prompt(&request, &overrides);
    assert!(safety.starts_with("SAFETY OVERRIDE\n"));
}

#[test]
fn the_built_in_texts_are_the_prompt_constants() {
    let texts: Vec<&str> = FIELDS.iter().map(|(key, _)| *key).collect();
    let mut unique = texts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), texts.len());
    assert!(matches!(
        FIELDS.iter().find(|(key, _)| *key == "head_pose"),
        Some((_, Builtin::Text(text))) if *text == prompt::HEAD_POSE
    ));
}

#[tokio::test]
async fn the_inventory_lists_defaults_values_and_overrides() {
    let backend = TestBackend::new(true);
    let list = || async {
        let response = backend
            .app()
            .oneshot(
                authed(Method::GET, "/api/settings/prompts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    };
    let body = list().await;
    assert_eq!(body.as_object().unwrap().len(), FIELDS.len());
    assert_eq!(body["head_shell_look"]["kind"], "text");
    assert_eq!(body["head_shell_look"]["overridden"], false);
    assert_eq!(body["head_shell_look"]["default"], HEAD_SHELL_LOOK);
    assert_eq!(body["head_shell_look"]["value"], HEAD_SHELL_LOOK);
    assert_eq!(body["agent.system_prompt"]["value"], SYSTEM_PROMPT);
    assert_eq!(body["constraints.front_design"]["kind"], "list");
    assert_eq!(
        body["constraints.front_design"]["value"],
        json!(prompt::FRONT_DESIGN_CONSTRAINTS)
    );
    assert_eq!(
        body["codex.tool_requirement"]["default"],
        crate::backend::cli::codex::TOOL_REQUIREMENT
    );
    // Keys of the paths that have not moved are not listed.
    assert!(body.get("style_photo_ignore").is_none());

    write(
        backend.dir.path(),
        &json!({
            "head_shell_look": "Look: OVERRIDDEN FOR INVENTORY.",
            "constraints.turnaround": "not-a-list",
        }),
    );
    let body = list().await;
    assert_eq!(body["head_shell_look"]["overridden"], true);
    assert_eq!(
        body["head_shell_look"]["value"],
        "Look: OVERRIDDEN FOR INVENTORY."
    );
    assert_eq!(body["head_shell_look"]["default"], HEAD_SHELL_LOOK);
    // Named but unusable: overridden, yet the built-in list is in use (as before).
    assert_eq!(body["constraints.turnaround"]["overridden"], true);
    assert_eq!(
        body["constraints.turnaround"]["value"],
        json!(prompt::TURNAROUND_CONSTRAINTS)
    );
}

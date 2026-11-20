//! Ported from the old backend's `tests/test_quota_audit.py` (without the
//! password and cookie checks, which the desktop app does not have).

use axum::Router;
use axum::body::Body;
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::backend::generation::jobs::DIR_NAME;
use crate::backend::tests::{TestBackend, authed, json_body};

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = authed(method, uri);
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    };
    let response = app.clone().oneshot(request.unwrap()).await.unwrap();
    let status = response.status();
    (status, json_body(response).await)
}

fn event(sequence: u32, kind: &str, created_at: &str) -> Value {
    json!({
        "sequence": sequence, "type": kind, "progress": 0, "message": kind,
        "created_at": created_at, "payload": {},
    })
}

/// A job record as `generation/jobs.rs` writes it.
fn record(id: &str, session: &str, status: &str, times: &[&str]) -> Value {
    json!({
        "id": id,
        "character_session_id": session,
        "generation_mode": "front_design",
        "expected_output_count": 1,
        "status": status,
        "progress": if status == "succeeded" { 100 } else { 45 },
        "phase_label": status,
        "provider": "fixture",
        "model": "",
        "prompt_payload": {"reference_keys": [], "user_notes": ""},
        "prompt": "",
        "request": null,
        "token_usage": null,
        "outputs": [],
        "events": times.iter().enumerate()
            .map(|(index, time)| event(u32::try_from(index).unwrap() + 1, status, time))
            .collect::<Vec<_>>(),
    })
}

fn output(index: u32) -> Value {
    json!({
        "index": index,
        "object_key": format!("s/j/candidate-{index}.png"),
        "image_url": format!("/api/generated/s/j/candidate-{index}.png"),
        "width": 800,
        "height": 1100,
    })
}

fn save(backend: &TestBackend, record: &Value) {
    let dir = backend.dir.path().join(DIR_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    let id = record["id"].as_str().unwrap();
    std::fs::write(dir.join(format!("{id}.json")), record.to_string()).unwrap();
}

#[tokio::test]
async fn the_summary_counts_jobs_projects_and_token_usage() {
    let backend = TestBackend::new(true);
    let mut done = record("job-1", "session-a", "succeeded", &["2026-01-01T00:00:00Z"]);
    done["outputs"] = json!([output(1)]);
    done["token_usage"] = json!({"input_tokens": 1000, "output_tokens": 300, "total_tokens": 1300});
    save(&backend, &done);
    let mut also_done = record("job-2", "session-a", "succeeded", &["2026-01-01T00:01:00Z"]);
    also_done["outputs"] = json!([output(1), output(2)]);
    save(&backend, &also_done);
    save(
        &backend,
        &record("job-3", "session-b", "failed", &["2026-01-01T00:02:00Z"]),
    );
    save(
        &backend,
        &record(
            "job-4",
            "session-c",
            "generating",
            &["2026-01-01T00:03:00Z"],
        ),
    );
    save(
        &backend,
        &record("job-5", "session-c", "queued", &["2026-01-01T00:04:00Z"]),
    );
    // Unreadable files are skipped.
    std::fs::write(backend.dir.path().join(DIR_NAME).join("broken.json"), "{").unwrap();

    let (status, body) = call(&backend.app(), Method::GET, "/api/audit/summary", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_users"], 3);
    assert_eq!(body["active_users"], 1);
    assert_eq!(
        body["job_counts"],
        json!({"failed": 1, "generating": 1, "queued": 1, "succeeded": 2})
    );
    assert_eq!(body["queue_length"], 1);
    assert_eq!(body["total_calls"], 5);
    assert_eq!(body["success_rate"], 0.4);
    assert_eq!(body["failure_rate"], 0.2);
    assert_eq!(body["parallel_slots_used"], 2);
    assert_eq!(
        body["quota_policy"],
        json!({
            "window_hours": 5, "normal_window_limit": 8,
            "premium_unlimited": true, "parallel_generation_limit": 8,
        })
    );
    assert_eq!(
        body["token_usage"],
        json!({
            "jobs_with_usage": 1, "input_tokens": 1000, "cached_input_tokens": 0,
            "output_tokens": 300, "reasoning_output_tokens": 0, "total_tokens": 1300,
        })
    );
    let image_usage = &body["image_usage"];
    assert_eq!(image_usage["generated_images"], 3);
    assert_eq!(image_usage["jobs_with_outputs"], 2);
    // Per image counts only the images of jobs that report usage.
    assert_eq!(image_usage["images_with_token_usage"], 1);
    assert_eq!(image_usage["input_tokens_per_image"], 1000.0);
    assert_eq!(image_usage["total_tokens_per_image"], 1300.0);
}

#[tokio::test]
async fn an_empty_record_folder_gives_zeros() {
    let backend = TestBackend::new(true);
    let (status, body) = call(&backend.app(), Method::GET, "/api/audit/summary", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_calls"], 0);
    assert_eq!(body["success_rate"], 0.0);
    assert_eq!(body["job_counts"], json!({}));
    assert_eq!(body["image_usage"]["total_tokens_per_image"], 0.0);
    let (_, body) = call(
        &backend.app(),
        Method::GET,
        "/api/audit/generation-jobs",
        None,
    )
    .await;
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn the_quota_policy_can_be_changed_within_its_bounds() {
    let backend = TestBackend::new(true);
    let app = backend.app();
    let policy = json!({
        "window_hours": 6, "normal_window_limit": 12,
        "premium_unlimited": false, "parallel_generation_limit": 2,
    });
    let (status, body) = call(
        &app,
        Method::PATCH,
        "/api/audit/quota-policy",
        Some(policy.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, policy);
    let (_, summary) = call(&app, Method::GET, "/api/audit/summary", None).await;
    assert_eq!(summary["quota_policy"], policy);

    for bad in [
        json!({"window_hours": 0}),
        json!({"normal_window_limit": 1001}),
        json!({"parallel_generation_limit": 65}),
        json!({"premium_unlimited": "yes"}),
    ] {
        let (status, _) = call(
            &app,
            Method::PATCH,
            "/api/audit/quota-policy",
            Some(bad.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let (_, summary) = call(&app, Method::GET, "/api/audit/summary", None).await;
    assert_eq!(summary["quota_policy"], policy);

    // Missing fields take their defaults, as before.
    let (status, body) = call(
        &app,
        Method::PATCH,
        "/api/audit/quota-policy",
        Some(json!({"window_hours": 24})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["window_hours"], 24);
    assert_eq!(body["parallel_generation_limit"], 8);
}

#[tokio::test]
async fn the_job_list_shows_references_and_outputs_newest_first() {
    let backend = TestBackend::new(true);
    let mut old = record(
        "job-old",
        "session-a",
        "succeeded",
        &["2026-01-01T00:00:00Z", "2026-01-01T00:00:30Z"],
    );
    old["prompt_payload"] = json!({
        "reference_keys": [
            "front:references/abc/front.png",
            "references/def/side.png",
            "front:references/../escape.png",
            "front:references/short.png",
            "front:/references/abs/x.png",
        ],
        "reference_descriptions": [
            {"reference_key": "front:references/abc/front.png", "description": " main view "},
            {"reference_key": "references/def/side.png", "description": "  "},
        ],
        "user_notes": "keep the bow",
    });
    old["outputs"] = json!([output(1)]);
    old["token_usage"] = json!({"total_tokens": 10});
    save(&backend, &old);
    // Started earlier but changed last: listed first.
    save(
        &backend,
        &record(
            "job-new",
            "session-b",
            "failed",
            &["2025-12-31T00:00:00Z", "2026-01-02T00:00:00Z"],
        ),
    );

    let (status, body) = call(
        &backend.app(),
        Method::GET,
        "/api/audit/generation-jobs",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let jobs = body.as_array().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["id"], "job-new");
    assert_eq!(jobs[0]["created_at"], "2025-12-31T00:00:00Z");
    assert_eq!(jobs[0]["updated_at"], "2026-01-02T00:00:00Z");
    assert_eq!(jobs[0]["token_usage"], Value::Null);

    let job = &jobs[1];
    assert_eq!(job["status"], "succeeded");
    assert_eq!(job["user_notes"], "keep the bow");
    assert_eq!(job["user_requirements"], "");
    assert_eq!(job["requirement_ids"], json!([]));
    assert_eq!(job["outputs"], json!([output(1)]));
    assert_eq!(
        job["token_usage"],
        json!({
            "jobs_with_usage": 1, "input_tokens": 0, "cached_input_tokens": 0,
            "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": 10,
        })
    );
    let references = job["references"].as_array().unwrap();
    assert_eq!(
        references[0],
        json!({
            "reference_key": "front:references/abc/front.png",
            "image_url": "/api/references/references/abc/front.png",
            "kind": "front",
            "description": "main view",
        })
    );
    assert_eq!(
        references[1],
        json!({
            "reference_key": "references/def/side.png",
            "image_url": "/api/references/references/def/side.png",
            "kind": null,
            "description": null,
        })
    );
    for reference in &references[2..] {
        assert_eq!(reference["image_url"], Value::Null, "{reference}");
        assert_eq!(reference["kind"], "front");
    }
}

#[tokio::test]
async fn jobs_the_generator_records_show_up() {
    use crate::backend::generation::{GeneratedImage, GenerationRequest, Mode};
    use crate::backend::prompts::Overrides;

    let backend = TestBackend::new(true);
    let mut request = GenerationRequest::new("session-1", "ja", Mode::CharacterFront);
    request.reference_keys = vec!["design_source:references/x/y.png".to_owned()];
    let job = backend
        .state
        .generation
        .jobs
        .create(&request, &Overrides::default(), "fixture", "");
    job.succeed(&GeneratedImage {
        job_id: job.id(),
        url: "/api/generated/session-1/a/candidate-1.png".to_owned(),
        clean_path: std::path::PathBuf::new(),
        width: 800,
        height: 1100,
    });
    let (_, body) = call(
        &backend.app(),
        Method::GET,
        "/api/audit/generation-jobs",
        None,
    )
    .await;
    assert_eq!(body[0]["generation_mode"], "character_front");
    assert_eq!(body[0]["status"], "succeeded");
    assert_eq!(body[0]["references"][0]["kind"], "design_source");
    assert_eq!(
        body[0]["outputs"][0]["object_key"],
        "session-1/a/candidate-1.png"
    );
    let (_, summary) = call(&backend.app(), Method::GET, "/api/audit/summary", None).await;
    assert_eq!(summary["success_rate"], 1.0);
}

/// The screen never calls the old album and requirement routes, so they
/// were not ported and are unknown paths.
#[tokio::test]
async fn album_and_requirement_routes_are_not_served() {
    let backend = TestBackend::new(true);
    for uri in [
        "/api/album/items",
        "/api/prompts/requirements",
        "/api/prompts/chips",
    ] {
        let (status, _) = call(&backend.app(), Method::GET, uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

//! Ported from the old backend's `tests/test_settings.py` and
//! `tests/test_settings_probe.py`, plus the credential store.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;
use axum::routing::post;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::secrets::SecretStore;
use super::store::{self, FILE_NAME};
use crate::backend::router;
use crate::backend::tests::{TestBackend, authed, json_body};

fn backend() -> TestBackend {
    // Like the old test app, which allowed the fixtures.
    TestBackend::new(true)
}

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> Response {
    let request = match body {
        Some(body) => authed(method, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => authed(method, uri).body(Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

async fn read(app: &Router) -> Value {
    let response = call(app, Method::GET, "/api/settings", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

async fn put(app: &Router, values: Value) -> Response {
    call(
        app,
        Method::PUT,
        "/api/settings",
        Some(json!({ "values": values })),
    )
    .await
}

async fn body_text(response: Response) -> String {
    use http_body_util::BodyExt;
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn settings_report_effective_backends() {
    let body = read(&backend().app()).await;

    assert_eq!(body["values"]["llm_provider"], "fixture");
    assert_eq!(body["values"]["image_provider"], "fixture");
    assert_eq!(
        body["values"]["agent_llm_base_url"],
        "https://openrouter.ai/api/v1"
    );
    assert_eq!(
        body["values"]["openrouter_image_model"],
        "google/gemini-nano-banana-2.1"
    );
    assert_eq!(body["overridden"], json!([]));
    assert_eq!(
        body["options"]["image_provider"],
        json!(["codex", "openrouter", "siliconflow", "ark", "fixture"])
    );
    assert_eq!(
        body["options"]["agent_llm_provider"],
        json!(["openai_compatible", "claude_code"])
    );
    assert!(body["values"].get("ark_api_key").is_none());
    assert_eq!(
        body["secrets"]["ark_api_key"],
        json!({"set": false, "source": "", "hint": ""})
    );
    assert_eq!(body["secrets"].as_object().unwrap().len(), 5);
    let status = body["status"].as_object().unwrap();
    for flag in [
        "codex_cli",
        "claude_cli",
        "siliconflow_key",
        "ark_key",
        "openrouter_key",
        "agent_llm_key",
        "analysis_llm_key",
    ] {
        assert!(status[flag].is_boolean(), "{flag}");
    }
    assert_eq!(status["ark_key"], false);
}

#[tokio::test]
async fn settings_update_saves_changes_and_resets() {
    let backend = backend();
    let app = backend.app();

    let response = put(
        &app,
        json!({"image_provider": "ark", "agent_llm_provider": "claude_code", "agent_llm_model": " m1 "}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["values"]["image_provider"], "ark");
    assert_eq!(body["values"]["agent_llm_model"], "m1");
    assert_eq!(
        body["overridden"],
        json!(["agent_llm_model", "agent_llm_provider", "image_provider"])
    );
    assert_eq!(backend.state.settings.settings().image_provider(), "ark");
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(backend.dir.path().join(FILE_NAME)).unwrap())
            .unwrap();
    assert_eq!(saved["agent_llm_provider"], "claude_code");

    let body = json_body(put(&app, json!({"image_provider": null})).await).await;
    assert_eq!(body["values"]["image_provider"], "fixture");
    assert_eq!(
        backend.state.settings.settings().get("agent_llm_provider"),
        "claude_code"
    );

    let response = call(&app, Method::DELETE, "/api/settings", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["overridden"], json!([]));
    assert_eq!(
        backend.state.settings.settings().get("agent_llm_provider"),
        "openai_compatible"
    );
    assert!(!backend.dir.path().join(FILE_NAME).exists());
}

#[tokio::test]
async fn settings_reject_unknown_and_invalid_values() {
    let app = backend().app();
    for (values, key) in [
        (json!({"image_provider": "dalle"}), "image_provider"),
        (json!({"jwt_secret": "secret"}), "jwt_secret"),
        (
            json!({"agent_llm_model": "x".repeat(301)}),
            "agent_llm_model",
        ),
        (json!({"ark_api_key": "  "}), "ark_api_key"),
        (json!({"ark_api_key": "two words"}), "ark_api_key"),
    ] {
        let response = put(&app, values).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            json_body(response).await,
            json!({ "detail": format!("invalid_setting:{key}") })
        );
    }
    assert_eq!(read(&app).await["overridden"], json!([]));
}

#[tokio::test]
async fn an_invalid_value_saves_nothing_from_the_same_request() {
    let backend = backend();
    let app = backend.app();
    let response = put(
        &app,
        json!({"ark_api_key": "ark-secret-key-1234", "image_provider": "dalle"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(read(&app).await["overridden"], json!([]));
    assert!(backend.secrets.snapshot().is_empty());
}

#[tokio::test]
async fn a_body_of_the_wrong_shape_is_unprocessable() {
    let app = backend().app();
    let response = put(&app, json!({"image_provider": 3})).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(json_body(response).await["detail"].is_string());
}

#[tokio::test]
async fn fixtures_are_hidden_and_refused_when_not_allowed() {
    // Release builds: the old backend's production / no-fixture setup.
    let app = TestBackend::new(false).app();
    let body = read(&app).await;
    // Unset, analysis and images go to OpenRouter.
    assert_eq!(body["values"]["llm_provider"], "openai_compatible");
    assert_eq!(body["values"]["image_provider"], "openrouter");
    assert!(
        !body["options"]["image_provider"]
            .as_array()
            .unwrap()
            .contains(&json!("fixture"))
    );
    let response = put(&app, json!({"image_provider": "fixture"})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(put(&app, json!({"image_provider": "ark"})).await).await["values"]["image_provider"],
        "ark"
    );
}

#[test]
fn an_unreadable_settings_file_is_ignored() {
    let backend = backend();
    std::fs::write(backend.dir.path().join(FILE_NAME), "{not json").unwrap();
    let state = backend.restart();
    assert_eq!(state.settings.settings().image_provider(), "fixture");
    assert!(state.settings.snapshot().1.is_empty());
}

#[test]
fn the_settings_file_never_supplies_api_keys_or_invalid_values() {
    let backend = backend();
    std::fs::write(
        backend.dir.path().join(FILE_NAME),
        r#"{"ark_api_key": "ark-secret-key-1234", "image_provider": "dalle", "ark_image_model": " m2 ", "jwt_secret": "x"}"#,
    )
    .unwrap();
    let state = backend.restart();
    let (settings, saved) = state.settings.snapshot();
    assert_eq!(settings.get("ark_api_key"), "");
    assert_eq!(settings.image_provider(), "fixture");
    assert_eq!(settings.get("ark_image_model"), "m2");
    assert_eq!(saved.into_iter().collect::<Vec<_>>(), ["ark_image_model"]);
}

#[tokio::test]
async fn a_saved_codex_bridge_is_kept_but_can_no_longer_be_chosen() {
    let backend = backend();
    std::fs::write(
        backend.dir.path().join(FILE_NAME),
        r#"{"image_provider": "codex_bridge", "codex_bridge_url": "http://127.0.0.1:8765"}"#,
    )
    .unwrap();
    backend
        .secrets
        .set("codex_bridge_token", "old-bridge-token")
        .unwrap();
    let state = backend.restart();
    let settings = state.settings.settings();
    assert_eq!(settings.image_provider(), "codex_bridge");
    assert_eq!(settings.get("codex_bridge_url"), "");
    assert_eq!(settings.get("codex_bridge_token"), "");
    let error = crate::backend::generation::image_backend(&settings).unwrap_err();
    assert_eq!(error.code, Some("codex_bridge_removed"), "{error}");

    let app = router(state.clone());
    let body = read(&app).await;
    assert_eq!(body["values"]["image_provider"], "codex_bridge");
    assert!(body["values"].get("codex_bridge_url").is_none());
    assert!(body["secrets"].get("codex_bridge_token").is_none());
    assert!(body["status"].get("codex_bridge").is_none());
    let response = put(&app, json!({"image_provider": "codex_bridge"})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Saving something else keeps it until another backend is chosen.
    put(&app, json!({"ark_image_model": "m2"})).await;
    assert_eq!(
        backend.restart().settings.settings().image_provider(),
        "codex_bridge"
    );

    // A reset also removes the token earlier builds kept.
    call(&app, Method::DELETE, "/api/settings", None).await;
    assert!(backend.secrets.snapshot().is_empty());
}

#[tokio::test]
async fn api_keys_are_write_only_and_kept_in_the_credential_store() {
    let backend = backend();
    let app = backend.app();
    // The chat on Ark (the old default endpoint).
    let response = put(
        &app,
        json!({"agent_llm_base_url": "https://ark.cn-beijing.volces.com/api/plan/v3"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = put(&app, json!({"ark_api_key": " ark-secret-key-1234 "})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = body_text(response).await;
    assert!(!text.contains("ark-secret-key-1234"));
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        body["secrets"]["ark_api_key"],
        json!({"set": true, "source": "override", "hint": "…1234"})
    );
    assert_eq!(body["status"]["ark_key"], true);
    assert_eq!(
        body["overridden"],
        json!(["agent_llm_base_url", "ark_api_key"])
    );
    // The agent reuses the Ark key when it talks to the same endpoint.
    assert_eq!(body["status"]["agent_llm_key"], true);
    assert_eq!(
        backend.state.settings.settings().get("ark_api_key"),
        "ark-secret-key-1234"
    );

    // In the credential store, and not in the settings file.
    assert_eq!(
        backend.secrets.get("ark_api_key").unwrap().as_deref(),
        Some("ark-secret-key-1234")
    );
    let file = backend.dir.path().join(FILE_NAME);
    assert!(
        !file.exists()
            || !std::fs::read_to_string(&file)
                .unwrap()
                .contains("ark-secret")
    );

    // Still there after a restart.
    let restarted = backend.restart();
    assert_eq!(
        restarted.settings.settings().get("ark_api_key"),
        "ark-secret-key-1234"
    );

    let body = json_body(put(&app, json!({"ark_api_key": null})).await).await;
    assert_eq!(body["secrets"]["ark_api_key"]["set"], false);
    assert_eq!(backend.secrets.get("ark_api_key").unwrap(), None);
}

#[tokio::test]
async fn short_keys_get_no_hint() {
    let app = backend().app();
    let body = json_body(put(&app, json!({"openrouter_api_key": "sk-short"})).await).await;
    assert_eq!(
        body["secrets"]["openrouter_api_key"],
        json!({"set": true, "source": "override", "hint": ""})
    );
}

#[tokio::test]
async fn reset_also_removes_the_api_keys() {
    let backend = backend();
    let app = backend.app();
    put(
        &app,
        json!({"openrouter_api_key": "sk-or-test-00001111", "image_provider": "openrouter"}),
    )
    .await;
    assert_eq!(backend.secrets.snapshot().len(), 1);
    let body = json_body(call(&app, Method::DELETE, "/api/settings", None).await).await;
    assert_eq!(body["secrets"]["openrouter_api_key"]["set"], false);
    assert!(backend.secrets.snapshot().is_empty());
}

#[tokio::test]
async fn a_failing_credential_store_is_reported_and_changes_nothing() {
    let backend = backend();
    let app = backend.app();
    *backend.secrets.fail_with.lock().unwrap() = Some("locked".to_owned());

    let response = put(
        &app,
        json!({"ark_api_key": "ark-secret-key-1234", "image_provider": "ark"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let detail = json_body(response).await["detail"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(detail.starts_with("settings_not_saved: "), "{detail}");
    assert!(!detail.contains("ark-secret"));
    *backend.secrets.fail_with.lock().unwrap() = None;
    let body = read(&app).await;
    assert_eq!(body["overridden"], json!([]));
    assert_eq!(body["secrets"]["ark_api_key"]["set"], false);
}

#[tokio::test]
async fn openrouter_reuses_the_analysis_key_on_the_same_endpoint() {
    let app = backend().app();
    let body = json_body(
        put(
            &app,
            json!({
                "analysis_llm_base_url": "https://openrouter.ai/api/v1/",
                "analysis_llm_api_key": "sk-or-test-00001111",
            }),
        )
        .await,
    )
    .await;
    assert_eq!(body["status"]["openrouter_key"], true);
    assert_eq!(body["status"]["analysis_llm_key"], true);
    assert_eq!(body["status"]["agent_llm_key"], false);
}

#[tokio::test]
async fn extra_body_setting_must_be_a_json_object() {
    let app = backend().app();
    let response = put(&app, json!({"agent_llm_extra_body": "[1]"})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = put(&app, json!({"agent_llm_extra_body": ""})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["values"]["agent_llm_extra_body"],
        ""
    );
}

#[tokio::test]
async fn analysis_extra_body_setting_must_be_a_json_object() {
    let app = backend().app();
    let response = put(&app, json!({"analysis_llm_extra_body": "not json"})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = r#"{"reasoning": {"effort": "medium"}}"#;
    let response = put(&app, json!({"analysis_llm_extra_body": value})).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await["values"]["analysis_llm_extra_body"],
        value
    );
}

#[test]
fn every_secret_is_an_editable_field() {
    assert_eq!(store::secret_keys().count(), 5);
    assert!(store::valid_setting("ark_api_key", "sk-1"));
    assert!(!store::valid_setting("ark_api_key", ""));
}

// --- Connection test -------------------------------------------------------

type Handler = Arc<dyn Fn(&Value) -> (StatusCode, Value) + Send + Sync>;

/// A stand-in OpenAI-compatible API that records the request bodies.
struct FakeLlm {
    base_url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
    authorization: Arc<Mutex<Vec<Option<String>>>>,
}

async fn fake_llm(
    handler: impl Fn(&Value) -> (StatusCode, Value) + Send + Sync + 'static,
) -> FakeLlm {
    let handler: Handler = Arc::new(handler);
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let authorization = Arc::new(Mutex::new(Vec::new()));
    let (recorded, auth) = (bodies.clone(), authorization.clone());
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |request: Request| {
            let (handler, recorded, auth) = (handler.clone(), recorded.clone(), auth.clone());
            async move {
                use http_body_util::BodyExt;
                let (parts, body) = request.into_parts();
                let bytes = body.collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                auth.lock().unwrap().push(
                    parts
                        .headers
                        .get(header::AUTHORIZATION)
                        .map(|value| value.to_str().unwrap().to_owned()),
                );
                let (status, answer) = handler(&body);
                recorded.lock().unwrap().push(body);
                (status, axum::Json(answer))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeLlm {
        base_url,
        bodies,
        authorization,
    }
}

fn reply(message: Value) -> (StatusCode, Value) {
    (StatusCode::OK, json!({"choices": [{"message": message}]}))
}

fn tool_call() -> Value {
    json!({"content": "", "tool_calls": [{"id": "c1", "function": {"name": "ping", "arguments": "{\"ok\": true}"}}]})
}

/// The old probe tests' endpoint: GLM behind the chat assistant settings,
/// with Doubao's `thinking` extra field (the old default) and no analysis
/// fields of its own.
fn agent_backend(llm: &FakeLlm, extra: Value) -> TestBackend {
    let backend = backend();
    let mut changes: Vec<(String, Option<String>)> = vec![
        ("agent_llm_base_url".to_owned(), Some(llm.base_url.clone())),
        ("agent_llm_api_key".to_owned(), Some("key".to_owned())),
        ("agent_llm_model".to_owned(), Some("glm-5".to_owned())),
        (
            "agent_llm_extra_body".to_owned(),
            Some(r#"{"thinking": {"type": "disabled"}}"#.to_owned()),
        ),
        ("analysis_llm_extra_body".to_owned(), Some(String::new())),
    ];
    if let Value::Object(extra) = extra {
        changes.extend(
            extra
                .into_iter()
                .map(|(key, value)| (key, value.as_str().map(str::to_owned))),
        );
    }
    backend.state.settings.update(&changes).unwrap();
    backend
}

async fn probe(backend: &TestBackend, role: &str) -> (StatusCode, Value) {
    let app = router(backend.state.clone());
    let response = call(
        &app,
        Method::POST,
        &format!("/api/settings/probe/{role}"),
        None,
    )
    .await;
    let status = response.status();
    (status, json_body(response).await)
}

fn check_statuses(body: &Value) -> Vec<(String, String)> {
    body["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| {
            (
                check["id"].as_str().unwrap().to_owned(),
                check["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

const GLM_REJECTION: &str = "thinking.type `disabled` is not supported";

#[tokio::test]
async fn agent_probe_reports_dropped_extra_fields_and_tool_support() {
    let llm = fake_llm(|body| {
        if body.get("thinking").is_some() {
            (
                StatusCode::BAD_REQUEST,
                json!({"error": {"code": "InvalidParameter", "message": GLM_REJECTION}}),
            )
        } else {
            reply(tool_call())
        }
    })
    .await;
    let backend = agent_backend(&llm, json!({}));

    let (status, body) = probe(&backend, "agent").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["model"], "glm-5");
    assert_eq!(body["base_url"], llm.base_url);
    assert!(body["latency_ms"].is_u64());
    assert_eq!(
        check_statuses(&body),
        [
            ("connection".to_owned(), "ok".to_owned()),
            ("extra_body".to_owned(), "warn".to_owned()),
            ("tools".to_owned(), "ok".to_owned()),
        ]
    );
    assert_eq!(body["checks"][1]["detail"], "thinking");
    let bodies = llm.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[1]["tools"][0]["function"]["name"], "ping");
    assert_eq!(
        llm.authorization.lock().unwrap()[0].as_deref(),
        Some("Bearer key")
    );
}

#[tokio::test]
async fn agent_probe_flags_missing_tool_calls() {
    let llm = fake_llm(|_| reply(json!({"content": "<think>hm</think> pong"}))).await;
    let backend = agent_backend(&llm, json!({}));

    let (_, body) = probe(&backend, "agent").await;

    assert_eq!(body["ok"], false);
    assert_eq!(
        body["checks"].as_array().unwrap().last().unwrap(),
        &json!({"id": "tools", "status": "fail", "detail": "pong"})
    );
}

#[tokio::test]
async fn agent_probe_classifies_auth_errors() {
    let llm = fake_llm(|_| (StatusCode::UNAUTHORIZED, json!({"error": "invalid key"}))).await;
    let backend = agent_backend(&llm, json!({}));

    let (_, body) = probe(&backend, "agent").await;

    assert_eq!(body["ok"], false);
    assert_eq!(body["latency_ms"], Value::Null);
    assert_eq!(body["checks"][0]["status"], "fail");
    let detail = body["checks"][0]["detail"].as_str().unwrap();
    assert!(detail.starts_with("auth: HTTP 401"), "{detail}");
}

#[tokio::test]
async fn agent_probe_retries_a_busy_endpoint() {
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let llm = fake_llm(move |_| {
        let mut calls = counter.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            (StatusCode::SERVICE_UNAVAILABLE, json!({"error": "busy"}))
        } else {
            reply(tool_call())
        }
    })
    .await;
    let backend = agent_backend(&llm, json!({"agent_llm_extra_body": ""}));

    let (_, body) = probe(&backend, "agent").await;

    assert_eq!(body["ok"], true, "{body}");
    assert_eq!(*calls.lock().unwrap(), 2);
}

#[tokio::test]
async fn agent_probe_reports_an_unreachable_endpoint() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let backend = backend();
    backend
        .state
        .settings
        .update(&[(
            "agent_llm_base_url".to_owned(),
            Some(format!("http://127.0.0.1:{port}/v1")),
        )])
        .unwrap();

    let (_, body) = probe(&backend, "agent").await;

    let detail = body["checks"][0]["detail"].as_str().unwrap();
    assert!(
        detail.starts_with("unreachable: request failed"),
        "{detail}"
    );
}

#[tokio::test]
async fn agent_probe_needs_an_endpoint_and_model() {
    let backend = backend();
    backend
        .state
        .settings
        .update(&[("agent_llm_model".to_owned(), Some(String::new()))])
        .unwrap();

    let (status, body) = probe(&backend, "agent").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["checks"],
        json!([{"id": "connection", "status": "fail", "detail": "missing_endpoint"}])
    );
}

#[tokio::test]
async fn analysis_probe_sends_an_image() {
    let llm = fake_llm(|_| reply(json!({"content": "Red."}))).await;
    let backend = agent_backend(
        &llm,
        json!({"llm_provider": "openai_compatible", "agent_llm_extra_body": ""}),
    );

    let (_, body) = probe(&backend, "analysis").await;

    assert_eq!(body["ok"], true);
    assert_eq!(
        body["checks"],
        json!([
            {"id": "connection", "status": "ok", "detail": ""},
            {"id": "vision", "status": "ok", "detail": "Red."},
        ])
    );
    let bodies = llm.bodies.lock().unwrap();
    let image = bodies[0]["messages"][0]["content"][1]["image_url"]["url"]
        .as_str()
        .unwrap();
    assert!(image.starts_with("data:image/png;base64,"));
}

#[tokio::test]
async fn analysis_probe_warns_when_the_model_does_not_see_the_image() {
    let llm = fake_llm(|_| reply(json!({"content": "I cannot see images."}))).await;
    let backend = agent_backend(
        &llm,
        json!({"llm_provider": "openai_compatible", "agent_llm_extra_body": ""}),
    );

    let (_, body) = probe(&backend, "analysis").await;

    assert_eq!(body["ok"], true);
    assert_eq!(body["checks"][1]["status"], "warn");
}

#[tokio::test]
async fn probe_rejects_roles_that_are_not_openai_compatible() {
    // Analysis runs on the fixtures by default.
    let backend = backend();
    for role in ["analysis", "other"] {
        let (status, body) = probe(&backend, role).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body, json!({"detail": "probe_unsupported"}));
    }
    backend
        .state
        .settings
        .update(&[(
            "agent_llm_provider".to_owned(),
            Some("claude_code".to_owned()),
        )])
        .unwrap();
    assert_eq!(probe(&backend, "agent").await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn analysis_probe_sends_its_own_extra_body() {
    let llm = fake_llm(|_| reply(json!({"content": "Red."}))).await;
    let backend = agent_backend(
        &llm,
        json!({
            "llm_provider": "openai_compatible",
            // Another URL for the same fake server.
            "analysis_llm_base_url": llm.base_url.replace("127.0.0.1", "localhost"),
            "analysis_llm_extra_body": r#"{"reasoning": {"effort": "medium"}}"#,
        }),
    );

    let (_, body) = probe(&backend, "analysis").await;

    assert_eq!(body["ok"], true);
    let bodies = llm.bodies.lock().unwrap();
    assert_eq!(bodies[0]["reasoning"], json!({"effort": "medium"}));
    assert!(bodies[0].get("thinking").is_none());
}

#[tokio::test]
async fn analysis_does_not_borrow_the_agent_extra_body_on_another_url() {
    let llm = fake_llm(|_| reply(json!({"content": "Red."}))).await;
    let backend = agent_backend(&llm, json!({"llm_provider": "openai_compatible"}));
    // Point the agent elsewhere; analysis keeps the fake server.
    backend
        .state
        .settings
        .update(&[
            (
                "analysis_llm_base_url".to_owned(),
                Some(llm.base_url.clone()),
            ),
            (
                "agent_llm_base_url".to_owned(),
                Some("https://agent.invalid/v1".to_owned()),
            ),
            ("analysis_llm_api_key".to_owned(), Some("key".to_owned())),
        ])
        .unwrap();

    let (_, body) = probe(&backend, "analysis").await;

    assert_eq!(check_statuses(&body).len(), 2, "{body}");
    assert!(llm.bodies.lock().unwrap()[0].get("thinking").is_none());
}

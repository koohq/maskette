use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;
use axum::routing::any;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::chat_api::ChatApi;
use super::relay::Relay;
use super::settings::secrets::MemorySecrets;
use super::settings::store::SettingsStore;
use super::token::{self, Token};
use super::{AppState, Backend, SharedState, router};

pub const TOKEN: &str = "test-token-0123456789abcdef";

/// A backend on a temporary data folder, with API keys kept in memory.
pub struct TestBackend {
    pub dir: tempfile::TempDir,
    pub secrets: Arc<MemorySecrets>,
    pub state: SharedState,
}

impl TestBackend {
    pub fn new(relay: Option<Relay>, allow_fixture: bool) -> Self {
        let dir = tempfile::tempdir().expect("temporary data folder");
        let secrets = Arc::new(MemorySecrets::default());
        let state = Self::state(dir.path(), &secrets, relay, allow_fixture);
        Self {
            dir,
            secrets,
            state,
        }
    }

    fn state(
        data_dir: &std::path::Path,
        secrets: &Arc<MemorySecrets>,
        relay: Option<Relay>,
        allow_fixture: bool,
    ) -> SharedState {
        let store = SettingsStore::open(data_dir, secrets.clone(), allow_fixture);
        Arc::new(AppState {
            token: Token::from_static(TOKEN),
            data_dir: data_dir.to_owned(),
            relay,
            settings: Arc::new(store),
            // Retries must not wait in tests.
            chat: ChatApi::with_retry_delays(vec![std::time::Duration::ZERO; 3]),
            agent: super::agent::Agent::new(data_dir),
            generation_log: Default::default(),
        })
    }

    /// The same data folder and credential store, read again as on a restart.
    pub fn restart(&self) -> SharedState {
        Self::state(
            self.dir.path(),
            &self.secrets,
            None,
            self.state.settings.settings().allow_fixture,
        )
    }

    pub fn app(&self) -> Router {
        router(self.state.clone())
    }
}

fn app(relay: Option<Relay>) -> Router {
    TestBackend::new(relay, true).app()
}

fn get(uri: &str) -> axum::http::request::Builder {
    Request::builder().method(Method::GET).uri(uri)
}

/// A request that carries the token.
pub fn authed(method: Method, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
}

pub async fn json_body(response: Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("json body")
}

#[tokio::test]
async fn health_answers_with_the_bearer_token() {
    let response = app(None)
        .oneshot(
            get("/api/health")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["app"], "maskette");
    assert_eq!(body["legacy_relay"], false);
}

#[tokio::test]
async fn health_answers_with_the_query_token() {
    let uri = format!("/api/health?{}={TOKEN}", token::QUERY_PARAM);
    let response = app(None)
        .oneshot(get(&uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn requests_without_the_token_are_refused() {
    for request in [
        get("/api/health").body(Body::empty()).unwrap(),
        get("/api/health")
            .header(header::AUTHORIZATION, "Bearer wrong")
            .body(Body::empty())
            .unwrap(),
        get("/api/health?maskette_token=wrong")
            .body(Body::empty())
            .unwrap(),
        get("/api/not-ported").body(Body::empty()).unwrap(),
    ] {
        let response = app(None).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            json_body(response).await["detail"],
            "Missing or invalid Maskette token"
        );
    }
}

#[tokio::test]
async fn cors_preflight_from_the_ui_passes_without_the_token() {
    let request = Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/health")
        .header(header::ORIGIN, "http://tauri.localhost")
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "authorization")
        .body(Body::empty())
        .unwrap();
    let response = app(None).oneshot(request).await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://tauri.localhost"
    );
}

#[tokio::test]
async fn other_origins_get_no_cors_grant() {
    let request = get("/api/health")
        .header(header::ORIGIN, "https://example.com")
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = app(None).oneshot(request).await.unwrap();
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
}

#[tokio::test]
async fn responses_opt_in_to_cross_origin_embedding() {
    let response = app(None)
        .oneshot(get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.headers()["cross-origin-resource-policy"],
        "cross-origin"
    );
}

#[tokio::test]
async fn unported_api_paths_say_so_when_the_relay_is_off() {
    let response = app(None)
        .oneshot(
            get("/api/generation/jobs")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    let response = app(None)
        .oneshot(
            get("/elsewhere")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A stand-in for the old Python backend that echoes what it received.
pub async fn spawn_legacy_echo() -> String {
    async fn echo(request: Request) -> axum::Json<Value> {
        let (parts, body) = request.into_parts();
        let body: Bytes = body.collect().await.expect("body").to_bytes();
        axum::Json(json!({
            "method": parts.method.as_str(),
            "path": parts.uri.path(),
            "query": parts.uri.query(),
            "authorization": parts.headers.get(header::AUTHORIZATION).map(|v| v.to_str().unwrap().to_owned()),
            "content_type": parts.headers.get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_owned()),
            "body": String::from_utf8_lossy(&body),
        }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(any(echo)))
            .await
            .unwrap();
    });
    base
}

#[tokio::test]
async fn relay_forwards_unported_routes_without_the_token() {
    let legacy = spawn_legacy_echo().await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!(
            "/api/settings/probe?role=agent&{}={TOKEN}",
            token::QUERY_PARAM
        ))
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "11")
        .body(Body::from("{\"a\":true}\n"))
        .unwrap();
    let response = app(Some(Relay::new(&legacy).unwrap()))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["cross-origin-resource-policy"],
        "cross-origin"
    );
    let echoed = json_body(response).await;
    assert_eq!(echoed["method"], "POST");
    assert_eq!(echoed["path"], "/api/settings/probe");
    assert_eq!(echoed["query"], "role=agent");
    assert_eq!(echoed["authorization"], Value::Null);
    assert_eq!(echoed["content_type"], "application/json");
    assert_eq!(echoed["body"], "{\"a\":true}\n");
}

#[tokio::test]
async fn relay_reports_an_unreachable_legacy_api() {
    // Take a free port, then close it again.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let relay = Relay::new(&format!("http://127.0.0.1:{port}/")).unwrap();
    let response = app(Some(relay))
        .oneshot(
            get("/api/generation/jobs")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}

#[test]
fn relay_accepts_only_http_urls() {
    assert!(Relay::new("http://127.0.0.1:18000/").is_ok());
    assert!(Relay::new("ftp://127.0.0.1").is_err());
    assert!(Relay::new("not a url").is_err());
}

#[test]
fn token_query_parameter_is_stripped() {
    let name = token::QUERY_PARAM;
    assert_eq!(token::strip_from_query(&format!("{name}=abc")), None);
    assert_eq!(
        token::strip_from_query(&format!("a=1&{name}=abc&b=2")).as_deref(),
        Some("a=1&b=2")
    );
    assert_eq!(
        token::strip_from_query("after=3").as_deref(),
        Some("after=3")
    );
}

#[test]
fn tokens_are_long_random_hex() {
    let a = Token::generate().unwrap();
    let b = Token::generate().unwrap();
    assert_eq!(a.as_str().len(), 64);
    assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a.as_str(), b.as_str());
}

#[test]
fn init_script_guards_the_token_by_origin() {
    let backend = Backend {
        port: 4321,
        token: Token::from_static(TOKEN),
    };
    let script = backend.init_script();
    assert!(script.contains("\"baseUrl\":\"http://127.0.0.1:4321\""));
    assert!(script.contains("window.location.origin"));
    assert!(script.contains("tauri://localhost"));
}

#[tokio::test]
async fn backend_serves_on_a_random_local_port() {
    let dir = tempfile::tempdir().unwrap();
    // `Backend::start` spawns on the Tauri runtime, which also works in tests.
    let backend = Backend::start(super::BackendConfig {
        data_dir: dir.path().to_owned(),
        legacy_api_url: None,
        secrets: Arc::new(MemorySecrets::default()),
    })
    .unwrap();
    let url = format!("{}/api/health", backend.base_url());
    let client = reqwest::Client::new();
    let refused = client.get(&url).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let ok = client
        .get(&url)
        .bearer_auth(backend.token.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

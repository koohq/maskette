use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::Response;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use super::chat_api::ChatApi;
use super::settings::secrets::MemorySecrets;
use super::settings::store::SettingsStore;
use super::token::{self, Token};
use super::{AppState, Backend, SharedState, router};

pub const TOKEN: &str = "test-token-0123456789abcdef";

/// Tests never start the real CLIs (they would use the user's
/// subscription): every run answers as if the CLI were not installed.
pub struct NoCli;

impl super::cli::Runner for NoCli {
    fn run(&self, _invocation: super::cli::Invocation) -> super::cli::RunFuture<'_> {
        Box::pin(async { Err(super::cli::RunError::NotFound) })
    }
}

/// A backend on a temporary data folder, with API keys kept in memory.
pub struct TestBackend {
    pub dir: tempfile::TempDir,
    pub secrets: Arc<MemorySecrets>,
    pub state: SharedState,
}

impl TestBackend {
    pub fn new(allow_fixture: bool) -> Self {
        Self::with_cli(allow_fixture, super::cli::Cli::with_runner(Arc::new(NoCli)))
    }

    /// A backend whose CLI runs go to `cli` (usually a stand-in runner).
    pub fn with_cli(allow_fixture: bool, cli: super::cli::Cli) -> Self {
        let dir = tempfile::tempdir().expect("temporary data folder");
        let secrets = Arc::new(MemorySecrets::default());
        let state = Self::state(dir.path(), &secrets, allow_fixture, cli);
        Self {
            dir,
            secrets,
            state,
        }
    }

    fn state(
        data_dir: &std::path::Path,
        secrets: &Arc<MemorySecrets>,
        allow_fixture: bool,
        cli: super::cli::Cli,
    ) -> SharedState {
        let store = SettingsStore::open(data_dir, secrets.clone(), allow_fixture);
        Arc::new(AppState {
            token: Token::from_static(TOKEN),
            data_dir: data_dir.to_owned(),
            settings: Arc::new(store),
            // Retries must not wait in tests.
            chat: ChatApi::with_retry_delays(vec![std::time::Duration::ZERO; 3]),
            agent: super::agent::Agent::new(data_dir),
            generation: super::generation::Services::with_retry_delays(
                data_dir,
                vec![std::time::Duration::ZERO; 3],
            ),
            audit: super::audit::Audit::default(),
            cli,
            model_lists: Default::default(),
            generation_log: Default::default(),
        })
    }

    /// The same data folder and credential store, read again as on a restart.
    pub fn restart(&self) -> SharedState {
        Self::state(
            self.dir.path(),
            &self.secrets,
            self.state.settings.settings().allow_fixture,
            self.state.cli.clone(),
        )
    }

    pub fn app(&self) -> Router {
        router(self.state.clone())
    }
}

fn app() -> Router {
    TestBackend::new(true).app()
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

// --- The real OpenRouter, for the ignored side-by-side runs -------------------

/// OpenRouter's API, as the settings' defaults name it.
pub const OPENROUTER: &str = "https://openrouter.ai/api/v1";

/// The OpenRouter key the app keeps in the OS credential store, for the
/// ignored runs against the real API (`generation/compare.rs`,
/// `agent/trial.rs`): the service's key, or one an earlier build saved for a
/// role that the app has not moved to its service yet (assumed OpenRouter's).
pub fn saved_openrouter_key() -> String {
    use super::settings::secrets::{KeyringSecrets, SERVICE, SecretStore as _};
    use super::settings::store::{KEY_FIELDS, key_entry, service_of};
    let secrets = KeyringSecrets::new(SERVICE);
    std::iter::once(key_entry(&service_of(OPENROUTER)))
        .chain(KEY_FIELDS[..3].iter().map(|name| (*name).to_owned()))
        .find_map(|name| secrets.get(&name).expect("credential store readable"))
        .expect("an OpenRouter key saved in Maskette's settings")
}

/// What `key` has spent on OpenRouter so far, in US dollars. A failed read
/// is tried again a few times: on 2026-11-19 one read lost its connection,
/// and the panic threw away the record of a run that was already billed.
async fn openrouter_usage(client: &reqwest::Client, key: &str) -> f64 {
    const ATTEMPTS: u32 = 4;
    let mut attempt = 1;
    let reply = loop {
        let read: Result<Value, reqwest::Error> = async {
            client
                .get(format!("{OPENROUTER}/key"))
                .bearer_auth(key)
                .send()
                .await?
                .json()
                .await
        }
        .await;
        match read {
            Ok(reply) => break reply,
            Err(error) if attempt < ATTEMPTS => {
                println!("reading the key's spending failed ({error}); trying again");
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            }
            Err(error) => panic!("OpenRouter's key status unreadable: {error}"),
        }
    };
    reply["data"]["usage"]
        .as_f64()
        .unwrap_or_else(|| panic!("no usage in {reply}"))
}

/// What a run billed `key`, read as the change of its spending. OpenRouter
/// counts a call some time after it ends, sometimes in parts (four seconds
/// was not enough on 2026-11-19: a run's cost landed in the next run's), so
/// this waits until the spending has stayed the same for 20 seconds, for
/// three minutes at most. Nothing else may use the key meanwhile.
pub struct Spending {
    client: reqwest::Client,
    key: String,
    before: f64,
}

impl Spending {
    pub async fn start(key: &str) -> Self {
        let client = reqwest::Client::new();
        let before = openrouter_usage(&client, key).await;
        Self {
            client,
            key: key.to_owned(),
            before,
        }
    }

    /// The dollars spent since `start`, rounded to a ten-thousandth.
    pub async fn spent(&self) -> f64 {
        let pause = std::time::Duration::from_secs(5);
        tokio::time::sleep(2 * pause).await;
        let mut last = openrouter_usage(&self.client, &self.key).await;
        let mut unchanged = 0;
        for _ in 0..34 {
            tokio::time::sleep(pause).await;
            let now = openrouter_usage(&self.client, &self.key).await;
            if (now - last).abs() < 1e-9 {
                unchanged += 1;
                if unchanged >= 4 && now > self.before {
                    break;
                }
            } else {
                unchanged = 0;
            }
            last = now;
        }
        ((last - self.before) * 10_000.0).round() / 10_000.0
    }
}

/// `vendor/model@effort` as the model and the reasoning effort (none
/// without an `@`).
pub fn model_and_effort(spec: &str) -> (String, Option<String>) {
    match spec.trim().rsplit_once('@') {
        Some((model, effort)) => (model.trim().to_owned(), Some(effort.trim().to_owned())),
        None => (spec.trim().to_owned(), None),
    }
}

/// The extra request fields that ask for `effort`, in the settings' format;
/// without one, an empty object (an empty setting would fall back to
/// another role's fields).
pub fn effort_body(effort: Option<&str>) -> String {
    effort.map_or_else(
        || "{}".to_owned(),
        |effort| serde_json::json!({"reasoning": {"effort": effort}}).to_string(),
    )
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
    let response = app()
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
}

#[tokio::test]
async fn health_answers_with_the_query_token() {
    let uri = format!("/api/health?{}={TOKEN}", token::QUERY_PARAM);
    let response = app()
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
        let response = app().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(response).await["detail"], "token_rejected");
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
    let response = app().oneshot(request).await.unwrap();
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
    let response = app().oneshot(request).await.unwrap();
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
}

#[tokio::test]
async fn responses_opt_in_to_cross_origin_embedding() {
    let response = app()
        .oneshot(get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.headers()["cross-origin-resource-policy"],
        "cross-origin"
    );
}

#[tokio::test]
async fn unknown_paths_are_not_found() {
    let response = app()
        .oneshot(
            get("/api/generation/jobs")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(response).await["detail"], "Not Found");
    let response = app()
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

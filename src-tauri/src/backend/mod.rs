//! The local HTTP API the UI talks to.
//!
//! An axum server on 127.0.0.1 with a random port, guarded by a token made
//! fresh on every launch. It serves the same `/api/...` paths as the old
//! Python backend, so routers can move over one at a time; until a router is
//! ported, its requests can be relayed to the old backend (see [`relay`]).

pub mod agent;
pub mod chat_api;
#[cfg(debug_assertions)]
pub mod dev_handoff;
mod error;
pub mod generation;
mod health;
mod ids;
pub mod references;
pub mod relay;
mod safety;
pub mod settings;
pub mod token;

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::header::{AUTHORIZATION, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, Method};
use axum::middleware;
use axum::routing::get;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use chat_api::ChatApi;
use relay::Relay;
use settings::secrets::SecretStore;
use settings::store::SettingsStore;
use token::Token;

/// Origins the UI is served from: the Vite dev server in debug builds, and
/// the bundled frontend (`tauri://localhost`, or `http(s)://tauri.localhost`
/// on Windows).
pub fn ui_origins() -> Vec<&'static str> {
    let mut origins = vec![
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ];
    if cfg!(debug_assertions) {
        origins.extend(["http://127.0.0.1:5173", "http://localhost:5173"]);
    }
    origins
}

pub struct BackendConfig {
    /// Where Maskette keeps its data (the old backend's `runtime/`).
    pub data_dir: PathBuf,
    /// Base URL of the old Python backend; `None` turns the relay off.
    pub legacy_api_url: Option<String>,
    /// Where API keys are kept (the OS credential store outside tests).
    pub secrets: Arc<dyn SecretStore>,
}

pub struct AppState {
    pub token: Token,
    pub data_dir: PathBuf,
    pub relay: Option<Relay>,
    pub settings: Arc<SettingsStore>,
    pub chat: ChatApi,
    pub agent: agent::Agent,
    pub generation: generation::Services,
    /// Every generation request, by job id, for the tests to inspect.
    #[cfg(test)]
    pub generation_log: std::sync::Mutex<Vec<(String, generation::GenerationRequest)>>,
}

pub type SharedState = Arc<AppState>;

/// A running backend: where it listens and the token it expects.
pub struct Backend {
    pub port: u16,
    pub token: Token,
}

impl Backend {
    /// Bind 127.0.0.1 on a free port and serve on the Tauri async runtime.
    pub fn start(config: BackendConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = Token::generate()?;

        let relay = match config.legacy_api_url {
            Some(url) => match Relay::new(&url) {
                Ok(relay) => {
                    log::info!("relaying unported API routes to {}", relay.base_url());
                    Some(relay)
                }
                Err(error) => {
                    log::error!("legacy API relay is off: {error}");
                    None
                }
            },
            None => None,
        };
        // Fixture backends are for development only.
        let settings =
            SettingsStore::open(&config.data_dir, config.secrets, cfg!(debug_assertions));
        let state = Arc::new(AppState {
            token: token.clone(),
            agent: agent::Agent::new(&config.data_dir),
            generation: generation::Services::new(&config.data_dir),
            data_dir: config.data_dir,
            relay,
            settings: Arc::new(settings),
            chat: ChatApi::new(),
            #[cfg(test)]
            generation_log: Default::default(),
        });
        let app = router(state);

        tauri::async_runtime::spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    log::error!("backend listener failed: {error}");
                    return;
                }
            };
            if let Err(error) = axum::serve(listener, app).await {
                log::error!("backend server stopped: {error}");
            }
        });

        Ok(Self { port, token })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Script run in the window before the page's own scripts. It hands the
    /// backend's address and token to the UI, but only on Maskette's own
    /// origins, so a page the window navigates to elsewhere never sees them.
    pub fn init_script(&self) -> String {
        let info = serde_json::json!({ "baseUrl": self.base_url(), "token": self.token.as_str() });
        let origins = serde_json::json!(ui_origins());
        format!(
            "if ({origins}.includes(window.location.origin)) {{\
             Object.defineProperty(window, \"__MASKETTE_BACKEND__\", {{ value: Object.freeze({info}) }});\
             }}"
        )
    }
}

pub fn router(state: SharedState) -> Router {
    let origins: Vec<HeaderValue> = ui_origins()
        .into_iter()
        .map(HeaderValue::from_static)
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .expose_headers([CONTENT_DISPOSITION])
        .allow_private_network(true)
        .max_age(Duration::from_secs(600));

    Router::new()
        .route("/api/health", get(health::health))
        .merge(settings::routes())
        .merge(references::routes())
        .merge(agent::routes())
        .merge(generation::routes())
        .fallback(relay::fallback)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            token::require_token,
        ))
        .with_state(state)
        // The UI is cross-origin isolated (COEP: require-corp); plain <img>
        // loads from this server need an explicit CORP opt-in.
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("cross-origin"),
        ))
        // Outermost, so CORS preflights are answered before the token check.
        .layer(cors)
}

/// A known path with another method, in the old backend's shape.
async fn method_not_allowed() -> error::ApiError {
    error::ApiError::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "Method Not Allowed",
    )
}

#[cfg(test)]
mod tests;

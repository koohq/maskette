//! Relay for API routes not yet ported to Rust.
//!
//! While routers move over one at a time, any `/api/...` request the Rust
//! server does not handle itself can be passed to the old Python backend
//! (`kig-craft-lab`), so the whole app keeps working and old and new
//! behaviour can be compared. Set `MASKETTE_LEGACY_API_URL` (for example
//! `http://127.0.0.1:18000`) to turn it on; without it the relay is off.
//! Once every router is ported, this module goes away.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, HOST, ORIGIN, TRANSFER_ENCODING};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{SharedState, token};

pub const ENV_VAR: &str = "MASKETTE_LEGACY_API_URL";

/// Reads [`ENV_VAR`]; an empty value counts as unset.
pub fn legacy_api_url_from_env() -> Option<String> {
    std::env::var(ENV_VAR)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub struct Relay {
    client: reqwest::Client,
    base_url: String,
}

impl Relay {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let base_url = base_url.trim().trim_end_matches('/');
        let parsed = reqwest::Url::parse(base_url)
            .map_err(|error| format!("invalid {ENV_VAR} {base_url:?}: {error}"))?;
        if parsed.scheme() != "http" || parsed.host_str().is_none() {
            return Err(format!(
                "{ENV_VAR} must be an http:// URL, got {base_url:?}"
            ));
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // Pass redirects back to the UI instead of following them here.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| format!("cannot build the relay client: {error}"))?;
        Ok(Self {
            client,
            base_url: base_url.to_owned(),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Forward the request as-is (the old backend serves the same `/api/...`
    /// paths) and stream the answer back, so uploads and server-sent events
    /// pass through. The Maskette token stays on this side.
    pub async fn forward(&self, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let path = parts.uri.path();
        let url = match parts.uri.query().and_then(token::strip_from_query) {
            Some(query) => format!("{}{path}?{query}", self.base_url),
            None => format!("{}{path}", self.base_url),
        };

        let has_body = parts.headers.contains_key(CONTENT_LENGTH)
            || parts.headers.contains_key(TRANSFER_ENCODING);
        let mut headers = parts.headers;
        strip_hop_by_hop(&mut headers);
        for name in [HOST, AUTHORIZATION, ORIGIN] {
            headers.remove(name);
        }

        let mut upstream = self.client.request(parts.method, &url).headers(headers);
        if has_body {
            upstream = upstream.body(reqwest::Body::wrap_stream(body.into_data_stream()));
        }

        match upstream.send().await {
            Ok(answer) => {
                let status = answer.status();
                let mut headers = answer.headers().clone();
                strip_hop_by_hop(&mut headers);
                // This server sets its own CORS headers.
                let cors: Vec<_> = headers
                    .keys()
                    .filter(|name| name.as_str().starts_with("access-control-"))
                    .cloned()
                    .collect();
                for name in cors {
                    headers.remove(name);
                }
                let mut response = Response::new(Body::from_stream(answer.bytes_stream()));
                *response.status_mut() = status;
                *response.headers_mut() = headers;
                response
            }
            Err(error) => {
                log::warn!("legacy API relay failed for {path}: {error}");
                detail(
                    StatusCode::BAD_GATEWAY,
                    format!("The legacy API at {} did not answer", self.base_url),
                )
            }
        }
    }
}

/// Fallback for every path the Rust server does not handle itself.
pub async fn fallback(State(state): State<SharedState>, request: Request) -> Response {
    let path = request.uri().path();
    let is_api = path == "/api" || path.starts_with("/api/");
    match &state.relay {
        Some(relay) if is_api => relay.forward(request).await,
        _ if is_api => detail(
            StatusCode::NOT_IMPLEMENTED,
            format!("{path} is not in Maskette yet; set {ENV_VAR} to relay it to the legacy API"),
        ),
        _ => detail(StatusCode::NOT_FOUND, "Not Found".to_owned()),
    }
}

/// Error body in the old backend's shape (`{"detail": ...}`), which the UI reads.
fn detail(status: StatusCode, detail: String) -> Response {
    (status, axum::Json(serde_json::json!({ "detail": detail }))).into_response()
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

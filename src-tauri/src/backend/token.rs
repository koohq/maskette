//! The per-launch token every API request must carry.
//!
//! `fetch` sends it as `Authorization: Bearer <token>`. URLs the browser
//! loads by itself (`<img>`, `EventSource`, download links) cannot set
//! headers, so they carry it as the `maskette_token` query parameter.

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::SharedState;

pub const QUERY_PARAM: &str = "maskette_token";

#[derive(Clone)]
pub struct Token(String);

impl Token {
    /// 32 random bytes from the OS, as 64 hex characters.
    pub fn generate() -> std::io::Result<Self> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(std::io::Error::other)?;
        Ok(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    #[cfg(test)]
    pub fn from_static(value: &str) -> Self {
        Self(value.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compare without returning early, so timing does not leak the token.
    fn matches(&self, candidate: &str) -> bool {
        let expected = self.0.as_bytes();
        let candidate = candidate.as_bytes();
        expected.len() == candidate.len()
            && expected
                .iter()
                .zip(candidate)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                == 0
    }

    pub fn accepts(&self, request: &Request) -> bool {
        let from_header = request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if let Some(candidate) = from_header {
            return self.matches(candidate.trim());
        }
        request
            .uri()
            .query()
            .and_then(query_token)
            .is_some_and(|candidate| self.matches(candidate))
    }
}

fn query_token(query: &str) -> Option<&str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(QUERY_PARAM)?.strip_prefix('='))
}

pub async fn require_token(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    if state.token.accepts(&request) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "detail": "Missing or invalid Maskette token" })),
        )
            .into_response()
    }
}

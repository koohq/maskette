use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};

use super::SharedState;

/// Liveness check for the UI and for tests.
pub async fn health(State(state): State<SharedState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "app": "maskette",
        "version": env!("CARGO_PKG_VERSION"),
        "legacy_relay": state.relay.is_some(),
    }))
}

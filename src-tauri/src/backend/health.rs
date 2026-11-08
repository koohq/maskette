use axum::Json;
use serde_json::{Value, json};

/// Liveness check for the UI and for tests.
pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "app": "maskette",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

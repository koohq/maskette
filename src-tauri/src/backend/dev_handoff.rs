//! Debug builds only: tell the Vite dev server where the backend is.
//!
//! The Tauri window gets the backend's address and token injected, but a
//! plain browser opened on the dev server does not. For that case the
//! running app writes `.maskette-dev/backend.json` at the repository root,
//! and the dev server's `/api` proxy reads it and adds the token. The file
//! is git-ignored and removed when the app exits.

use std::path::PathBuf;

use super::Backend;

fn handoff_path() -> PathBuf {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(std::path::Path::new("."));
    repo_root.join(".maskette-dev").join("backend.json")
}

pub fn write(backend: &Backend) {
    let path = handoff_path();
    let body = serde_json::json!({ "port": backend.port, "token": backend.token.as_str() });
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, body.to_string()));
    if let Err(error) = result {
        log::warn!("could not write the dev backend handoff file: {error}");
    }
}

pub fn remove() {
    let _ = std::fs::remove_file(handoff_path());
}

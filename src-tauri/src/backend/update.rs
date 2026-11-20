//! Updates to Maskette itself, from its releases on GitHub.
//!
//! The Tauri updater plugin does the work (the endpoint and the public key
//! are in `tauri.conf.json`); [`Updater`] is the part of it these routes
//! need, so tests can stand in for it. Debug builds have none: they would
//! replace the installed app with a release, so the routes say updates are
//! not supported and the screen shows nothing.
//!
//! - `GET /api/update`: whether updates are supported, the running version
//!   and how an install is going. No network.
//! - `POST /api/update/check`: asks the release server for a newer version.
//! - `POST /api/update/install`: downloads the version the last check found
//!   and runs its installer. On Windows the app exits as the installer
//!   starts; the installer opens it again when it is done.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use super::SharedState;
use super::error::ApiError;

/// A newer version the release server offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    pub version: String,
    /// The release notes, as written in `latest.json`.
    pub notes: Option<String>,
    /// When it was published, in seconds since 1970.
    pub published_at: Option<i64>,
}

pub type CheckFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Release>, String>> + Send + 'a>>;
pub type InstallFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Called as the download goes: bytes so far, and the total when known.
pub type Progress = Box<dyn FnMut(u64, Option<u64>) + Send>;

pub trait Updater: Send + Sync {
    /// Asks the release server; `Ok(None)` when this is the newest version.
    fn check(&self) -> CheckFuture<'_>;
    /// Downloads `version`, which the last check must have found, checks
    /// its signature and runs its installer. On Windows the app exits as
    /// the installer starts, so this returns only when something failed.
    fn install(&self, version: String, progress: Progress) -> InstallFuture<'_>;
}

/// How an install is going.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum InstallState {
    #[default]
    Idle,
    Downloading {
        downloaded: u64,
        total: Option<u64>,
    },
    /// Downloaded; the installer is starting and the app is about to exit.
    Installing,
    Failed {
        detail: String,
    },
}

/// The updater (none in debug builds and tests), the running version and
/// the install under way.
pub struct Updates {
    updater: Option<Arc<dyn Updater>>,
    /// The app's version as the updater compares it: `tauri.conf.json`'s,
    /// which a build can override (`--config`), so not always Cargo's.
    current_version: String,
    install: Arc<Mutex<InstallState>>,
}

impl Default for Updates {
    fn default() -> Self {
        Self::new(None, env!("CARGO_PKG_VERSION"))
    }
}

impl Updates {
    pub fn new(updater: Option<Arc<dyn Updater>>, current_version: impl Into<String>) -> Self {
        Self {
            updater,
            current_version: current_version.into(),
            install: Default::default(),
        }
    }

    fn install_state(&self) -> InstallState {
        self.install.lock().expect("install state").clone()
    }
}

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/update", get(status))
        .route("/api/update/check", post(check))
        .route("/api/update/install", post(install))
}

#[derive(Serialize)]
struct Status {
    supported: bool,
    current_version: String,
    install: InstallState,
}

fn status_of(updates: &Updates) -> Status {
    Status {
        supported: updates.updater.is_some(),
        current_version: updates.current_version.clone(),
        install: updates.install_state(),
    }
}

async fn status(State(state): State<SharedState>) -> Json<Status> {
    Json(status_of(&state.updates))
}

/// A check that got no answer the updater could read (no network, no
/// release yet, a broken `latest.json`); the updater's words follow it.
const CHECK_FAILED: &str = "update_check_failed";

#[derive(Serialize)]
struct CheckReply {
    supported: bool,
    current_version: String,
    update: Option<Release>,
}

async fn check(State(state): State<SharedState>) -> Result<Json<CheckReply>, ApiError> {
    let update = match &state.updates.updater {
        Some(updater) => updater.check().await.map_err(|detail| {
            log::warn!("update check failed: {detail}");
            ApiError::new(StatusCode::BAD_GATEWAY, format!("{CHECK_FAILED}: {detail}"))
        })?,
        None => None,
    };
    Ok(Json(CheckReply {
        supported: state.updates.updater.is_some(),
        current_version: state.updates.current_version.clone(),
        update,
    }))
}

#[derive(Deserialize)]
struct InstallRequest {
    version: String,
}

async fn install(
    State(state): State<SharedState>,
    body: Result<Json<InstallRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Status>), ApiError> {
    let Json(request) = body?;
    let Some(updater) = state.updates.updater.clone() else {
        return Err(ApiError::bad_request("update_unsupported"));
    };
    {
        let mut install = state.updates.install.lock().expect("install state");
        if matches!(
            *install,
            InstallState::Downloading { .. } | InstallState::Installing
        ) {
            return Err(ApiError::new(StatusCode::CONFLICT, "update_in_progress"));
        }
        *install = InstallState::Downloading {
            downloaded: 0,
            total: None,
        };
    }

    let shared = state.updates.install.clone();
    tokio::spawn(async move {
        let progress_state = shared.clone();
        let progress: Progress = Box::new(move |downloaded, total| {
            let mut install = progress_state.lock().expect("install state");
            *install = if total.is_some_and(|total| downloaded >= total) {
                InstallState::Installing
            } else {
                InstallState::Downloading { downloaded, total }
            };
        });
        let result = updater.install(request.version.clone(), progress).await;
        let mut install = shared.lock().expect("install state");
        *install = match result {
            // Only where the installer does not end the app (tests).
            Ok(()) => InstallState::Installing,
            Err(detail) => {
                log::warn!("update to {} failed: {detail}", request.version);
                InstallState::Failed { detail }
            }
        };
    });

    Ok((StatusCode::ACCEPTED, Json(status_of(&state.updates))))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{Method, StatusCode, header};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::{CheckFuture, InstallFuture, Progress, Release, Updater, Updates};
    use crate::backend::tests::{TestBackend, authed, json_body};

    /// Offers `offer`, and on install reports two chunks of a 10-byte
    /// download, then answers with `install`.
    struct FakeUpdater {
        offer: Result<Option<Release>, String>,
        install: Result<(), String>,
    }

    impl Updater for FakeUpdater {
        fn check(&self) -> CheckFuture<'_> {
            let offer = self.offer.clone();
            Box::pin(async move { offer })
        }

        fn install(&self, version: String, mut progress: Progress) -> InstallFuture<'_> {
            let result = self.install.clone();
            Box::pin(async move {
                assert_eq!(version, "0.9.0");
                progress(4, Some(10));
                progress(10, Some(10));
                result
            })
        }
    }

    fn release() -> Release {
        Release {
            version: "0.9.0".into(),
            notes: Some("Notes".into()),
            published_at: Some(1_790_000_000),
        }
    }

    fn backend(updater: Option<FakeUpdater>) -> TestBackend {
        let backend = TestBackend::new(true);
        let updater = updater.map(|updater| Arc::new(updater) as Arc<dyn Updater>);
        let updates = Updates::new(updater, "1.2.3");
        backend.replace_updates(updates)
    }

    async fn call(
        backend: &TestBackend,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = authed(method, uri);
        let request = match body {
            Some(body) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        };
        let response = backend
            .app()
            .oneshot(request.expect("request"))
            .await
            .expect("response");
        let status = response.status();
        (status, json_body(response).await)
    }

    /// The install runs on its own task; wait until it leaves `downloading`.
    async fn settled_install(backend: &TestBackend) -> Value {
        for _ in 0..100 {
            let (_, status) = call(backend, Method::GET, "/api/update", None).await;
            if status["install"]["phase"] != "downloading" {
                return status["install"].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the install did not finish");
    }

    #[tokio::test]
    async fn debug_builds_and_tests_have_no_updates() {
        let backend = backend(None);
        let (status, body) = call(&backend, Method::GET, "/api/update", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["supported"], false);
        // The app's version (tauri.conf.json's), not Cargo's.
        assert_eq!(body["current_version"], "1.2.3");
        assert_eq!(body["install"], json!({ "phase": "idle" }));

        let (status, body) = call(&backend, Method::POST, "/api/update/check", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["supported"], false);
        assert_eq!(body["update"], Value::Null);

        let install = Some(json!({ "version": "0.9.0" }));
        let (status, body) = call(&backend, Method::POST, "/api/update/install", install).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["detail"], "update_unsupported");
    }

    #[tokio::test]
    async fn a_check_reports_the_newer_version() {
        let backend = backend(Some(FakeUpdater {
            offer: Ok(Some(release())),
            install: Ok(()),
        }));
        let (status, body) = call(&backend, Method::POST, "/api/update/check", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["supported"], true);
        assert_eq!(
            body["update"],
            json!({ "version": "0.9.0", "notes": "Notes", "published_at": 1_790_000_000 })
        );
    }

    #[tokio::test]
    async fn a_failed_check_is_a_coded_error() {
        let backend = backend(Some(FakeUpdater {
            offer: Err("404 Not Found".into()),
            install: Ok(()),
        }));
        let (status, body) = call(&backend, Method::POST, "/api/update/check", None).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["detail"], "update_check_failed: 404 Not Found");
    }

    #[tokio::test]
    async fn an_install_downloads_then_starts_the_installer() {
        let backend = backend(Some(FakeUpdater {
            offer: Ok(Some(release())),
            install: Ok(()),
        }));
        let install = Some(json!({ "version": "0.9.0" }));
        let (status, body) = call(&backend, Method::POST, "/api/update/install", install).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(body["install"]["phase"], "downloading");
        assert_eq!(
            settled_install(&backend).await,
            json!({ "phase": "installing" })
        );
    }

    #[tokio::test]
    async fn a_failed_install_keeps_the_reason_and_can_be_tried_again() {
        let backend = backend(Some(FakeUpdater {
            offer: Ok(Some(release())),
            install: Err("signature did not match".into()),
        }));
        let install = json!({ "version": "0.9.0" });
        let (status, _) = call(
            &backend,
            Method::POST,
            "/api/update/install",
            Some(install.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(
            settled_install(&backend).await,
            json!({ "phase": "failed", "detail": "signature did not match" })
        );
        let (status, _) = call(&backend, Method::POST, "/api/update/install", Some(install)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn a_second_install_waits_for_the_first() {
        let backend = backend(Some(FakeUpdater {
            offer: Ok(Some(release())),
            install: Ok(()),
        }));
        *backend.state.updates.install.lock().unwrap() = super::InstallState::Downloading {
            downloaded: 1,
            total: None,
        };
        let install = Some(json!({ "version": "0.9.0" }));
        let (status, body) = call(&backend, Method::POST, "/api/update/install", install).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["detail"], "update_in_progress");
    }
}

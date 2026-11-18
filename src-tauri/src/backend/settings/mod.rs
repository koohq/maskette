//! Settings page: switch the agent LLM, the analysis LLM and the image
//! backend, and fill in their API keys.
//!
//! Ported from the old backend's `app/settings/`. API keys are kept per
//! service and are write-only: responses only say which services have one
//! and its last four characters. `GET /api/settings/prompts` (the prompt inventory) is
//! served by [`crate::backend::prompts`].

pub mod models;
pub mod probe;
pub mod secrets;
pub mod store;

use std::collections::BTreeMap;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use super::SharedState;
use super::error::ApiError;
use models::CandidatesOut;
use probe::ProbeOut;
use store::{FIELDS, Settings, SettingsStore, UpdateError};

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route(
            "/api/settings",
            get(read_settings)
                .put(update_settings)
                .delete(reset_settings),
        )
        .route("/api/settings/probe/{role}", post(probe_backend))
        .route("/api/settings/models/{role}", get(model_candidates))
}

/// A saved API key, as the settings page may see it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct KeyOut {
    /// The last four characters of a key long enough to keep it secret.
    pub hint: String,
}

#[derive(Serialize, Debug)]
pub struct SettingsOut {
    pub values: BTreeMap<&'static str, String>,
    /// The services that have a key (see [`store::service_of`]).
    pub keys: BTreeMap<String, KeyOut>,
    pub overridden: Vec<String>,
    pub options: BTreeMap<&'static str, Vec<&'static str>>,
    pub status: BTreeMap<&'static str, bool>,
}

#[derive(Deserialize)]
pub struct SettingsUpdate {
    /// A null value drops the saved change, so the default applies again.
    #[serde(default)]
    values: BTreeMap<String, Option<String>>,
    /// API keys by service; a null value removes the service's key.
    #[serde(default)]
    keys: BTreeMap<String, Option<String>>,
}

fn key_out(value: &str) -> KeyOut {
    let chars: Vec<char> = value.chars().collect();
    let hint = if chars.len() >= 12 {
        format!("…{}", chars[chars.len() - 4..].iter().collect::<String>())
    } else {
        String::new()
    };
    KeyOut { hint }
}

fn on_path(program: &str) -> bool {
    which::which(program).is_ok()
}

pub fn describe(store: &SettingsStore) -> SettingsOut {
    let (settings, saved) = store.snapshot();
    let mut values: BTreeMap<&'static str, String> = FIELDS
        .iter()
        .map(|field| (field.key, settings.get(field.key).to_owned()))
        .collect();
    values.insert("llm_provider", settings.llm_provider().to_owned());
    values.insert("image_provider", settings.image_provider().to_owned());

    let options = FIELDS
        .iter()
        .filter_map(|field| {
            let allowed = field.allowed?;
            let usable = allowed
                .iter()
                .copied()
                .filter(|value| *value != "fixture" || settings.allow_fixture)
                .collect();
            Some((field.key, usable))
        })
        .collect();

    let status = BTreeMap::from([
        ("codex_cli", on_path(Settings::CODEX_PATH)),
        ("claude_cli", on_path(Settings::CLAUDE_CODE_PATH)),
        (
            "siliconflow_key",
            !settings.siliconflow_api_key().is_empty(),
        ),
        ("ark_key", !settings.ark_api_key().is_empty()),
        ("openrouter_key", !settings.openrouter_api_key().is_empty()),
        ("agent_llm_key", !settings.agent_api_key().is_empty()),
        (
            "analysis_llm_key",
            !settings.analysis_endpoint().1.is_empty(),
        ),
    ]);

    SettingsOut {
        values,
        keys: settings
            .api_keys()
            .iter()
            .map(|(service, key)| (service.clone(), key_out(key)))
            .collect(),
        overridden: saved.into_iter().collect(),
        options,
        status,
    }
}

impl From<UpdateError> for ApiError {
    fn from(error: UpdateError) -> Self {
        match error {
            UpdateError::Invalid(key) => ApiError::bad_request(format!("invalid_setting:{key}")),
            UpdateError::InvalidKey(service) => {
                ApiError::bad_request(format!("invalid_api_key:{service}"))
            }
            UpdateError::Storage(message) => {
                log::error!("saving settings failed: {message}");
                ApiError::internal(format!("settings_not_saved: {message}"))
            }
        }
    }
}

/// Runs a blocking store call (the credential store) off the async threads.
async fn blocking<F>(state: &SharedState, work: F) -> Result<Json<SettingsOut>, ApiError>
where
    F: FnOnce(&SettingsStore) -> Result<(), UpdateError> + Send + 'static,
{
    let store = state.settings.clone();
    tokio::task::spawn_blocking(move || {
        work(&store)?;
        Ok(Json(describe(&store)))
    })
    .await
    .map_err(|error| ApiError::internal(format!("settings task failed: {error}")))?
}

async fn read_settings(State(state): State<SharedState>) -> Json<SettingsOut> {
    Json(describe(&state.settings))
}

async fn update_settings(
    State(state): State<SharedState>,
    body: Result<Json<SettingsUpdate>, JsonRejection>,
) -> Result<Json<SettingsOut>, ApiError> {
    let Json(update) = body?;
    let changes: Vec<(String, Option<String>)> = update.values.into_iter().collect();
    let keys: Vec<(String, Option<String>)> = update.keys.into_iter().collect();
    blocking(&state, move |store| store.update_with_keys(&changes, &keys)).await
}

async fn reset_settings(State(state): State<SharedState>) -> Result<Json<SettingsOut>, ApiError> {
    blocking(&state, SettingsStore::reset).await
}

/// Sends one small test request with the saved settings (costs a few tokens).
async fn probe_backend(
    State(state): State<SharedState>,
    Path(role): Path<String>,
) -> Result<Json<ProbeOut>, ApiError> {
    let settings = state.settings.settings();
    match role.as_str() {
        "agent" if settings.get("agent_llm_provider") == "openai_compatible" => {
            Ok(Json(probe::probe_agent(&settings, &state.chat).await))
        }
        "analysis" if settings.analysis_provider() == "openai_compatible" => {
            Ok(Json(probe::probe_analysis(&settings, &state.chat).await))
        }
        _ => Err(ApiError::bad_request("probe_unsupported")),
    }
}

#[derive(Deserialize)]
struct ModelsQuery {
    /// The URL in the form, which may not be saved yet.
    #[serde(default)]
    base_url: String,
}

/// The models a role's model field offers for `base_url` (none unless it is
/// OpenRouter). Free: the lists need no key.
async fn model_candidates(
    State(state): State<SharedState>,
    Path(role): Path<String>,
    Query(query): Query<ModelsQuery>,
) -> Result<Json<CandidatesOut>, ApiError> {
    let role = models::Role::parse(&role)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Not Found"))?;
    let settings = state.settings.settings();
    Ok(Json(
        models::candidates(
            &state.generation.client,
            &state.model_lists,
            &state.generation.models,
            &settings,
            role,
            &query.base_url,
        )
        .await,
    ))
}

#[cfg(test)]
mod tests;

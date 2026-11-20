//! The audit page (`/admin/audit`): counts and the list of generation jobs,
//! read from the job records (the old `app/audit/router.py`).
//!
//! The records are the files under `generation-jobs/` ([`jobs`]); the old
//! SQLite database is not imported.
//!
//! The old page was an admin area of a shared server, behind a password and
//! a session cookie. Here only the app's own window can call the API (the
//! per-launch token), so the page has no sign-in: the session, sign-in and
//! sign-out routes are gone. The quota policy is kept in memory, as before;
//! nothing enforces it in the desktop app.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::SharedState;
use super::error::ApiError;
use super::generation::jobs::{self, JobRecord};
use super::references;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/audit/summary", get(summary))
        .route("/api/audit/generation-jobs", get(generation_jobs))
        .route("/api/audit/quota-policy", patch(update_quota_policy))
}

/// What the audit page keeps between requests.
#[derive(Default)]
pub struct Audit {
    quota_policy: Mutex<QuotaPolicy>,
}

impl Audit {
    fn quota_policy(&self) -> QuotaPolicy {
        self.quota_policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuotaPolicy {
    pub window_hours: i64,
    pub normal_window_limit: i64,
    pub premium_unlimited: bool,
    pub parallel_generation_limit: i64,
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        Self {
            window_hours: 5,
            normal_window_limit: 8,
            premium_unlimited: true,
            parallel_generation_limit: 8,
        }
    }
}

impl QuotaPolicy {
    /// The old model's bounds.
    fn check(&self) -> Result<(), ApiError> {
        let bounds = [
            ("window_hours", self.window_hours, 1, 168),
            ("normal_window_limit", self.normal_window_limit, 1, 1000),
            (
                "parallel_generation_limit",
                self.parallel_generation_limit,
                1,
                64,
            ),
        ];
        for (name, value, min, max) in bounds {
            if !(min..=max).contains(&value) {
                return Err(ApiError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("{name} must be between {min} and {max}"),
                ));
            }
        }
        Ok(())
    }
}

/// `PATCH /api/audit/quota-policy`: replaces the whole policy (missing
/// fields take their defaults, as before).
async fn update_quota_policy(
    State(state): State<SharedState>,
    payload: Result<Json<QuotaPolicy>, JsonRejection>,
) -> Result<Json<QuotaPolicy>, ApiError> {
    let Json(policy) = payload?;
    policy.check()?;
    *state
        .audit
        .quota_policy
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = policy.clone();
    Ok(Json(policy))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TokenUsageOut {
    pub jobs_with_usage: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsageOut {
    /// A job's usage, when it has any value (the old `has_values`).
    fn of(record: &JobRecord) -> Option<Self> {
        let usage = record.token_usage.as_ref()?.as_object()?;
        let field = |name: &str| usage.get(name).and_then(Value::as_u64);
        let names = [
            "input_tokens",
            "cached_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
            "total_tokens",
        ];
        if names.iter().all(|name| field(name).is_none()) {
            return None;
        }
        Some(Self {
            jobs_with_usage: 1,
            input_tokens: field("input_tokens").unwrap_or(0),
            cached_input_tokens: field("cached_input_tokens").unwrap_or(0),
            output_tokens: field("output_tokens").unwrap_or(0),
            reasoning_output_tokens: field("reasoning_output_tokens").unwrap_or(0),
            total_tokens: field("total_tokens").unwrap_or(0),
        })
    }

    fn add(&mut self, other: Self) {
        self.jobs_with_usage += other.jobs_with_usage;
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct ImageUsageOut {
    pub generated_images: u64,
    pub jobs_with_outputs: u64,
    pub images_with_token_usage: u64,
    pub input_tokens_per_image: f64,
    pub cached_input_tokens_per_image: f64,
    pub output_tokens_per_image: f64,
    pub reasoning_output_tokens_per_image: f64,
    pub total_tokens_per_image: f64,
}

#[derive(Debug, Serialize)]
pub struct SummaryOut {
    pub total_users: usize,
    pub active_users: usize,
    pub job_counts: BTreeMap<String, u64>,
    pub queue_length: u64,
    pub quota_policy: QuotaPolicy,
    pub total_calls: usize,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub parallel_slots_used: i64,
    pub token_usage: TokenUsageOut,
    pub image_usage: ImageUsageOut,
}

fn round_to(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

fn rate(count: u64, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    round_to(count as f64 / total as f64, 4)
}

fn per_image(tokens: u64, images: u64) -> f64 {
    if images == 0 {
        return 0.0;
    }
    round_to(tokens as f64 / images as f64, 2)
}

/// Projects that have jobs (the old "users": one per character session).
fn unique_sessions<'a>(records: impl Iterator<Item = &'a JobRecord>) -> usize {
    records
        .map(|record| record.character_session_id.as_str())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>()
        .len()
}

pub fn summarize(records: &[JobRecord], quota_policy: QuotaPolicy) -> SummaryOut {
    let mut job_counts: BTreeMap<String, u64> = BTreeMap::new();
    for record in records {
        *job_counts.entry(record.status.clone()).or_default() += 1;
    }
    let count = |status: &str| job_counts.get(status).copied().unwrap_or(0);
    // Any job not yet finished is active; the old list of active statuses
    // named the Codex steps, not this backend's.
    let active: Vec<&JobRecord> = records.iter().filter(|r| !r.is_finished()).collect();

    let mut token_usage = TokenUsageOut::default();
    let mut images_with_token_usage = 0;
    for record in records {
        if let Some(usage) = TokenUsageOut::of(record) {
            token_usage.add(usage);
            images_with_token_usage += record.outputs.len() as u64;
        }
    }
    let generated_images: u64 = records.iter().map(|r| r.outputs.len() as u64).sum();
    let denominator = if images_with_token_usage > 0 {
        images_with_token_usage
    } else {
        generated_images
    };
    let image_usage = ImageUsageOut {
        generated_images,
        jobs_with_outputs: records.iter().filter(|r| !r.outputs.is_empty()).count() as u64,
        images_with_token_usage,
        input_tokens_per_image: per_image(token_usage.input_tokens, denominator),
        cached_input_tokens_per_image: per_image(token_usage.cached_input_tokens, denominator),
        output_tokens_per_image: per_image(token_usage.output_tokens, denominator),
        reasoning_output_tokens_per_image: per_image(
            token_usage.reasoning_output_tokens,
            denominator,
        ),
        total_tokens_per_image: per_image(token_usage.total_tokens, denominator),
    };

    SummaryOut {
        total_users: unique_sessions(records.iter()),
        active_users: unique_sessions(active.iter().copied()),
        queue_length: count("queued"),
        total_calls: records.len(),
        success_rate: rate(count("succeeded"), records.len()),
        failure_rate: rate(count("failed"), records.len()),
        parallel_slots_used: (active.len() as i64).min(quota_policy.parallel_generation_limit),
        job_counts,
        quota_policy,
        token_usage,
        image_usage,
    }
}

/// `GET /api/audit/summary`.
async fn summary(State(state): State<SharedState>) -> Json<SummaryOut> {
    let records = state.generation.jobs.list();
    Json(summarize(&records, state.audit.quota_policy()))
}

#[derive(Debug, Serialize)]
pub struct ReferenceOut {
    pub reference_key: String,
    pub image_url: Option<String>,
    pub kind: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GenerationJobOut {
    pub id: String,
    pub character_session_id: String,
    pub generation_mode: String,
    pub status: String,
    pub progress: u32,
    pub phase_label: String,
    pub provider: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub user_notes: String,
    pub user_requirements: String,
    pub requirement_ids: Vec<String>,
    pub references: Vec<ReferenceOut>,
    pub outputs: Vec<jobs::JobOutput>,
    pub token_usage: Option<TokenUsageOut>,
}

/// A reference key's object key (`references/...`, after the `kind:`
/// prefix), when it is a plain path inside the reference folder.
fn reference_object_key(reference_key: &str) -> Option<String> {
    let normalized = reference_key.replace('\\', "/");
    let path = normalized
        .split_once(':')
        .map_or(normalized.as_str(), |(_, path)| path);
    if path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.len() < 3 || parts[0] != references::DIR_NAME || parts.contains(&"..") {
        return None;
    }
    Some(parts.join("/"))
}

fn reference_kind(reference_key: &str) -> Option<String> {
    let (kind, _) = reference_key.split_once(':')?;
    let kind = kind.trim();
    (!kind.is_empty()).then(|| kind.to_owned())
}

fn text_field(payload: &Value, key: &str) -> String {
    match payload.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn string_list(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| match item {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn created_at(record: &JobRecord) -> Option<&str> {
    record.events.first().map(|event| event.created_at.as_str())
}

fn updated_at(record: &JobRecord) -> Option<&str> {
    record.events.last().map(|event| event.created_at.as_str())
}

pub fn job_out(record: &JobRecord) -> GenerationJobOut {
    let payload = &record.prompt_payload;
    let descriptions: HashMap<String, String> = payload
        .get("reference_descriptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let key = item.get("reference_key")?.as_str()?.trim();
            let description = item.get("description")?.as_str()?.trim();
            (!key.is_empty() && !description.is_empty())
                .then(|| (key.to_owned(), description.to_owned()))
        })
        .collect();
    let references = string_list(payload, "reference_keys")
        .into_iter()
        .map(|reference_key| ReferenceOut {
            image_url: reference_object_key(&reference_key).map(|key| references::asset_url(&key)),
            kind: reference_kind(&reference_key),
            description: descriptions.get(&reference_key).cloned(),
            reference_key,
        })
        .collect();
    GenerationJobOut {
        id: record.id.clone(),
        character_session_id: record.character_session_id.clone(),
        generation_mode: record.generation_mode.clone(),
        status: record.status.clone(),
        progress: record.progress,
        phase_label: record.phase_label.clone(),
        provider: record.provider.clone(),
        created_at: created_at(record).map(str::to_owned),
        updated_at: updated_at(record).map(str::to_owned),
        user_notes: text_field(payload, "user_notes"),
        user_requirements: text_field(payload, "user_requirements"),
        requirement_ids: string_list(payload, "requirement_ids"),
        references,
        outputs: record.outputs.clone(),
        token_usage: TokenUsageOut::of(record),
    }
}

/// `GET /api/audit/generation-jobs`: newest change first.
async fn generation_jobs(State(state): State<SharedState>) -> Json<Vec<GenerationJobOut>> {
    let mut records = state.generation.jobs.list();
    records.sort_by(|a, b| {
        let key = |record: &JobRecord| {
            updated_at(record)
                .or_else(|| created_at(record))
                .unwrap_or("")
                .to_owned()
        };
        key(b).cmp(&key(a))
    });
    Json(records.iter().map(job_out).collect())
}

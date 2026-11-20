//! A failure the screen can put in the user's language.
//!
//! Every failure has plain English text: the model reads it (a failed
//! tool's result), and the logs and job records keep it. The ones the user
//! has to act on (a CLI that is missing or logged out, an API that refused
//! the key or is busy, a missing setting, a refused reference) also carry
//! a code and the values to fill in. The screen translates the code
//! (`agent.failures.<code>`) and adds `detail`, the CLI's or API's own
//! words, as they are. Failures without a code (mostly the model misusing a
//! tool) are shown in English as they are.

use std::fmt;

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    pub code: Option<&'static str>,
    pub params: Map<String, Value>,
}

impl Failure {
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: Some(code),
            params: Map::new(),
        }
    }

    /// Adds a value the translation fills in.
    #[must_use]
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// Adds `detail`, words the screen shows as they are (a CLI's or an
    /// API's own message). Nothing when they are empty.
    #[must_use]
    pub fn with_detail(self, detail: &str) -> Self {
        let detail = detail.trim();
        if detail.is_empty() {
            self
        } else {
            self.with("detail", detail)
        }
    }

    /// A request to a hosted API (`service`, e.g. OpenRouter or the
    /// endpoint's host) that got no answer (`status` `None`) or an error
    /// status, sorted by what the user can do about it. `detail` is the
    /// API's own message. 402 is OpenRouter's answer when the account's
    /// credits have run out, which stops the chat before anything else.
    pub fn api(
        service: &str,
        status: Option<u16>,
        message: impl Into<String>,
        detail: &str,
    ) -> Self {
        let code = match status {
            None => "api_unreachable",
            Some(401 | 403) => "api_unauthorized",
            Some(402) => "api_payment_required",
            Some(429) => "api_rate_limited",
            Some(500..=599) => "api_server_error",
            Some(_) => "api_http_error",
        };
        let failure = Self::coded(code, message).with("service", service);
        match status {
            Some(status) => failure.with("status", status),
            None => failure,
        }
        .with_detail(detail)
    }

    /// A hosted API that sent an error partway through a streamed answer.
    pub fn api_stream_failed(service: &str, message: impl Into<String>, detail: &str) -> Self {
        Self::coded("api_stream_failed", message)
            .with("service", service)
            .with_detail(detail)
    }

    /// A hosted API that did not answer within `seconds`.
    pub fn api_timeout(service: &str, seconds: u64, message: impl Into<String>) -> Self {
        Self::coded("api_timeout", message)
            .with("service", service)
            .with("seconds", seconds)
    }

    /// Settings that must be filled in first (their keys, e.g.
    /// `openrouter_api_key`).
    pub fn missing_settings(settings: &[&str], message: impl Into<String>) -> Self {
        let code = if settings.iter().all(|setting| setting.ends_with("_api_key")) {
            "api_key_missing"
        } else {
            "setting_missing"
        };
        Self::coded(code, message).with("settings", settings.join(", "))
    }

    /// The user stopped the run.
    pub fn cancelled() -> Self {
        Self::coded("cancelled", "Cancelled by the user.")
    }

    /// The fields an event carries next to the English text: `code` and
    /// `params`, under names that start with `prefix`. Nothing without a
    /// code.
    pub fn event_fields(&self, prefix: &str) -> Map<String, Value> {
        let mut fields = Map::new();
        if let Some(code) = self.code {
            fields.insert(format!("{prefix}code"), json!(code));
            fields.insert(
                format!("{prefix}params"),
                Value::Object(self.params.clone()),
            );
        }
        fields
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            code: None,
            params: Map::new(),
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_no_event_fields() {
        let failure = Failure::from("boom");
        assert_eq!(failure.to_string(), "boom");
        assert!(failure.event_fields("").is_empty());
    }

    #[test]
    fn a_coded_failure_carries_its_code_and_values() {
        let failure = Failure::coded("cli_timeout", "too slow")
            .with("name", "Codex")
            .with("seconds", 240);
        let fields = failure.event_fields("error_");
        assert_eq!(fields["error_code"], json!("cli_timeout"));
        assert_eq!(
            fields["error_params"],
            json!({"name": "Codex", "seconds": 240})
        );
    }

    #[test]
    fn api_failures_are_sorted_by_status() {
        let code = |status| Failure::api("OpenRouter", status, "x", "").code;
        assert_eq!(code(None), Some("api_unreachable"));
        assert_eq!(code(Some(401)), Some("api_unauthorized"));
        assert_eq!(code(Some(403)), Some("api_unauthorized"));
        assert_eq!(code(Some(402)), Some("api_payment_required"));
        assert_eq!(code(Some(429)), Some("api_rate_limited"));
        assert_eq!(code(Some(500)), Some("api_server_error"));
        assert_eq!(code(Some(503)), Some("api_server_error"));
        assert_eq!(code(Some(400)), Some("api_http_error"));
        assert_eq!(code(Some(404)), Some("api_http_error"));
        let failure = Failure::api("OpenRouter", Some(402), "HTTP 402", " no credits ");
        assert_eq!(
            Value::Object(failure.params),
            json!({"service": "OpenRouter", "status": 402, "detail": "no credits"})
        );
    }

    #[test]
    fn missing_keys_and_other_settings_have_their_own_codes() {
        let keys = Failure::missing_settings(&["openrouter_api_key"], "x");
        assert_eq!(keys.code, Some("api_key_missing"));
        assert_eq!(keys.params["settings"], json!("openrouter_api_key"));
        let other = Failure::missing_settings(&["agent_llm_base_url", "agent_llm_model"], "x");
        assert_eq!(other.code, Some("setting_missing"));
        assert_eq!(
            other.params["settings"],
            json!("agent_llm_base_url, agent_llm_model")
        );
    }
}

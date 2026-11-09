//! A failure the screen can put in the user's language.
//!
//! Most failures stay plain English text: the model reads them (a failed
//! tool's result), and the screen shows them as they are. The ones the
//! user has to act on (a CLI that is missing, logged out or over its usage
//! limit) also carry a code and the values to fill in. The screen
//! translates the code (`agent.failures.<code>`) and adds `detail`, the
//! CLI's own words, as they are. `message` says the same in English, for
//! the logs, the model and screens that do not know the code.

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
}

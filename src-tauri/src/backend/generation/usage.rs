//! Token usage of one job, ported from the old `app/generation/usage.py`.
//!
//! Image APIs and the Codex CLI report usage under different names
//! (`prompt_tokens` or `input_tokens`, and so on); this reads either into the
//! old `TokenUsage` shape, which the audit page sums.

use serde_json::{Value, json};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    /// Reads a `usage` object; `None` when it holds no count at all.
    pub fn parse(value: &Value) -> Option<Self> {
        let fields = value.as_object()?;
        let plain = |name: &str| fields.get(name).and_then(count);
        let nested = |parent: &str, name: &str| {
            fields
                .get(parent)
                .and_then(|parent| parent.get(name))
                .and_then(count)
        };
        let input_tokens = plain("input_tokens").or_else(|| plain("prompt_tokens"));
        let output_tokens = plain("output_tokens").or_else(|| plain("completion_tokens"));
        let usage = Self {
            input_tokens,
            cached_input_tokens: plain("cached_input_tokens")
                .or_else(|| plain("cached_tokens"))
                .or_else(|| nested("input_tokens_details", "cached_tokens"))
                .or_else(|| nested("prompt_tokens_details", "cached_tokens")),
            output_tokens,
            reasoning_output_tokens: plain("reasoning_output_tokens")
                .or_else(|| plain("reasoning_tokens"))
                .or_else(|| nested("output_tokens_details", "reasoning_tokens"))
                .or_else(|| nested("completion_tokens_details", "reasoning_tokens")),
            total_tokens: plain("total_tokens").or_else(|| Some(input_tokens? + output_tokens?)),
        };
        usage.has_values().then_some(usage)
    }

    /// Adds up several reports, as the old `merge_token_usage`.
    pub fn merge(usages: impl IntoIterator<Item = Self>) -> Option<Self> {
        let usages: Vec<Self> = usages.into_iter().filter(Self::has_values).collect();
        if usages.is_empty() {
            return None;
        }
        let sum = |field: fn(&Self) -> Option<u64>| {
            usages
                .iter()
                .filter_map(field)
                .reduce(|total, value| total + value)
        };
        let input_tokens = sum(|usage| usage.input_tokens);
        let output_tokens = sum(|usage| usage.output_tokens);
        Some(Self {
            input_tokens,
            cached_input_tokens: sum(|usage| usage.cached_input_tokens),
            output_tokens,
            reasoning_output_tokens: sum(|usage| usage.reasoning_output_tokens),
            total_tokens: sum(|usage| usage.total_tokens)
                .or_else(|| Some(input_tokens? + output_tokens?)),
        })
    }

    /// Every `usage` in the Codex CLI's JSON events (`turn.completed`), added up.
    pub fn from_codex_events(stdout: &[u8]) -> Option<Self> {
        Self::merge(
            stdout
                .split(|byte| *byte == b'\n')
                .filter_map(|line| serde_json::from_slice::<Value>(line.trim_ascii()).ok())
                .filter_map(|event| event.get("usage").and_then(Self::parse)),
        )
    }

    pub fn has_values(&self) -> bool {
        [
            self.input_tokens,
            self.cached_input_tokens,
            self.output_tokens,
            self.reasoning_output_tokens,
            self.total_tokens,
        ]
        .iter()
        .any(Option::is_some)
    }

    /// The old `to_dict`, plus the price in US dollars when the API gave one.
    pub fn to_json(self, cost: Option<f64>) -> Value {
        let mut value = json!({
            "input_tokens": self.input_tokens,
            "cached_input_tokens": self.cached_input_tokens,
            "output_tokens": self.output_tokens,
            "reasoning_output_tokens": self.reasoning_output_tokens,
            "total_tokens": self.total_tokens,
        });
        if let (Some(cost), Value::Object(fields)) = (cost, &mut value) {
            fields.insert("cost".to_owned(), json!(cost));
        }
        value
    }
}

/// A non-negative whole number (the old `_int_value`).
fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|number| *number >= 0.0 && number.fract() == 0.0 && *number <= u64::MAX as f64)
            .map(|number| number as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_usage_maps_to_the_old_names() {
        let usage = TokenUsage::parse(&json!({
            "prompt_tokens": 3573,
            "completion_tokens": 3052,
            "completion_tokens_details": {"image_tokens": 1680, "reasoning_tokens": 0},
            "prompt_tokens_details": {"cached_tokens": 12},
            "cost": 0.066,
        }))
        .unwrap();
        assert_eq!(
            usage,
            TokenUsage {
                input_tokens: Some(3573),
                cached_input_tokens: Some(12),
                output_tokens: Some(3052),
                reasoning_output_tokens: Some(0),
                total_tokens: Some(6625),
            }
        );
        let stored = usage.to_json(Some(0.066));
        assert_eq!(stored["total_tokens"], 6625);
        assert_eq!(stored["cost"], 0.066);
    }

    #[test]
    fn empty_or_odd_usage_is_none() {
        assert_eq!(TokenUsage::parse(&json!({})), None);
        assert_eq!(TokenUsage::parse(&json!({"cost": 0.1})), None);
        assert_eq!(TokenUsage::parse(&json!("12")), None);
        assert_eq!(
            TokenUsage::parse(&json!({"input_tokens": -1, "output_tokens": true})),
            None
        );
        // A whole float still counts; a total is not made up from one side.
        let usage = TokenUsage::parse(&json!({"input_tokens": 10.0})).unwrap();
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.total_tokens, None);
    }

    #[test]
    fn codex_events_are_added_up() {
        let stdout = b"{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n\
            not json\n\
            {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":40,\"output_tokens\":7}}\n\
            {\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":50,\"output_tokens\":3,\"reasoning_output_tokens\":2}}\n";
        let usage = TokenUsage::from_codex_events(stdout).unwrap();
        assert_eq!(usage.input_tokens, Some(150));
        assert_eq!(usage.cached_input_tokens, Some(40));
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.reasoning_output_tokens, Some(2));
        assert_eq!(usage.total_tokens, Some(160));
        assert_eq!(TokenUsage::from_codex_events(b"{\"type\":\"x\"}\n"), None);
    }
}

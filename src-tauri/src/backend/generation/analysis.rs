//! Reference analysis: the head details (hair, eyes, ears, headwear, ...) a
//! design has to keep.
//!
//! Only the fixture analysis has moved so far (see the module above). The
//! clean-up of a result is the old `app/generation/detail_analysis.py`'s
//! `filter_head_detail_analysis_result` and `ensure_user_requirement_feature`.

use serde::{Deserialize, Serialize};

use crate::backend::settings::store::Settings;

pub const USER_REQUIREMENT_FEATURE_ID: &str = "feature-user-requirement";

const HEAD_DETAIL_KINDS: [&str; 6] = [
    "hair",
    "ears",
    "eyes",
    "expression",
    "headwear",
    "accessory",
];
const NON_HEAD_DETAIL_TERMS: [&str; 26] = [
    "hand", "hands", "gesture", "pose", "body", "clothing", "clothes", "outfit", "uniform",
    "dress", "shirt", "skirt", "wrist", "bracelet", "手", "手势", "动作", "姿势", "身体", "服装",
    "衣服", "制服", "裙", "上衣", "手腕", "手链",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Feature {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub description: String,
}

impl Feature {
    fn new(id: &str, kind: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.to_owned(),
            kind: kind.to_owned(),
            label: label.to_owned(),
            description: description.to_owned(),
        }
    }

    fn is_user_requirement(&self) -> bool {
        self.id == USER_REQUIREMENT_FEATURE_ID || self.kind == "requirement"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnalysisResult {
    pub features: Vec<Feature>,
    pub warnings: Vec<String>,
}

#[expect(
    dead_code,
    reason = "read by the real analysis backends, which move next"
)]
pub struct AnalysisRequest {
    pub analysis_id: String,
    pub character_session_id: String,
    pub free_text: String,
    pub locale: String,
    pub reference_keys: Vec<String>,
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "raised by the real analysis backends, which move next"
)]
pub enum AnalysisError {
    /// The safety check refused a reference.
    Rejected {
        reason: String,
        message: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisBackend {
    Fixture,
}

/// The analysis backend the settings choose, or why there is none.
pub fn analysis_backend(settings: &Settings) -> Result<AnalysisBackend, String> {
    match settings.llm_provider() {
        "fixture" if settings.allow_fixture => Ok(AnalysisBackend::Fixture),
        "fixture" => Err("No real analysis backend is configured (llm_provider).".to_owned()),
        other => Err(format!(
            "Reference analysis with {other} has not moved to the new backend yet."
        )),
    }
}

impl AnalysisBackend {
    pub async fn analyze(
        self,
        _request: &AnalysisRequest,
    ) -> Result<AnalysisResult, AnalysisError> {
        match self {
            Self::Fixture => Ok(AnalysisResult {
                features: vec![
                    Feature::new(
                        "feature-hair",
                        "hair",
                        "Hair",
                        "Long straight hair with visible bangs",
                    ),
                    Feature::new(
                        "feature-expression",
                        "expression",
                        "Expression",
                        "Soft sad expression with a small mouth",
                    ),
                    Feature::new(
                        "feature-avoid-smile",
                        "avoid",
                        "Avoid",
                        "Do not change the expression into a smile",
                    ),
                ],
                warnings: Vec::new(),
            }),
        }
    }
}

fn is_head_detail(kind: &str, label: &str, description: &str) -> bool {
    if !HEAD_DETAIL_KINDS.contains(&kind) {
        return false;
    }
    let haystack = format!("{label} {description}").to_lowercase();
    !NON_HEAD_DETAIL_TERMS
        .iter()
        .any(|term| haystack.contains(term))
}

/// Keeps the first user requirement and the details of the head only.
pub fn filter_head_details(result: AnalysisResult) -> AnalysisResult {
    let mut requirement = None;
    let mut features = Vec::new();
    for feature in result.features {
        if feature.is_user_requirement() {
            requirement.get_or_insert(feature);
        } else if is_head_detail(&feature.kind, &feature.label, &feature.description) {
            features.push(feature);
        }
    }
    AnalysisResult {
        features: requirement.into_iter().chain(features).collect(),
        warnings: result.warnings,
    }
}

/// Puts what the user asked for first, as a feature of its own.
pub fn ensure_user_requirement(
    result: AnalysisResult,
    free_text: &str,
    locale: &str,
) -> AnalysisResult {
    let description = free_text.trim();
    let (existing, others): (Vec<Feature>, Vec<Feature>) = result
        .features
        .into_iter()
        .partition(Feature::is_user_requirement);
    let first = match existing.into_iter().next() {
        Some(feature) => feature,
        None if description.is_empty() => {
            return AnalysisResult {
                features: others,
                warnings: result.warnings,
            };
        }
        None => Feature::new(
            USER_REQUIREMENT_FEATURE_ID,
            "requirement",
            user_requirement_label(locale),
            &description.chars().take(1000).collect::<String>(),
        ),
    };
    AnalysisResult {
        features: std::iter::once(first).chain(others).collect(),
        warnings: result.warnings,
    }
}

fn user_requirement_label(locale: &str) -> &'static str {
    match locale {
        "en" => "User requirement",
        "ja" => "ユーザー要望",
        _ => "用户要求",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_head_details_and_one_requirement_are_kept() {
        let result = filter_head_details(AnalysisResult {
            features: vec![
                Feature::new("a", "hair", "Hair", "twin tails"),
                Feature::new("b", "requirement", "Wish", "keep the bow"),
                Feature::new("c", "accessory", "Bracelet", "on the wrist"),
                Feature::new("d", "outfit", "Dress", "sailor"),
                Feature::new("e", "requirement", "Wish", "second"),
            ],
            warnings: vec!["w".to_owned()],
        });
        let ids: Vec<&str> = result.features.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(result.warnings, ["w"]);
    }

    #[test]
    fn the_users_note_becomes_the_first_feature() {
        let base = AnalysisResult {
            features: vec![Feature::new("a", "hair", "Hair", "x")],
            warnings: Vec::new(),
        };
        let result = ensure_user_requirement(base.clone(), " 猫耳を残す ", "ja");
        assert_eq!(result.features[0].id, USER_REQUIREMENT_FEATURE_ID);
        assert_eq!(result.features[0].label, "ユーザー要望");
        assert_eq!(result.features[0].description, "猫耳を残す");
        assert_eq!(ensure_user_requirement(base.clone(), "", "ja"), base);
    }
}

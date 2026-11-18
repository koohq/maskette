//! Model candidates for the model fields on the settings page.
//!
//! On OpenRouter the page offers the models that suit each role, with their
//! prices: chat models that call tools for the chat assistant, models that
//! take images for analysis, and image models that take reference images
//! for generation. OpenRouter lists its models without a key. Elsewhere
//! there are no candidates and the model is typed in, as it always can be;
//! whether a typed model suits the role is what the connection test checks.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use serde_json::Value;

use super::store::Settings;
use crate::backend::chat_api::JsonObject;
use crate::backend::generation::hosted;

const OPENROUTER_HOST: &str = "openrouter.ai";
const LIST_TTL: Duration = Duration::from_secs(3600);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
/// An image model's price is on a page of its own; this many are read at
/// once.
const PRICE_READS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Agent,
    Analysis,
    Image,
}

impl Role {
    pub fn parse(role: &str) -> Option<Self> {
        match role {
            "agent" => Some(Self::Agent),
            "analysis" => Some(Self::Analysis),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// What a model costs, in US dollars, as OpenRouter lists it.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Price {
    /// `token`: per million tokens; `image`: per image.
    pub unit: &'static str,
    /// The input (prompt) price; image models list only the output.
    pub input: Option<f64>,
    /// The output price: generated text, or the generated image.
    pub output: Option<f64>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub price: Option<Price>,
}

#[derive(Serialize, Debug)]
pub struct CandidatesOut {
    pub models: Vec<Candidate>,
}

/// A chat model as listed, with what the two chat roles look for.
#[derive(Clone)]
struct ChatModel {
    candidate: Candidate,
    tools: bool,
    vision: bool,
}

type Lists<T> = Mutex<HashMap<String, (Instant, Vec<T>)>>;

/// The lists read for the settings page, by base URL, with when they were
/// read. Cleared on restart. The image list itself is shared with
/// generation ([`hosted::ModelCache`]); kept here are the candidates made
/// from it, with the prices read for them.
#[derive(Default)]
pub struct ModelLists {
    chat: Lists<ChatModel>,
    images: Lists<Candidate>,
}

/// The models at `base_url` that suit `role`, or none when it is not
/// OpenRouter or the list cannot be read.
pub async fn candidates(
    client: &reqwest::Client,
    lists: &ModelLists,
    image_cache: &hosted::ModelCache,
    settings: &Settings,
    role: Role,
    base_url: &str,
) -> CandidatesOut {
    let base_url = base_url.trim().trim_end_matches('/');
    if !is_openrouter(settings, base_url) {
        return CandidatesOut { models: Vec::new() };
    }
    let models = match role {
        Role::Agent | Role::Analysis => chat_models(client, lists, base_url).await.map(|models| {
            models
                .into_iter()
                .filter(|model| match role {
                    Role::Agent => model.tools,
                    _ => model.vision,
                })
                .map(|model| model.candidate)
                .collect()
        }),
        Role::Image => image_models(client, lists, image_cache, base_url).await,
    };
    CandidatesOut {
        models: models.unwrap_or_else(|error| {
            log::warn!("model list of {base_url} unavailable: {error}");
            Vec::new()
        }),
    }
}

/// OpenRouter's own address, or the one saved as OpenRouter's for images
/// (a stand-in in the tests).
fn is_openrouter(settings: &Settings, base_url: &str) -> bool {
    if base_url.is_empty() {
        return false;
    }
    let saved = settings
        .get("openrouter_base_url")
        .trim()
        .trim_end_matches('/');
    base_url == saved
        || reqwest::Url::parse(base_url)
            .is_ok_and(|url| url.scheme() == "https" && url.host_str() == Some(OPENROUTER_HOST))
}

fn fresh<T: Clone>(lists: &Lists<T>, base_url: &str) -> Option<Vec<T>> {
    lists
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(base_url)
        .filter(|(read_at, _)| read_at.elapsed() <= LIST_TTL)
        .map(|(_, models)| models.clone())
}

fn keep<T>(lists: &Lists<T>, base_url: &str, models: Vec<T>) {
    lists
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(base_url.to_owned(), (Instant::now(), models));
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let response = client
        .get(url)
        .timeout(LIST_TIMEOUT)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json().await.map_err(|error| error.to_string())
}

/// `GET {base}/models`: the chat models that take chat completions.
async fn chat_models(
    client: &reqwest::Client,
    lists: &ModelLists,
    base_url: &str,
) -> Result<Vec<ChatModel>, String> {
    if let Some(models) = fresh(&lists.chat, base_url) {
        return Ok(models);
    }
    let body = get_json(client, &format!("{base_url}/models")).await?;
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or("unexpected model list")?;
    let models: Vec<ChatModel> = items
        .iter()
        .filter_map(Value::as_object)
        .filter_map(chat_model)
        .collect();
    keep(&lists.chat, base_url, models.clone());
    Ok(models)
}

fn chat_model(item: &JsonObject) -> Option<ChatModel> {
    let id = item.get("id")?.as_str().filter(|id| !id.is_empty())?;
    // Batch variants list no chat completions; models that do not say are
    // kept.
    if let Some(endpoints) = item.get("endpoints").and_then(Value::as_array) {
        let chat = endpoints
            .iter()
            .filter_map(Value::as_str)
            .any(|path| path.ends_with("/chat/completions"));
        if !chat {
            return None;
        }
    }
    let listed = |value: Option<&Value>, wanted: &str| {
        value
            .and_then(Value::as_array)
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(wanted)))
    };
    let pricing = item.get("pricing");
    let per_token = |name: &str| {
        pricing?
            .get(name)?
            .as_str()?
            .parse::<f64>()
            .ok()
            // Routers list -1: the price depends on the model they pick.
            .filter(|price| *price >= 0.0)
    };
    let price = match (per_token("prompt"), per_token("completion")) {
        (Some(input), Some(output)) => Some(Price {
            unit: "token",
            input: Some(per_million(input)),
            output: Some(per_million(output)),
        }),
        _ => None,
    };
    Some(ChatModel {
        candidate: Candidate {
            id: id.to_owned(),
            name: text(item.get("name")),
            price,
        },
        tools: listed(item.get("supported_parameters"), "tools"),
        vision: listed(
            item.get("architecture")
                .and_then(|architecture| architecture.get("input_modalities")),
            "image",
        ),
    })
}

/// `GET {base}/images/models`: the image models that take reference images,
/// with the output price of each from `GET {base}/images/models/{id}/endpoints`.
async fn image_models(
    client: &reqwest::Client,
    lists: &ModelLists,
    image_cache: &hosted::ModelCache,
    base_url: &str,
) -> Result<Vec<Candidate>, String> {
    if let Some(models) = fresh(&lists.images, base_url) {
        return Ok(models);
    }
    let listed = hosted::image_models(client, image_cache, base_url, "").await?;
    let models: Vec<Candidate> = stream::iter(
        listed
            .iter()
            .filter(|item| hosted::supported_parameters(item).contains_key("input_references"))
            .filter_map(|item| Some((hosted::model_id(item)?, text(item.get("name")))))
            .collect::<Vec<_>>(),
    )
    .map(|(id, name)| async move {
        let price = image_price(client, base_url, &id).await;
        Candidate { id, name, price }
    })
    .buffered(PRICE_READS)
    .collect()
    .await;
    keep(&lists.images, base_url, models.clone());
    Ok(models)
}

/// The output price of the model's first endpoint (the provider OpenRouter
/// tries first); `None` when it cannot be read.
async fn image_price(client: &reqwest::Client, base_url: &str, model: &str) -> Option<Price> {
    let body = match get_json(
        client,
        &format!("{base_url}/images/models/{model}/endpoints"),
    )
    .await
    {
        Ok(body) => body,
        Err(error) => {
            log::debug!("no price for image model {model}: {error}");
            return None;
        }
    };
    let pricing = body.get("endpoints")?.get(0)?.get("pricing")?.as_array()?;
    let output = pricing
        .iter()
        .find(|entry| entry.get("billable").and_then(Value::as_str) == Some("output_image"))?;
    let cost = output
        .get("cost_usd")?
        .as_f64()
        .filter(|cost| *cost >= 0.0)?;
    match output.get("unit")?.as_str()? {
        "image" => Some(Price {
            unit: "image",
            input: None,
            output: Some(round(cost)),
        }),
        "token" => Some(Price {
            unit: "token",
            input: None,
            output: Some(per_million(cost)),
        }),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn per_million(per_token: f64) -> f64 {
    round(per_token * 1_000_000.0)
}

/// Four decimal places: enough for any price, and no float noise
/// (0.0000003 per token is 0.3, not 0.30000000000000004).
fn round(price: f64) -> f64 {
    (price * 10_000.0).round() / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chat_models_say_what_they_take_and_cost() {
        let model = chat_model(
            json!({
                "id": "vendor/model",
                "name": "Vendor: Model",
                "architecture": {"input_modalities": ["text", "image"]},
                "pricing": {"prompt": "0.0000003", "completion": "0.0000025"},
                "supported_parameters": ["tools", "reasoning"],
                "endpoints": ["/api/v1/chat/completions"],
            })
            .as_object()
            .unwrap(),
        )
        .unwrap();
        assert!(model.tools && model.vision);
        assert_eq!(
            model.candidate,
            Candidate {
                id: "vendor/model".to_owned(),
                name: "Vendor: Model".to_owned(),
                price: Some(Price {
                    unit: "token",
                    input: Some(0.3),
                    output: Some(2.5),
                }),
            }
        );

        let router = chat_model(
            json!({"id": "openrouter/auto", "pricing": {"prompt": "-1", "completion": "-1"}})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(router.candidate.price, None);
        assert!(!router.tools && !router.vision);

        let batch = json!({"id": "vendor/model:batch", "endpoints": ["/api/v1/responses"]});
        assert!(chat_model(batch.as_object().unwrap()).is_none());
    }

    /// Reads OpenRouter's real lists (no key, nothing billed):
    /// `cargo test --lib real_openrouter_candidates -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "reads openrouter.ai"]
    async fn real_openrouter_candidates() {
        let client = reqwest::Client::new();
        let (lists, images) = (ModelLists::default(), hosted::ModelCache::default());
        let settings = Settings::defaults(false);
        let base_url = "https://openrouter.ai/api/v1";
        // Each role offers its own default model.
        for (role, default) in [
            (Role::Agent, "agent_llm_model"),
            (Role::Analysis, "agent_llm_model"),
            (Role::Image, "openrouter_image_model"),
        ] {
            let started = Instant::now();
            let out = candidates(&client, &lists, &images, &settings, role, base_url).await;
            let priced = out.models.iter().filter(|model| model.price.is_some()).count();
            let default = settings.get(default);
            let offered = out.models.iter().find(|model| model.id == default);
            println!(
                "{role:?}: {} models, {priced} priced, {:?}; default: {offered:?}",
                out.models.len(),
                started.elapsed(),
            );
            assert!(offered.is_some(), "{role:?} lacks {default}");
        }
    }

    #[test]
    fn openrouter_is_its_own_host_or_the_saved_image_url() {
        let settings = Settings::defaults(false);
        assert!(is_openrouter(&settings, "https://openrouter.ai/api/v1"));
        assert!(!is_openrouter(&settings, "http://openrouter.ai/api/v1"));
        assert!(!is_openrouter(
            &settings,
            "https://openrouter.ai.example.test/v1"
        ));
        assert!(!is_openrouter(&settings, "https://api.openai.com/v1"));
        assert!(!is_openrouter(&settings, ""));
    }
}

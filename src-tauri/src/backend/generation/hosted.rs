//! The hosted image APIs: OpenRouter (the old `openrouter.py`), Volcengine
//! Ark (`ark.py`) and SiliconFlow (`siliconflow.py`).
//!
//! OpenRouter's Image API (`POST {base}/images`) reaches the image models of
//! several vendors with one key. Models differ in which options they take
//! (FLUX 3 rejects `output_format`, Krea has no 3:4, Grok takes three
//! references), so the model's `supported_parameters` are read from
//! `GET {base}/images/models` before generating, and only what the model
//! accepts is sent. Without the listing the documented defaults go out. The
//! listing does not report prompt limits, so models with a known one get a
//! shortened prompt.
//!
//! Ark and SiliconFlow are kept but not looked after (the old fork's rule
//! for the Chinese services): if they break, they are removed rather than
//! fixed. Neither was tried against the real service in the old fork.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use image::RgbImage;
use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use super::Mode;
use super::image_api::{ApiImage, ApiRequest, ImageApi, to_data_uri};
use crate::backend::chat_api::{self, JsonObject};
use crate::backend::failure::Failure;
use crate::backend::settings::store::Settings;

/// Ratios the OpenRouter Image API documents; used when a model's own list
/// is unknown.
pub const ASPECT_RATIOS: [&str; 13] = [
    "1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "1:2", "2:1", "9:21", "21:9",
];
/// Fields the backend owns; extra fields may not replace them.
const OWNED_FIELDS: [&str; 5] = ["model", "prompt", "input_references", "n", "stream"];
/// Prompt limits by model id prefix. The model list does not report them;
/// Grok Imagine answers longer prompts with "Prompt length exceeds the
/// maximum allowed length of 8000."
const PROMPT_LIMITS: [(&str, usize); 1] = [("x-ai/grok-imagine", 8000)];
const MODELS_TTL: Duration = Duration::from_secs(3600);
const MODELS_TIMEOUT: Duration = Duration::from_secs(15);

// Settings the page does not change (the old backend's `.env`-only ones).
const OPENROUTER_FRONT_RESOLUTION: &str = "1K";
const OPENROUTER_TURNAROUND_RESOLUTION: &str = "2K";
const OPENROUTER_LOCAL_EDIT_RESOLUTION: &str = "1K";
const OPENROUTER_MAX_REFERENCES: usize = 4;
const OPENROUTER_RETRIES: usize = 2;
const ARK_FRONT_SIZE: &str = "2K";
const ARK_TURNAROUND_SIZE: &str = "1920x1280";
const ARK_LOCAL_EDIT_SIZE: &str = "1K";
const ARK_MAX_REFERENCES: usize = 4;
const ARK_RETRIES: usize = 2;
const SILICONFLOW_IMAGE_FIELDS: [&str; 3] = ["image", "image2", "image3"];
const SILICONFLOW_NEGATIVE_PROMPT: &str = "text, watermark, logo, caption, realistic human skin, realistic human eyes, multiple characters";
const SILICONFLOW_RETRIES: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_INPUT_SIDE: u32 = 2048;

/// The `supported_parameters` of every model, by base URL, with when they
/// were read. Cleared on restart.
pub type ModelCache = Mutex<HashMap<String, (Instant, HashMap<String, JsonObject>)>>;

#[derive(Debug, Clone)]
pub enum HostedApi {
    OpenRouter(OpenRouter),
    Ark(Ark),
    SiliconFlow(SiliconFlow),
}

impl HostedApi {
    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenRouter(_) => "openrouter",
            Self::Ark(_) => "ark",
            Self::SiliconFlow(_) => "siliconflow",
        }
    }

    pub fn api(&self) -> &dyn ImageApi {
        match self {
            Self::OpenRouter(api) => api,
            Self::Ark(api) => api,
            Self::SiliconFlow(api) => api,
        }
    }

    pub fn reference_limit(&self) -> usize {
        match self {
            Self::OpenRouter(api) => {
                let model_max = api
                    .parameters
                    .as_ref()
                    .and_then(|parameters| parameters.get("input_references"))
                    .and_then(|spec| spec.get("max"))
                    .and_then(Value::as_u64)
                    .and_then(|max| usize::try_from(max).ok())
                    .filter(|max| *max > 0);
                model_max.map_or(OPENROUTER_MAX_REFERENCES, |max| {
                    max.min(OPENROUTER_MAX_REFERENCES)
                })
            }
            Self::Ark(_) => ARK_MAX_REFERENCES,
            Self::SiliconFlow(_) => SILICONFLOW_IMAGE_FIELDS.len(),
        }
    }

    pub fn prompt_limit(&self) -> Option<usize> {
        match self {
            Self::OpenRouter(api) => prompt_limit_for_model(&api.model),
            _ => None,
        }
    }

    /// The width:height ratios the request picks from (the nearest to the
    /// first image's), or `None` when it asks for no shape. Ark sends a size
    /// class (`1K`) and SiliconFlow no size, so their models shape the
    /// answer themselves.
    pub fn aspect_ratios(&self) -> Option<Vec<String>> {
        match self {
            Self::OpenRouter(api) => api.aspect_ratios(),
            Self::Ark(_) | Self::SiliconFlow(_) => None,
        }
    }
}

fn base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_owned()
}

fn required(value: String, setting: &str, provider: &str) -> Result<String, Failure> {
    if value.is_empty() {
        Err(Failure::missing_settings(
            &[setting],
            format!("{setting} is required for image generation with {provider}"),
        ))
    } else {
        Ok(value)
    }
}

// --- OpenRouter --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OpenRouter {
    pub base_url: String,
    api_key: String,
    pub model: String,
    extra_body: JsonObject,
    /// The model's `supported_parameters`, read before each job; `None` =
    /// unknown.
    pub parameters: Option<JsonObject>,
}

impl OpenRouter {
    pub fn from_settings(settings: &Settings) -> Result<Self, Failure> {
        let model = required(
            settings.get("openrouter_image_model").trim().to_owned(),
            "openrouter_image_model",
            "openrouter",
        )?;
        let api_key = required(
            settings.openrouter_api_key(),
            "openrouter_api_key",
            "openrouter",
        )?;
        let extra_body = chat_api::parse_extra_body(settings.get("openrouter_extra_body"))
            .unwrap_or_else(|_| {
                log::warn!("ignoring invalid openrouter_extra_body");
                JsonObject::new()
            });
        Ok(Self {
            base_url: base(settings.get("openrouter_base_url")),
            api_key,
            model,
            extra_body,
            parameters: None,
        })
    }

    /// Reads the model's `supported_parameters`, from the cache when it is
    /// fresh.
    pub async fn load_parameters(&mut self, client: &reqwest::Client, cache: &ModelCache) {
        self.parameters =
            model_parameters(client, cache, &self.base_url, &self.api_key, &self.model).await;
    }

    /// The ratios `aspect_ratio` is picked from (see
    /// [`HostedApi::aspect_ratios`]); an `aspect_ratio` in the extra fields
    /// replaces the pick, and a null one removes it.
    pub fn aspect_ratios(&self) -> Option<Vec<String>> {
        match self.extra_body.get("aspect_ratio") {
            None => self.model_aspect_ratios(),
            Some(Value::String(ratio)) => Some(vec![ratio.clone()]),
            Some(_) => None,
        }
    }

    /// The model's own ratios, the documented ones when the model is
    /// unknown, or `None` when it takes no ratio.
    fn model_aspect_ratios(&self) -> Option<Vec<String>> {
        let all = || ASPECT_RATIOS.map(str::to_owned).to_vec();
        match &self.parameters {
            None => Some(all()),
            Some(parameters) if parameters.contains_key("aspect_ratio") => {
                Some(enum_values(parameters, "aspect_ratio").unwrap_or_else(all))
            }
            Some(_) => None,
        }
    }
}

pub fn prompt_limit_for_model(model: &str) -> Option<usize> {
    let model = model.trim().to_lowercase();
    PROMPT_LIMITS
        .iter()
        .find(|(prefix, _)| model.starts_with(prefix))
        .map(|(_, limit)| *limit)
}

/// `"3:2"` as 1.5; `None` for anything else (`"auto"`).
pub fn ratio_value(ratio: &str) -> Option<f64> {
    let (width, height) = ratio.split_once(':')?;
    let (width, height): (f64, f64) = (width.parse().ok()?, height.parse().ok()?);
    (width > 0.0 && height > 0.0).then(|| width / height)
}

/// The allowed ratio closest to `width:height` (the first of equals).
pub fn nearest_aspect_ratio(width: u32, height: u32, allowed: &[String]) -> Option<String> {
    let target = f64::from(width) / f64::from(height);
    let mut best: Option<(&String, f64)> = None;
    for ratio in allowed {
        let Some(value) = ratio_value(ratio) else {
            continue;
        };
        let distance = (value - target).abs();
        if best.is_none_or(|(_, best_distance)| distance < best_distance) {
            best = Some((ratio, distance));
        }
    }
    best.map(|(ratio, _)| ratio.clone())
}

/// The model's `supported_parameters` (possibly empty), or `None` when the
/// listing is unavailable or does not have the model.
async fn model_parameters(
    client: &reqwest::Client,
    cache: &ModelCache,
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Option<JsonObject> {
    let fresh = |cache: &ModelCache| {
        let cache = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .get(base_url)
            .filter(|(read_at, _)| read_at.elapsed() <= MODELS_TTL)
            .map(|(_, models)| models.get(model).cloned())
    };
    let parameters = match fresh(cache) {
        Some(parameters) => parameters,
        None => {
            let models = match fetch_models(client, base_url, api_key).await {
                Ok(models) => models,
                Err(error) => {
                    log::warn!(
                        "OpenRouter image model list unavailable ({error}); using default parameters"
                    );
                    return None;
                }
            };
            let parameters = models.get(model).cloned();
            cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(base_url.to_owned(), (Instant::now(), models));
            parameters
        }
    };
    if parameters.is_none() {
        log::warn!(
            "OpenRouter image model {model} is not in the model list; using default parameters"
        );
    }
    parameters
}

async fn fetch_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<HashMap<String, JsonObject>, String> {
    let mut request = client
        .get(format!("{base_url}/images/models"))
        .timeout(MODELS_TIMEOUT);
    if !api_key.is_empty() {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let body: Value = response.json().await.map_err(|error| error.to_string())?;
    let items = match &body {
        Value::Object(object) => object.get("data").and_then(Value::as_array),
        Value::Array(items) => Some(items),
        _ => None,
    }
    .ok_or("unexpected model list")?;
    Ok(items
        .iter()
        .filter_map(|item| {
            let id = match item.get("id")? {
                Value::String(id) if !id.is_empty() => id.clone(),
                Value::String(_) | Value::Null => return None,
                other => other.to_string(),
            };
            let parameters = match item.get("supported_parameters") {
                Some(Value::Object(parameters)) => parameters.clone(),
                _ => JsonObject::new(),
            };
            Some((id, parameters))
        })
        .collect())
}

fn enum_values(parameters: &JsonObject, name: &str) -> Option<Vec<String>> {
    let values = parameters.get(name)?.get("values")?.as_array()?;
    Some(
        values
            .iter()
            .map(|value| match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
    )
}

fn openrouter_resolution(mode: Mode) -> &'static str {
    match mode {
        Mode::Turnaround | Mode::CharacterTurnaround => OPENROUTER_TURNAROUND_RESOLUTION,
        Mode::FrontLocalRevision => OPENROUTER_LOCAL_EDIT_RESOLUTION,
        _ => OPENROUTER_FRONT_RESOLUTION,
    }
}

impl ImageApi for OpenRouter {
    fn display_name(&self) -> &'static str {
        "OpenRouter"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn timeout(&self) -> Duration {
        REQUEST_TIMEOUT
    }

    fn max_retries(&self) -> usize {
        OPENROUTER_RETRIES
    }

    fn retryable(&self, status: u16) -> bool {
        // 502 is how OpenRouter reports a failed (unbilled) generation
        // upstream.
        matches!(status, 429 | 500 | 502 | 503 | 504)
    }

    fn build_request(
        &self,
        prompt: &str,
        images: &[RgbImage],
        _trace_id: &str,
        mode: Mode,
    ) -> ApiRequest {
        let parameters = self.parameters.as_ref();
        let takes = |name: &str| parameters.is_none_or(|parameters| parameters.contains_key(name));
        let mut payload = JsonObject::new();
        payload.insert("model".to_owned(), json!(self.model));
        payload.insert("prompt".to_owned(), json!(prompt));
        if takes("n") {
            payload.insert("n".to_owned(), json!(1));
        }
        if let Some(first) = images.first() {
            let references: Vec<Value> = images
                .iter()
                .map(|image| {
                    json!({"type": "image_url", "image_url": {"url": to_data_uri(image, MAX_INPUT_SIDE)}})
                })
                .collect();
            payload.insert("input_references".to_owned(), Value::Array(references));
            if let Some(ratio) = self
                .model_aspect_ratios()
                .and_then(|allowed| nearest_aspect_ratio(first.width(), first.height(), &allowed))
            {
                payload.insert("aspect_ratio".to_owned(), json!(ratio));
            }
        }
        let resolution = openrouter_resolution(mode);
        let allowed = parameters.and_then(|parameters| enum_values(parameters, "resolution"));
        if takes("resolution")
            && allowed.is_none_or(|allowed| allowed.iter().any(|value| value == resolution))
        {
            payload.insert("resolution".to_owned(), json!(resolution));
        } else {
            log::info!(
                "OpenRouter model {} does not take resolution {resolution}; using its default",
                self.model
            );
        }
        // Extra fields may add vendor options; a null value removes a field
        // the backend would otherwise send.
        for (key, value) in &self.extra_body {
            if OWNED_FIELDS.contains(&key.as_str()) {
                continue;
            }
            if value.is_null() {
                payload.remove(key);
            } else {
                payload.insert(key.clone(), value.clone());
            }
        }
        ApiRequest {
            url: format!("{}/images", self.base_url),
            headers: vec![
                ("Authorization", format!("Bearer {}", self.api_key)),
                ("X-Title", "Maskette".to_owned()),
            ],
            payload,
        }
    }

    fn extract_image(&self, body: &Value) -> Option<ApiImage> {
        let item = body.get("data")?.get(0)?;
        match item.get("b64_json").and_then(Value::as_str) {
            // Some providers hand back a data URL instead of bare base64.
            Some(encoded) if !encoded.is_empty() => Some(ApiImage::Base64(
                encoded
                    .split_once(',')
                    .filter(|_| encoded.starts_with("data:"))
                    .map_or(encoded, |(_, data)| data)
                    .to_owned(),
            )),
            _ => item
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(|url| ApiImage::Url(url.to_owned())),
        }
    }
}

// --- Volcengine Ark ----------------------------------------------------------

/// Ark (Agent Plan) Seedream via `/images/generations`. References go into
/// one `image` list; the platform watermark is turned off and Maskette adds
/// its own AI label afterwards.
#[derive(Debug, Clone)]
pub struct Ark {
    base_url: String,
    api_key: String,
    model: String,
}

impl Ark {
    pub fn from_settings(settings: &Settings) -> Result<Self, Failure> {
        Ok(Self {
            api_key: required(
                settings.get("ark_api_key").trim().to_owned(),
                "ark_api_key",
                "ark",
            )?,
            base_url: base(settings.get("ark_base_url")),
            model: settings.get("ark_image_model").trim().to_owned(),
        })
    }
}

impl ImageApi for Ark {
    fn display_name(&self) -> &'static str {
        "Volcengine Ark"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn timeout(&self) -> Duration {
        REQUEST_TIMEOUT
    }

    fn max_retries(&self) -> usize {
        ARK_RETRIES
    }

    fn retryable(&self, status: u16) -> bool {
        matches!(status, 429 | 500 | 502 | 503 | 504)
    }

    fn build_request(
        &self,
        prompt: &str,
        images: &[RgbImage],
        trace_id: &str,
        mode: Mode,
    ) -> ApiRequest {
        let size = match mode {
            Mode::Turnaround | Mode::CharacterTurnaround => ARK_TURNAROUND_SIZE,
            Mode::FrontLocalRevision => ARK_LOCAL_EDIT_SIZE,
            _ => ARK_FRONT_SIZE,
        };
        let mut payload = JsonObject::new();
        payload.insert("model".to_owned(), json!(self.model));
        payload.insert("prompt".to_owned(), json!(prompt));
        payload.insert("size".to_owned(), json!(size));
        payload.insert("output_format".to_owned(), json!("png"));
        payload.insert("response_format".to_owned(), json!("url"));
        payload.insert("watermark".to_owned(), json!(false));
        let mut encoded: Vec<Value> = images
            .iter()
            .map(|image| json!(to_data_uri(image, MAX_INPUT_SIDE)))
            .collect();
        match encoded.len() {
            0 => {}
            1 => {
                payload.insert("image".to_owned(), encoded.remove(0));
            }
            _ => {
                payload.insert("image".to_owned(), Value::Array(encoded));
            }
        }
        ApiRequest {
            url: format!("{}/images/generations", self.base_url),
            headers: vec![
                ("Authorization", format!("Bearer {}", self.api_key)),
                ("X-Client-Request-Id", trace_id.to_owned()),
            ],
            payload,
        }
    }

    fn trace_id(&self, headers: &HeaderMap) -> Option<String> {
        ["x-request-id", "x-tt-logid"]
            .iter()
            .find_map(|name| headers.get(*name)?.to_str().ok().map(str::to_owned))
    }

    fn extract_image(&self, body: &Value) -> Option<ApiImage> {
        let item = body.get("data")?.get(0)?;
        if let Some(url) = item
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        {
            return Some(ApiImage::Url(url.to_owned()));
        }
        item.get("b64_json")
            .and_then(Value::as_str)
            .filter(|data| !data.is_empty())
            .map(|data| ApiImage::Base64(data.to_owned()))
    }
}

// --- SiliconFlow -------------------------------------------------------------

/// SiliconFlow `/images/generations`. Edit models take up to three images as
/// `image`, `image2` and `image3`.
#[derive(Debug, Clone)]
pub struct SiliconFlow {
    base_url: String,
    api_key: String,
    model: String,
}

impl SiliconFlow {
    pub fn from_settings(settings: &Settings) -> Result<Self, Failure> {
        Ok(Self {
            api_key: required(
                settings.get("siliconflow_api_key").trim().to_owned(),
                "siliconflow_api_key",
                "siliconflow",
            )?,
            base_url: base(settings.get("siliconflow_base_url")),
            model: settings.get("siliconflow_image_model").trim().to_owned(),
        })
    }
}

impl ImageApi for SiliconFlow {
    fn display_name(&self) -> &'static str {
        "SiliconFlow"
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn timeout(&self) -> Duration {
        REQUEST_TIMEOUT
    }

    fn max_retries(&self) -> usize {
        SILICONFLOW_RETRIES
    }

    fn retryable(&self, status: u16) -> bool {
        matches!(status, 429 | 503 | 504)
    }

    fn build_request(
        &self,
        prompt: &str,
        images: &[RgbImage],
        trace_id: &str,
        _mode: Mode,
    ) -> ApiRequest {
        let mut payload = JsonObject::new();
        payload.insert("model".to_owned(), json!(self.model));
        payload.insert("prompt".to_owned(), json!(prompt));
        for (field, image) in SILICONFLOW_IMAGE_FIELDS.iter().zip(images) {
            payload.insert(
                (*field).to_owned(),
                json!(to_data_uri(image, MAX_INPUT_SIDE)),
            );
        }
        payload.insert(
            "negative_prompt".to_owned(),
            json!(SILICONFLOW_NEGATIVE_PROMPT),
        );
        ApiRequest {
            url: format!("{}/images/generations", self.base_url),
            headers: vec![
                ("Authorization", format!("Bearer {}", self.api_key)),
                ("X-Enable-Watermark", "0".to_owned()),
                ("X-Trace-Id", trace_id.to_owned()),
            ],
            payload,
        }
    }

    fn trace_id(&self, headers: &HeaderMap) -> Option<String> {
        headers
            .get("x-siliconcloud-trace-id")?
            .to_str()
            .ok()
            .map(str::to_owned)
    }

    fn extract_image(&self, body: &Value) -> Option<ApiImage> {
        body.get("images")?
            .get(0)?
            .get("url")?
            .as_str()
            .filter(|url| !url.is_empty())
            .map(|url| ApiImage::Url(url.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ratios(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn the_nearest_aspect_ratio_wins() {
        let all = ratios(&ASPECT_RATIOS);
        assert_eq!(
            nearest_aspect_ratio(800, 1100, &all).as_deref(),
            Some("3:4")
        );
        assert_eq!(
            nearest_aspect_ratio(3000, 2000, &all).as_deref(),
            Some("3:2")
        );
        // Krea has no 3:4.
        assert_eq!(
            nearest_aspect_ratio(800, 1100, &ratios(&["1:1", "2:3", "16:9"])).as_deref(),
            Some("2:3")
        );
        assert_eq!(nearest_aspect_ratio(1, 1, &ratios(&["bad"])), None);
    }

    #[test]
    fn grok_imagine_has_a_prompt_limit() {
        assert_eq!(
            prompt_limit_for_model(" X-AI/Grok-Imagine-Image "),
            Some(8000)
        );
        assert_eq!(
            prompt_limit_for_model("google/gemini-nano-banana-2.1"),
            None
        );
    }

    fn openrouter(parameters: Option<Value>, extra_body: Value) -> OpenRouter {
        OpenRouter {
            base_url: "https://example.test/api/v1".to_owned(),
            api_key: "key".to_owned(),
            model: "vendor/model".to_owned(),
            extra_body: extra_body.as_object().cloned().unwrap_or_default(),
            parameters: parameters.and_then(|value| value.as_object().cloned()),
        }
    }

    #[test]
    fn openrouter_sends_only_what_the_model_takes() {
        let image = RgbImage::new(800, 1100);
        let unknown = openrouter(None, json!({})).build_request(
            "p",
            std::slice::from_ref(&image),
            "t",
            Mode::Turnaround,
        );
        assert_eq!(unknown.url, "https://example.test/api/v1/images");
        assert_eq!(unknown.payload["n"], 1);
        assert_eq!(unknown.payload["aspect_ratio"], "3:4");
        assert_eq!(unknown.payload["resolution"], "2K");
        assert_eq!(
            unknown.payload["input_references"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let known = openrouter(
            Some(json!({
                "aspect_ratio": {"values": ["1:1", "2:3"]},
                "resolution": {"values": ["1K"]},
                "input_references": {"max": 2},
            })),
            json!({"quality": "high", "model": "other", "aspect_ratio": null}),
        );
        let request = known.build_request("p", &[image], "t", Mode::FrontDesign);
        assert!(!request.payload.contains_key("n"));
        assert!(!request.payload.contains_key("aspect_ratio"));
        assert_eq!(request.payload["resolution"], "1K");
        assert_eq!(request.payload["quality"], "high");
        assert_eq!(request.payload["model"], "vendor/model");
        assert_eq!(HostedApi::OpenRouter(known.clone()).reference_limit(), 2);
        let turnaround = known.build_request("p", &[], "t", Mode::Turnaround);
        assert!(!turnaround.payload.contains_key("resolution"));
        assert!(!turnaround.payload.contains_key("input_references"));
    }

    #[test]
    fn the_ratios_to_crop_to_are_the_ones_the_request_picks_from() {
        let all = Some(ratios(&ASPECT_RATIOS));
        assert_eq!(openrouter(None, json!({})).aspect_ratios(), all);
        let listed = json!({"aspect_ratio": {"values": ["1:1", "3:2", "auto"]}});
        assert_eq!(
            openrouter(Some(listed.clone()), json!({})).aspect_ratios(),
            Some(ratios(&["1:1", "3:2", "auto"]))
        );
        assert_eq!(
            openrouter(Some(json!({"aspect_ratio": {}})), json!({})).aspect_ratios(),
            all
        );
        assert_eq!(
            openrouter(Some(json!({"resolution": {}})), json!({})).aspect_ratios(),
            None
        );
        // The extra fields win: one ratio, or none at all.
        let fixed = openrouter(Some(listed.clone()), json!({"aspect_ratio": "16:9"}));
        assert_eq!(fixed.aspect_ratios(), Some(ratios(&["16:9"])));
        let image = RgbImage::new(624, 416);
        let sent = fixed.build_request("p", &[image], "t", Mode::FrontLocalRevision);
        assert_eq!(sent.payload["aspect_ratio"], "16:9");
        let removed = openrouter(Some(listed), json!({"aspect_ratio": null}));
        assert_eq!(removed.aspect_ratios(), None);
        assert_eq!(HostedApi::OpenRouter(removed).aspect_ratios(), None);
        let ark = Ark {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        };
        assert_eq!(HostedApi::Ark(ark).aspect_ratios(), None);
        let silicon = SiliconFlow {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        };
        assert_eq!(HostedApi::SiliconFlow(silicon).aspect_ratios(), None);
    }

    #[test]
    fn answers_are_read_per_api() {
        let api = openrouter(None, json!({}));
        assert!(matches!(
            api.extract_image(&json!({"data": [{"b64_json": "data:image/png;base64,QUJD"}]})),
            Some(ApiImage::Base64(data)) if data == "QUJD"
        ));
        assert!(matches!(
            api.extract_image(&json!({"data": [{"url": "https://x/y.png"}]})),
            Some(ApiImage::Url(url)) if url == "https://x/y.png"
        ));
        assert!(api.extract_image(&json!({"data": []})).is_none());
        let silicon = SiliconFlow {
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
        };
        assert!(matches!(
            silicon.extract_image(&json!({"images": [{"url": "u"}]})),
            Some(ApiImage::Url(url)) if url == "u"
        ));
    }
}

//! The shared pipeline of the hosted image APIs (the old
//! `app/generation/backends/image_api.py`).
//!
//! Each API ([`super::openrouter`], [`super::ark`], [`super::siliconflow`])
//! only builds its request and reads the image from the answer; picking and
//! ordering the references, the output sizes, local regeneration, retries,
//! the download, the watermark and storage live here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use image::imageops::{self, FilterType};
use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use reqwest::header::HeaderMap;
use serde_json::{Value, json};

use super::prompt::{self, Role};
use super::{
    FRONT_SIZE, GeneratedImage, GenerationRequest, LocalEdit, Mode, TURNAROUND_SIZE, clean_root,
    generated_root, generated_url, watermark,
};
use crate::backend::chat_api::JsonObject;
use crate::backend::prompts::Overrides;
use crate::backend::references;

/// Waits between retries of a failed request.
pub const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(45),
];
const CONTACT_SHEET_HEIGHT: u32 = 1024;
const CONTACT_SHEET_GAP: u32 = 32;
const LOCAL_EDIT_MARGIN_RATIO: f64 = 0.15;
const LOCAL_EDIT_MIN_SIDE: u32 = 384;
/// How much of an error body is kept in the message.
const ERROR_TEXT_CHARS: usize = 300;

/// One request to a hosted image API.
pub struct ApiRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub payload: JsonObject,
}

/// What a backend reads from a successful answer.
pub enum ApiImage {
    Url(String),
    Base64(String),
}

/// A user reference attached to the job: `<kind>:references/...`.
struct Reference {
    kind: String,
    path: PathBuf,
}

/// The images to send, the prompt, and how to turn the answer into the
/// final output.
pub struct Prepared {
    pub prompt: String,
    pub images: Vec<RgbImage>,
    finish: Finish,
}

enum Finish {
    Front,
    Turnaround,
    Local {
        base: RgbImage,
        mask: GrayImage,
        crop: Crop,
        feather: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Crop {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl Crop {
    fn width(self) -> u32 {
        self.right - self.left
    }

    fn height(self) -> u32 {
        self.bottom - self.top
    }
}

/// The references, images and prompt for a job. `max_images` is how many
/// images the API takes; `max_prompt_chars` its prompt limit, if any.
pub fn prepare(
    data_dir: &Path,
    request: &GenerationRequest,
    max_images: usize,
    max_prompt_chars: Option<usize>,
) -> Result<Prepared, String> {
    match &request.local_edit {
        Some(edit) => prepare_local_revision(data_dir, request, edit),
        None => prepare_generation(data_dir, request, max_images.max(1), max_prompt_chars),
    }
}

fn prepare_generation(
    data_dir: &Path,
    request: &GenerationRequest,
    max_images: usize,
    max_prompt_chars: Option<usize>,
) -> Result<Prepared, String> {
    let mode = request.generation_mode;
    let references = user_references(data_dir, &request.reference_keys);
    // An approved stage-1 design sheet outranks the raw character references.
    let primary_index = references
        .iter()
        .position(|reference| reference.kind == "design")
        .or_else(|| {
            references
                .iter()
                .position(|reference| reference.kind == "front")
        })
        .ok_or("Image generation requires a front reference image")?;
    let primary = &references[primary_index];
    let is_turnaround = is_turnaround(mode);
    let is_design = matches!(mode, Mode::FrontDesign | Mode::CharacterFront);
    // Revision and turnaround already put the annotations onto the edited
    // front image.
    let others: Vec<&Reference> = references
        .iter()
        .enumerate()
        .filter(|(index, reference)| {
            *index != primary_index && (is_design || reference.kind != "annotation")
        })
        .map(|(_, reference)| reference)
        .collect();

    let (width, height) = mode.output_size();
    let target_ratio = f64::from(width) / f64::from(height);
    let mut images = vec![pad_to_ratio(
        &load_rgb(&primary.path)?,
        target_ratio,
        Rgb([255, 255, 255]),
    )];
    let mut roles = vec![if primary.kind == "design" {
        Role::ApprovedDesign
    } else if mode == Mode::CharacterRevision {
        Role::DesignDraft
    } else if is_design || mode == Mode::CharacterTurnaround {
        Role::Character
    } else {
        Role::ApprovedFront
    }];

    let remaining = max_images.saturating_sub(images.len());
    if !others.is_empty() && remaining > 0 {
        let (separate, merged) = if others.len() <= remaining {
            (&others[..], &others[others.len()..])
        } else {
            others.split_at(remaining - 1)
        };
        for reference in separate {
            images.push(load_rgb(&reference.path)?);
            roles.push(if reference.kind == "design_extra" {
                Role::DesignSheet
            } else {
                Role::Supplemental
            });
        }
        if !merged.is_empty() {
            let merged: Vec<RgbImage> = merged
                .iter()
                .map(|reference| load_rgb(&reference.path))
                .collect::<Result<_, _>>()?;
            images.push(contact_sheet(&merged));
            roles.push(Role::Supplemental);
        }
    }

    Ok(Prepared {
        prompt: prompt::build_image_prompt(
            request,
            &roles,
            max_prompt_chars,
            &Overrides::load(data_dir),
        ),
        images,
        finish: if is_turnaround {
            Finish::Turnaround
        } else {
            Finish::Front
        },
    })
}

fn prepare_local_revision(
    data_dir: &Path,
    request: &GenerationRequest,
    edit: &LocalEdit,
) -> Result<Prepared, String> {
    if !edit.base_image_path.is_file() || !edit.mask_image_path.is_file() {
        return Err("Local revision base or mask image is missing".to_owned());
    }
    let base = load_rgb(&edit.base_image_path)?;
    let mask = load_mask(&edit.mask_image_path)?;
    if mask.dimensions() != base.dimensions() {
        return Err("Local revision mask does not match the base image".to_owned());
    }
    let crop = local_edit_crop(&mask)?;
    let supplemental: Vec<RgbImage> = user_references(data_dir, &request.reference_keys)
        .iter()
        .map(|reference| load_rgb(&reference.path))
        .collect::<Result<_, _>>()?;
    let mut images = vec![
        imageops::crop_imm(&base, crop.left, crop.top, crop.width(), crop.height()).to_image(),
    ];
    match supplemental.len() {
        0 => {}
        1 => images.extend(supplemental),
        _ => images.push(contact_sheet(&supplemental)),
    }
    let note = if edit.edit_note.trim().is_empty() {
        &request.free_text
    } else {
        &edit.edit_note
    };
    Ok(Prepared {
        prompt: prompt::build_local_revision_prompt(note, images.len() > 1, edit.subject),
        images,
        finish: Finish::Local {
            base,
            mask,
            crop,
            feather: edit.feather_radius_px,
        },
    })
}

impl Prepared {
    /// The final output from the API's image.
    pub fn finish(&self, raw: &RgbImage) -> Result<RgbImage, String> {
        match &self.finish {
            Finish::Front => Ok(fit_cover(raw, FRONT_SIZE)),
            Finish::Turnaround => {
                // Keep the model's native resolution (no upscaling). Pad to
                // 3:2 with the background colour instead of cropping, which
                // can cut off the outermost view.
                let (width, height) = TURNAROUND_SIZE;
                let padded =
                    pad_to_ratio(raw, f64::from(width) / f64::from(height), edge_colour(raw));
                Ok(if padded.width() > width {
                    imageops::resize(&padded, width, height, FilterType::Lanczos3)
                } else {
                    padded
                })
            }
            Finish::Local {
                base,
                mask,
                crop,
                feather,
            } => {
                let edited = fit_cover(raw, (crop.width(), crop.height()));
                let mut pasted = base.clone();
                imageops::replace(
                    &mut pasted,
                    &edited,
                    i64::from(crop.left),
                    i64::from(crop.top),
                );
                Ok(composite_local_edit(base, mask, &pasted, *feather))
            }
        }
    }
}

fn is_turnaround(mode: Mode) -> bool {
    matches!(mode, Mode::Turnaround | Mode::CharacterTurnaround)
}

/// The attached user references that exist on disk, in order. Keys look
/// like `<kind>:references/<folder>/<file>`; anything else is skipped.
fn user_references(data_dir: &Path, keys: &[String]) -> Vec<Reference> {
    keys.iter()
        .filter_map(|key| reference_file(data_dir, key))
        .map(|(kind, path)| Reference { kind, path })
        .collect()
}

/// The kind and file of a reference key (`<kind>:references/<folder>/<file>`,
/// a bare object key counts as supplemental), when the file exists.
pub fn reference_file(data_dir: &Path, key: &str) -> Option<(String, PathBuf)> {
    if key.contains("://") || key.starts_with("//") {
        return None;
    }
    let (kind, object_key) = key.split_once(':').unwrap_or(("supplemental", key));
    let relative = object_key.replace('\\', "/");
    let relative = relative.strip_prefix("references/")?;
    if relative.split('/').filter(|part| !part.is_empty()).count() < 2 {
        return None;
    }
    let path = super::join_plain(&references::root(data_dir), relative)?;
    path.is_file().then(|| (kind.to_owned(), path))
}

// --- Images ----------------------------------------------------------------

/// An image as RGB; transparency is flattened onto white.
pub fn load_rgb(path: &Path) -> Result<RgbImage, String> {
    let image = open(path)?;
    Ok(flatten(image))
}

pub fn decode_rgb(bytes: &[u8]) -> Result<RgbImage, String> {
    image::load_from_memory(bytes)
        .map(flatten)
        .map_err(|error| format!("unreadable image: {error}"))
}

fn open(path: &Path) -> Result<DynamicImage, String> {
    image::ImageReader::open(path)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(|error| format!("cannot read {}: {error}", file_name(path)))?
        .decode()
        .map_err(|error| format!("cannot read {}: {error}", file_name(path)))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn flatten(image: DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |channel: u8| {
            let value = u32::from(channel) * u32::from(a) + 255 * (255 - u32::from(a));
            u8::try_from((value + 127) / 255).unwrap_or(u8::MAX)
        };
        Rgb([over(r), over(g), over(b)])
    })
}

/// The mask as one channel: its alpha when it has one, else its grey.
fn load_mask(path: &Path) -> Result<GrayImage, String> {
    let image = open(path)?;
    if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        Ok(GrayImage::from_fn(rgba.width(), rgba.height(), |x, y| {
            Luma([rgba.get_pixel(x, y).0[3]])
        }))
    } else {
        Ok(image.to_luma8())
    }
}

/// Centres the image on a canvas of the given width:height ratio.
pub fn pad_to_ratio(image: &RgbImage, ratio: f64, fill: Rgb<u8>) -> RgbImage {
    let (width, height) = image.dimensions();
    let current = f64::from(width) / f64::from(height);
    if (current - ratio).abs() < 0.01 {
        return image.clone();
    }
    let (canvas_width, canvas_height) = if current < ratio {
        (round_u32(f64::from(height) * ratio), height)
    } else {
        (width, round_u32(f64::from(width) / ratio))
    };
    let mut canvas = RgbImage::from_pixel(canvas_width, canvas_height, fill);
    imageops::replace(
        &mut canvas,
        image,
        i64::from((canvas_width - width) / 2),
        i64::from((canvas_height - height) / 2),
    );
    canvas
}

fn round_u32(value: f64) -> u32 {
    // Sizes here are a few thousand pixels at most.
    value.round().max(0.0) as u32
}

/// Median colour of the outer border, used to extend studio backgrounds
/// seamlessly.
pub fn edge_colour(image: &RgbImage) -> Rgb<u8> {
    let (width, height) = image.dimensions();
    let mut pixels: Vec<[u8; 3]> = Vec::new();
    for x in 0..width {
        pixels.push(image.get_pixel(x, 0).0);
        pixels.push(image.get_pixel(x, height - 1).0);
    }
    for y in 0..height {
        pixels.push(image.get_pixel(0, y).0);
        pixels.push(image.get_pixel(width - 1, y).0);
    }
    let median = |channel: usize| {
        let mut values: Vec<u8> = pixels.iter().map(|pixel| pixel[channel]).collect();
        values.sort_unstable();
        values[values.len() / 2]
    };
    Rgb([median(0), median(1), median(2)])
}

/// Scales the image to cover `size` and crops the centre.
pub fn fit_cover(image: &RgbImage, (target_width, target_height): (u32, u32)) -> RgbImage {
    let (width, height) = image.dimensions();
    let scale = (f64::from(target_width) / f64::from(width))
        .max(f64::from(target_height) / f64::from(height));
    let resized_width = target_width.max(round_u32(f64::from(width) * scale));
    let resized_height = target_height.max(round_u32(f64::from(height) * scale));
    let resized = if (resized_width, resized_height) == (width, height) {
        image.clone()
    } else {
        imageops::resize(image, resized_width, resized_height, FilterType::Lanczos3)
    };
    let left = (resized_width - target_width) / 2;
    let top = (resized_height - target_height) / 2;
    imageops::crop_imm(&resized, left, top, target_width, target_height).to_image()
}

/// Several references side by side at one height, as one image.
fn contact_sheet(images: &[RgbImage]) -> RgbImage {
    let scaled: Vec<RgbImage> = images
        .iter()
        .map(|image| {
            let width = round_u32(
                f64::from(image.width()) * f64::from(CONTACT_SHEET_HEIGHT)
                    / f64::from(image.height()),
            )
            .max(1);
            imageops::resize(image, width, CONTACT_SHEET_HEIGHT, FilterType::CatmullRom)
        })
        .collect();
    let gaps = CONTACT_SHEET_GAP * u32::try_from(scaled.len().saturating_sub(1)).unwrap_or(0);
    let width = scaled.iter().map(RgbImage::width).sum::<u32>() + gaps;
    let mut sheet = RgbImage::from_pixel(width, CONTACT_SHEET_HEIGHT, Rgb([255, 255, 255]));
    let mut x = 0;
    for image in &scaled {
        imageops::replace(&mut sheet, image, i64::from(x), 0);
        x += image.width() + CONTACT_SHEET_GAP;
    }
    sheet
}

/// The image shrunk so its longer side is at most `max_side` (0 = as is).
pub fn downscale(image: &RgbImage, max_side: u32) -> RgbImage {
    let longest = image.width().max(image.height());
    if max_side == 0 || longest <= max_side {
        return image.clone();
    }
    let scale = f64::from(max_side) / f64::from(longest);
    imageops::resize(
        image,
        round_u32(f64::from(image.width()) * scale).max(1),
        round_u32(f64::from(image.height()) * scale).max(1),
        FilterType::Lanczos3,
    )
}

/// The image as a PNG data URL, shrunk to `max_side`.
pub fn to_data_uri(image: &RgbImage, max_side: u32) -> String {
    let mut bytes = Vec::new();
    // Encoding an in-memory RGB image as PNG cannot fail.
    downscale(image, max_side)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("PNG encoding");
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// The masked area with a margin, grown to at least 384 pixels a side.
fn local_edit_crop(mask: &GrayImage) -> Result<Crop, String> {
    let (width, height) = mask.dimensions();
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in mask.enumerate_pixels() {
        if pixel.0[0] > 0 {
            bounds = Some(match bounds {
                None => (x, y, x + 1, y + 1),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1))
                }
            });
        }
    }
    let (left, top, right, bottom) = bounds.ok_or("Local revision mask is empty")?;
    let margin =
        |length: u32| i64::from(16_u32.max(round_u32(f64::from(length) * LOCAL_EDIT_MARGIN_RATIO)));
    let (margin_x, margin_y) = (margin(right - left), margin(bottom - top));
    let (left, right) = grow_span(
        i64::from(left) - margin_x,
        i64::from(right) + margin_x,
        width,
    );
    let (top, bottom) = grow_span(
        i64::from(top) - margin_y,
        i64::from(bottom) + margin_y,
        height,
    );
    Ok(Crop {
        left,
        top,
        right,
        bottom,
    })
}

fn grow_span(mut start: i64, mut end: i64, limit: u32) -> (u32, u32) {
    let (min_length, limit) = (i64::from(LOCAL_EDIT_MIN_SIDE), i64::from(limit));
    let length = end - start;
    if length < min_length {
        let extra = min_length - length;
        start -= extra / 2;
        end += extra - extra / 2;
    }
    if start < 0 {
        end -= start;
        start = 0;
    }
    if end > limit {
        start -= end - limit;
        end = limit;
    }
    let clamp = |value: i64| u32::try_from(value.clamp(0, limit)).unwrap_or(0);
    (clamp(start), clamp(end))
}

/// The edited pixels inside the mask, the base outside. The mask's edge is
/// feathered inwards only, so nothing outside it changes.
fn composite_local_edit(
    base: &RgbImage,
    mask: &GrayImage,
    edited: &RgbImage,
    feather: u32,
) -> RgbImage {
    let binary = GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        Luma([if mask.get_pixel(x, y).0[0] > 0 {
            255
        } else {
            0
        }])
    });
    let alpha = if feather > 0 {
        // Radii are small, so the cast is exact.
        imageops::fast_blur(&binary, feather as f32)
    } else {
        binary.clone()
    };
    RgbImage::from_fn(base.width(), base.height(), |x, y| {
        let weight = if binary.get_pixel(x, y).0[0] == 0 {
            0
        } else {
            u32::from(alpha.get_pixel(x, y).0[0])
        };
        let (from, to) = (base.get_pixel(x, y).0, edited.get_pixel(x, y).0);
        let mix = |channel: usize| {
            let value = u32::from(to[channel]) * weight + u32::from(from[channel]) * (255 - weight);
            u8::try_from((value + 127) / 255).unwrap_or(u8::MAX)
        };
        Rgb([mix(0), mix(1), mix(2)])
    })
}

// --- Requests --------------------------------------------------------------

/// How one hosted image API differs from the others.
pub trait ImageApi: Send + Sync {
    /// The name in error messages.
    fn display_name(&self) -> &'static str;
    fn model(&self) -> &str;
    fn timeout(&self) -> Duration;
    fn max_retries(&self) -> usize;
    fn retryable(&self, status: u16) -> bool;
    fn build_request(
        &self,
        prompt: &str,
        images: &[RgbImage],
        trace_id: &str,
        mode: Mode,
    ) -> ApiRequest;
    fn trace_id(&self, _headers: &HeaderMap) -> Option<String> {
        None
    }
    fn extract_image(&self, body: &Value) -> Option<ApiImage>;
}

/// Sends the request, retrying what the API marks as retryable, and returns
/// the image bytes.
pub async fn request_image(
    client: &reqwest::Client,
    api: &dyn ImageApi,
    request: &ApiRequest,
    trace_id: &str,
    retry_delays: &[Duration],
) -> Result<Vec<u8>, String> {
    let name = api.display_name();
    let max_retries = api.max_retries();
    let mut attempt = 0;
    loop {
        let started = std::time::Instant::now();
        let mut builder = client
            .post(&request.url)
            .timeout(api.timeout())
            .json(&request.payload);
        for (header, value) in &request.headers {
            builder = builder.header(*header, value);
        }
        let (failure, retryable) = match builder.send().await {
            Err(error) if error.is_timeout() => {
                // The server accepted the job but never answered; retrying
                // would multiply the wait without changing the outcome.
                return Err(format!(
                    "{name} did not respond within {}s (model {}, trace_id={trace_id}); the model may be overloaded",
                    api.timeout().as_secs(),
                    api.model()
                ));
            }
            Err(error) => (format!("request failed: {error}"), true),
            Ok(response) => {
                let status = response.status().as_u16();
                let server_trace = api
                    .trace_id(response.headers())
                    .unwrap_or_else(|| trace_id.to_owned());
                let text = match response.text().await {
                    Ok(text) => text,
                    Err(error) if error.is_timeout() => {
                        return Err(format!(
                            "{name} did not finish its answer within {}s (trace_id={server_trace})",
                            api.timeout().as_secs()
                        ));
                    }
                    Err(error) => format!("unreadable answer: {error}"),
                };
                if status == 200 {
                    let image = serde_json::from_str::<Value>(&text)
                        .ok()
                        .and_then(|body| api.extract_image(&body))
                        .ok_or_else(|| format!("{name} response did not include an image"))?;
                    log::info!(
                        "{name} image generated trace_id={server_trace} duration={:.1}s attempt={}",
                        started.elapsed().as_secs_f64(),
                        attempt + 1
                    );
                    return match image {
                        ApiImage::Base64(data) => base64::engine::general_purpose::STANDARD
                            .decode(data.trim())
                            .map_err(|_| format!("{name} response did not include an image")),
                        ApiImage::Url(url) => download(client, &url, name, api.timeout()).await,
                    };
                }
                (
                    format!(
                        "HTTP {status} trace_id={server_trace}: {}",
                        error_message(&text)
                    ),
                    api.retryable(status),
                )
            }
        };
        if !retryable || attempt >= max_retries {
            return Err(format!("{name} image generation failed: {failure}"));
        }
        let delay = retry_delays
            .get(attempt)
            .or(retry_delays.last())
            .copied()
            .unwrap_or_default();
        log::warn!(
            "{name} request failed ({failure}); retrying in {}s",
            delay.as_secs()
        );
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

/// Hosted APIs return short-lived URLs, so the image is fetched at once.
async fn download(
    client: &reqwest::Client,
    url: &str,
    name: &str,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .timeout(timeout)
        .send()
        .await
        .map_err(|error| format!("Failed to download {name} image: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download {name} image: HTTP {}",
            response.status().as_u16()
        ));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("Failed to download {name} image: {error}"))
}

/// The useful part of an error body.
fn error_message(text: &str) -> String {
    let message = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(body)) => match body.get("error") {
            Some(Value::Object(error)) => error
                .get("message")
                .or_else(|| error.get("code"))
                .map_or_else(|| Value::Object(error.clone()).to_string(), plain),
            _ => body
                .get("message")
                .map_or_else(|| Value::Object(body.clone()).to_string(), plain),
        },
        Ok(other) => other.to_string(),
        Err(_) => text.to_owned(),
    };
    message.chars().take(ERROR_TEXT_CHARS).collect()
}

fn plain(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// ASCII letters, digits and `_.-` only, for folder names.
pub fn safe_segment(value: &str) -> String {
    let replaced: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let mut collapsed = String::new();
    for c in replaced.chars() {
        if !(c == '-' && collapsed.ends_with('-')) {
            collapsed.push(c);
        }
    }
    let trimmed = collapsed.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        "unknown".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Writes `image` as a PNG file.
pub fn save_png(image: &RgbImage, path: &Path) -> Result<(), String> {
    image
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|error| format!("cannot write {}: {error}", file_name(path)))
}

/// Writes the clean output and its watermarked public copy.
pub fn store_output(
    data_dir: &Path,
    request: &GenerationRequest,
    job_id: &str,
    image: &RgbImage,
) -> Result<GeneratedImage, String> {
    let relative = format!(
        "{}/{}/candidate-1.png",
        safe_segment(&request.character_session_id),
        safe_segment(job_id)
    );
    let clean_path = clean_root(data_dir).join(&relative);
    let public_path = generated_root(data_dir).join(&relative);
    let mut public = image.clone();
    watermark::apply(&mut public);
    for (path, pixels) in [(&clean_path, image), (&public_path, &public)] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        pixels
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|error| format!("cannot save the generated image: {error}"))?;
    }
    Ok(GeneratedImage {
        job_id: job_id.to_owned(),
        url: generated_url(&relative),
        clean_path,
        width: image.width(),
        height: image.height(),
    })
}

/// A short description of the job's request, for the job record.
pub fn request_summary(api: &ApiRequest) -> Value {
    let mut payload = api.payload.clone();
    for key in ["input_references", "image", "image2", "image3"] {
        if let Some(value) = payload.get_mut(key) {
            let count = value.as_array().map_or(1, Vec::len);
            *value = json!(format!("[{count} image(s)]"));
        }
    }
    payload.remove("prompt");
    json!({"url": api.url, "payload": payload})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_centres_the_image_on_the_ratio() {
        let image = RgbImage::from_pixel(100, 100, Rgb([0, 0, 0]));
        let padded = pad_to_ratio(&image, 1.5, Rgb([255, 255, 255]));
        assert_eq!(padded.dimensions(), (150, 100));
        assert_eq!(padded.get_pixel(0, 50).0, [255, 255, 255]);
        assert_eq!(padded.get_pixel(75, 50).0, [0, 0, 0]);
        let tall = pad_to_ratio(&image, 800.0 / 1100.0, Rgb([255, 255, 255]));
        assert_eq!(tall.dimensions(), (100, 138));
        assert_eq!(pad_to_ratio(&image, 1.0, Rgb([1, 1, 1])), image);
    }

    #[test]
    fn cover_fills_and_crops_the_centre() {
        let mut image = RgbImage::from_pixel(200, 100, Rgb([0, 0, 0]));
        image.put_pixel(100, 50, Rgb([255, 0, 0]));
        let covered = fit_cover(&image, (100, 100));
        assert_eq!(covered.dimensions(), (100, 100));
        assert_eq!(covered.get_pixel(50, 50).0, [255, 0, 0]);
        assert_eq!(fit_cover(&image, (800, 1100)).dimensions(), (800, 1100));
    }

    #[test]
    fn the_edge_colour_is_the_border_median() {
        let mut image = RgbImage::from_pixel(10, 10, Rgb([240, 240, 240]));
        image.put_pixel(5, 5, Rgb([0, 0, 0]));
        image.put_pixel(0, 0, Rgb([0, 0, 0]));
        assert_eq!(edge_colour(&image).0, [240, 240, 240]);
    }

    #[test]
    fn the_local_crop_has_a_margin_and_a_minimum_size() {
        let mut mask = GrayImage::new(800, 1100);
        for y in 500..520 {
            for x in 10..30 {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
        let crop = local_edit_crop(&mask).unwrap();
        assert_eq!((crop.left, crop.width(), crop.height()), (0, 384, 384));
        assert!(crop.top <= 484 && crop.bottom >= 536);
        assert!(local_edit_crop(&GrayImage::new(10, 10)).is_err());
    }

    #[test]
    fn the_local_edit_keeps_everything_outside_the_mask() {
        let base = RgbImage::from_pixel(64, 64, Rgb([10, 10, 10]));
        let edited = RgbImage::from_pixel(64, 64, Rgb([250, 250, 250]));
        let mut mask = GrayImage::new(64, 64);
        for y in 16..48 {
            for x in 16..48 {
                mask.put_pixel(x, y, Luma([255]));
            }
        }
        let result = composite_local_edit(&base, &mask, &edited, 3);
        assert_eq!(result.get_pixel(8, 8).0, [10, 10, 10]);
        assert_eq!(result.get_pixel(15, 32).0, [10, 10, 10]);
        assert_eq!(result.get_pixel(32, 32).0, [250, 250, 250]);
        // The edge fades in.
        let edge = result.get_pixel(16, 32).0[0];
        assert!(edge > 10 && edge < 250, "{edge}");
    }

    #[test]
    fn error_bodies_are_read_like_the_old_backend() {
        assert_eq!(
            error_message(r#"{"error": {"message": "bad prompt", "code": 400}}"#),
            "bad prompt"
        );
        assert_eq!(error_message(r#"{"error": {"code": "x"}}"#), "x");
        assert_eq!(error_message(r#"{"message": "busy"}"#), "busy");
        assert_eq!(error_message("plain text"), "plain text");
        assert_eq!(error_message(&"x".repeat(400)).len(), 300);
    }

    #[test]
    fn folder_names_are_made_safe() {
        assert_eq!(safe_segment("ab-12_c.d"), "ab-12_c.d");
        assert_eq!(safe_segment("../a b//c"), "a-b-c");
        assert_eq!(safe_segment("..."), "unknown");
    }
}

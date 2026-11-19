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

use super::hosted::ratio_value;
use super::prompt::{self, Role};
use super::usage::TokenUsage;
use super::{
    EYE_BAND, EYE_DETAIL_SIZE, EyeDetail, FRONT_SIZE, GeneratedImage, GenerationRequest, LocalEdit,
    Mode, TURNAROUND_SIZE, clean_root, generated_root, generated_url, watermark,
};
use crate::backend::chat_api::{self, JsonObject};
use crate::backend::failure::Failure;
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

/// One request to a hosted image API.
pub struct ApiRequest {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub payload: JsonObject,
}

/// The image bytes of a successful answer, with what the request used.
pub struct ApiOutput {
    pub bytes: Vec<u8>,
    pub usage: Option<TokenUsage>,
    /// The price in US dollars, when the API says (OpenRouter's `usage.cost`).
    pub cost: Option<f64>,
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
    /// Kept at the model's resolution, padded to the cut-out's shape.
    EyeDetail {
        ratio: f64,
    },
    Local {
        base: RgbImage,
        mask: GrayImage,
        crop: Crop,
        feather: u32,
    },
}

/// A rectangle on the base image, in pixels. A local revision's may reach
/// past the image's edges, where the cut-out is padded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Crop {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Crop {
    fn width(self) -> u32 {
        u32::try_from(self.right - self.left).unwrap_or(0)
    }

    fn height(self) -> u32 {
        u32::try_from(self.bottom - self.top).unwrap_or(0)
    }

    fn contains(self, other: Self) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }
}

/// The references, images and prompt for a job. `max_images` is how many
/// images the API takes; `max_prompt_chars` its prompt limit, if any;
/// `aspect_ratios` the width:height ratios it asks for the answer's shape
/// from, if it asks for one.
pub fn prepare(
    data_dir: &Path,
    request: &GenerationRequest,
    max_images: usize,
    max_prompt_chars: Option<usize>,
    aspect_ratios: Option<&[String]>,
) -> Result<Prepared, String> {
    if let Some(edit) = &request.local_edit {
        return prepare_local_revision(data_dir, request, edit, aspect_ratios);
    }
    if let Some(eyes) = &request.eye_detail {
        return prepare_eye_detail(data_dir, request, eyes, aspect_ratios);
    }
    if request.generation_mode == Mode::EyeDetail {
        return Err("An eye close-up needs the front view to take the eyes from".to_owned());
    }
    prepare_generation(data_dir, request, max_images.max(1), max_prompt_chars)
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
                Role::for_tag(&reference.kind)
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
    aspect_ratios: Option<&[String]>,
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
    let crop = aspect_ratios.map_or(crop, |ratios| {
        fit_crop_to_ratio(crop, base.dimensions(), ratios)
    });
    let supplemental: Vec<RgbImage> = user_references(data_dir, &request.reference_keys)
        .iter()
        .map(|reference| load_rgb(&reference.path))
        .collect::<Result<_, _>>()?;
    let mut images = vec![cut_out(&base, crop)];
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

/// The long side the eye cut-out is enlarged to before it is sent, so the
/// model gets a canvas of its own size rather than a thumbnail.
const EYE_CUT_OUT_LONG_SIDE: u32 = 1536;

fn prepare_eye_detail(
    data_dir: &Path,
    request: &GenerationRequest,
    eyes: &EyeDetail,
    aspect_ratios: Option<&[String]>,
) -> Result<Prepared, String> {
    if !eyes.base_image_path.is_file() {
        return Err("The front view for the eye close-up is missing".to_owned());
    }
    let base = load_rgb(&eyes.base_image_path)?;
    let crop = eye_region(eyes.region, base.dimensions())?;
    let crop = aspect_ratios.map_or(crop, |ratios| {
        fit_crop_to_ratio(crop, base.dimensions(), ratios)
    });
    let cut = cut_out(&base, crop);
    let scale = f64::from(EYE_CUT_OUT_LONG_SIDE) / f64::from(cut.width().max(cut.height()));
    let cut = if scale > 1.0 {
        imageops::resize(
            &cut,
            round_u32(f64::from(cut.width()) * scale),
            round_u32(f64::from(cut.height()) * scale),
            FilterType::Lanczos3,
        )
    } else {
        cut
    };
    let ratio = f64::from(crop.width()) / f64::from(crop.height());
    let supplemental: Vec<RgbImage> = user_references(data_dir, &request.reference_keys)
        .iter()
        .map(|reference| load_rgb(&reference.path))
        .collect::<Result<_, _>>()?;
    let mut images = vec![cut];
    match supplemental.len() {
        0 => {}
        1 => images.extend(supplemental),
        _ => images.push(contact_sheet(&supplemental)),
    }
    Ok(Prepared {
        prompt: prompt::build_eye_detail_prompt(images.len() > 1),
        images,
        finish: Finish::EyeDetail { ratio },
    })
}

/// The eye region on a base of `(width, height)`: the given rectangle, or
/// [`EYE_BAND`].
fn eye_region(region: Option<[u32; 4]>, (width, height): (u32, u32)) -> Result<Crop, String> {
    let [left, top, right, bottom] = region.unwrap_or_else(|| {
        let [left, top, right, bottom] = EYE_BAND;
        [
            round_u32(f64::from(width) * left),
            round_u32(f64::from(height) * top),
            round_u32(f64::from(width) * right),
            round_u32(f64::from(height) * bottom),
        ]
    });
    let (right, bottom) = (right.min(width), bottom.min(height));
    if left >= right || top >= bottom {
        return Err("The eye region is empty".to_owned());
    }
    Ok(Crop {
        left: i64::from(left),
        top: i64::from(top),
        right: i64::from(right),
        bottom: i64::from(bottom),
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
            Finish::EyeDetail { ratio } => {
                // Like the turnaround: never upscaled, padded rather than
                // cropped, so no eyelash is cut off.
                let padded = pad_to_ratio(raw, *ratio, edge_colour(raw));
                let (max_width, max_height) = EYE_DETAIL_SIZE;
                let scale = (f64::from(max_width) / f64::from(padded.width()))
                    .min(f64::from(max_height) / f64::from(padded.height()));
                Ok(if scale < 1.0 {
                    imageops::resize(
                        &padded,
                        round_u32(f64::from(padded.width()) * scale),
                        round_u32(f64::from(padded.height()) * scale),
                        FilterType::Lanczos3,
                    )
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
                // When the crop has the ratio the API asked for, this only
                // scales the answer.
                let edited = fit_cover(raw, (crop.width(), crop.height()));
                let mut pasted = base.clone();
                imageops::replace(&mut pasted, &edited, crop.left, crop.top);
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

fn grow_span(mut start: i64, mut end: i64, limit: u32) -> (i64, i64) {
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
    (start.clamp(0, limit), end.clamp(0, limit))
}

/// `crop` widened to the one of `ratios` the API will ask for, so that its
/// answer scales back onto the crop. Cutting an answer of another ratio
/// down to the crop shifts what the model drew by whatever it added on one
/// side only (GPT Image 2.6 kept a 624x384 crop at the top of its 3:2
/// answer and drew more below, so it came back 16 px too high).
///
/// The crop only grows, around its centre, and moves to stay on the image.
/// The ratio is the nearest (by the area added) of those that fit on the
/// image; when none does, the nearest of all, and the crop then reaches
/// past the image's edges on the side that is too short, centred on it.
/// Without a usable ratio the crop is kept.
fn fit_crop_to_ratio(crop: Crop, (width, height): (u32, u32), ratios: &[String]) -> Crop {
    let (crop_width, crop_height) = (i64::from(crop.width()), i64::from(crop.height()));
    if crop_width == 0 || crop_height == 0 {
        return crop;
    }
    let current = f64::from(crop.width()) / f64::from(crop.height());
    let fits = |(grown_width, grown_height): (i64, i64)| {
        grown_width <= i64::from(width) && grown_height <= i64::from(height)
    };
    // The crop grown to each ratio, and how far that ratio is from its own.
    let best = ratios
        .iter()
        .filter_map(|ratio| ratio_value(ratio))
        .map(|ratio| {
            let grown = if ratio >= current {
                let grown_width = round_u32(f64::from(crop.height()) * ratio);
                (crop_width.max(i64::from(grown_width)), crop_height)
            } else {
                let grown_height = round_u32(f64::from(crop.width()) / ratio);
                (crop_width, crop_height.max(i64::from(grown_height)))
            };
            (grown, (ratio / current).ln().abs())
        })
        .min_by(|(a, a_distance), (b, b_distance)| {
            fits(*b)
                .cmp(&fits(*a))
                .then(a_distance.total_cmp(b_distance))
        });
    let Some(((grown_width, grown_height), _)) = best else {
        return crop;
    };
    let (left, right) = place_span(crop.left, crop.right, grown_width, width);
    let (top, bottom) = place_span(crop.top, crop.bottom, grown_height, height);
    Crop {
        left,
        top,
        right,
        bottom,
    }
}

/// `start..end` grown to `length` around its centre and moved onto
/// `0..limit`, or, when longer than `limit`, centred on it.
fn place_span(start: i64, end: i64, length: i64, limit: u32) -> (i64, i64) {
    let limit = i64::from(limit);
    let start = if length <= limit {
        (start - (length - (end - start)) / 2).clamp(0, limit - length)
    } else {
        -(length - limit) / 2
    };
    (start, start + length)
}

/// The base under `crop`; where the crop reaches past the base, its border
/// colour.
fn cut_out(base: &RgbImage, crop: Crop) -> RgbImage {
    let (width, height) = base.dimensions();
    let on_image = Crop {
        left: 0,
        top: 0,
        right: i64::from(width),
        bottom: i64::from(height),
    }
    .contains(crop);
    let fill = if on_image {
        Rgb([0, 0, 0])
    } else {
        edge_colour(base)
    };
    let mut canvas = RgbImage::from_pixel(crop.width(), crop.height(), fill);
    imageops::replace(&mut canvas, base, -crop.left, -crop.top);
    canvas
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
/// the image bytes and the reported usage.
pub async fn request_image(
    client: &reqwest::Client,
    api: &dyn ImageApi,
    request: &ApiRequest,
    trace_id: &str,
    retry_delays: &[Duration],
) -> Result<ApiOutput, Failure> {
    let name = api.display_name();
    let max_retries = api.max_retries();
    let seconds = api.timeout().as_secs();
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
        // What went wrong, the HTTP status if one came, the API's own
        // words, and whether trying again may help.
        let (failure, status, detail, retryable) = match builder.send().await {
            Err(error) if error.is_timeout() => {
                // The server accepted the job but never answered; retrying
                // would multiply the wait without changing the outcome.
                return Err(Failure::api_timeout(
                    name,
                    seconds,
                    format!(
                        "{name} did not respond within {seconds}s (model {}, trace_id={trace_id}); the model may be overloaded",
                        api.model()
                    ),
                ));
            }
            Err(error) => (
                format!("request failed: {error}"),
                None,
                error.to_string(),
                true,
            ),
            Ok(response) => {
                let status = response.status().as_u16();
                let server_trace = api
                    .trace_id(response.headers())
                    .unwrap_or_else(|| trace_id.to_owned());
                let text = match response.text().await {
                    Ok(text) => text,
                    Err(error) if error.is_timeout() => {
                        return Err(Failure::api_timeout(
                            name,
                            seconds,
                            format!(
                                "{name} did not finish its answer within {seconds}s (trace_id={server_trace})"
                            ),
                        ));
                    }
                    Err(error) => format!("unreadable answer: {error}"),
                };
                if status == 200 {
                    let body = serde_json::from_str::<Value>(&text).unwrap_or_default();
                    let no_image = || {
                        Failure::coded(
                            "api_no_image",
                            format!("{name} response did not include an image"),
                        )
                        .with("service", name)
                    };
                    let image = api.extract_image(&body).ok_or_else(no_image)?;
                    let reported = body.get("usage");
                    let usage = reported.and_then(TokenUsage::parse);
                    let cost = reported
                        .and_then(|usage| usage.get("cost"))
                        .and_then(Value::as_f64);
                    log::info!(
                        "{name} image generated trace_id={server_trace} duration={:.1}s attempt={} usage={} cost={cost:?}",
                        started.elapsed().as_secs_f64(),
                        attempt + 1,
                        usage.map_or_else(
                            || "none".to_owned(),
                            |usage| usage.to_json(None).to_string()
                        ),
                    );
                    let bytes = match image {
                        ApiImage::Base64(data) => base64::engine::general_purpose::STANDARD
                            .decode(data.trim())
                            .map_err(|_| no_image())?,
                        ApiImage::Url(url) => download(client, &url, name, api.timeout()).await?,
                    };
                    return Ok(ApiOutput { bytes, usage, cost });
                }
                let detail = chat_api::error_message(&text);
                (
                    format!("HTTP {status} trace_id={server_trace}: {detail}"),
                    Some(status),
                    detail,
                    api.retryable(status),
                )
            }
        };
        if !retryable || attempt >= max_retries {
            return Err(Failure::api(
                name,
                status,
                format!("{name} image generation failed: {failure}"),
                &detail,
            ));
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

/// A short description of the job's request, for the job record, with the
/// size of each prepared image (a local revision's crop is the first).
pub fn request_summary(api: &ApiRequest, images: &[RgbImage]) -> Value {
    let mut payload = api.payload.clone();
    for key in ["input_references", "image", "image2", "image3"] {
        if let Some(value) = payload.get_mut(key) {
            let count = value.as_array().map_or(1, Vec::len);
            *value = json!(format!("[{count} image(s)]"));
        }
    }
    payload.remove("prompt");
    let sizes: Vec<String> = images
        .iter()
        .map(|image| format!("{}x{}", image.width(), image.height()))
        .collect();
    json!({"url": api.url, "payload": payload, "image_sizes": sizes})
}

#[cfg(test)]
mod tests {
    use super::super::hosted::{ASPECT_RATIOS, nearest_aspect_ratio};
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

    fn ratios(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn crop(left: i64, top: i64, right: i64, bottom: i64) -> Crop {
        Crop {
            left,
            top,
            right,
            bottom,
        }
    }

    /// An 800x1100 front view's mask painted over `left..right`,
    /// `top..bottom`.
    fn box_mask(left: u32, top: u32, right: u32, bottom: u32) -> GrayImage {
        GrayImage::from_fn(800, 1100, |x, y| {
            Luma([
                if (left..right).contains(&x) && (top..bottom).contains(&y) {
                    255
                } else {
                    0
                },
            ])
        })
    }

    /// The ratios OpenRouter lists for GPT Image 2.6 and Nano Banana 2.1.
    const GPT_RATIOS: [&str; 15] = [
        "1:1", "3:2", "2:3", "4:3", "3:4", "16:9", "9:16", "21:9", "auto", "4:5", "5:4", "2:1",
        "1:2", "3:1", "1:3",
    ];
    const NANO_BANANA_RATIOS: [&str; 14] = [
        "1:1", "1:4", "1:8", "2:3", "3:2", "3:4", "4:1", "4:3", "4:5", "5:4", "8:1", "9:16",
        "16:9", "21:9",
    ];

    fn on_front_view(crop: Crop) -> bool {
        self::crop(0, 0, 800, 1100).contains(crop)
    }

    #[test]
    fn the_local_crop_is_widened_to_the_ratio_the_api_asks_for() {
        // Both eyes: the box with its margins is 624x384 (1.625), which went
        // out as 3:2 and came back 1536x1024, cut by 16 px at the top and
        // bottom to fit.
        let natural = local_edit_crop(&box_mask(168, 474, 648, 638)).unwrap();
        assert_eq!(natural, crop(96, 364, 720, 748));
        for list in [ratios(&ASPECT_RATIOS), ratios(&GPT_RATIOS)] {
            let fitted = fit_crop_to_ratio(natural, (800, 1100), &list);
            assert_eq!(fitted, crop(96, 348, 720, 764));
            assert_eq!(
                nearest_aspect_ratio(fitted.width(), fitted.height(), &list).as_deref(),
                Some("3:2")
            );
        }
        // Nano Banana's 5:4 for the cat-eared character's eyes: 456x384
        // grows sideways.
        let natural = local_edit_crop(&box_mask(230, 365, 580, 501)).unwrap();
        assert_eq!(natural, crop(177, 241, 633, 625));
        assert_eq!(
            fit_crop_to_ratio(natural, (800, 1100), &ratios(&NANO_BANANA_RATIOS)),
            crop(165, 241, 645, 625)
        );
    }

    #[test]
    fn the_widened_crop_stays_on_the_image_at_its_edges() {
        let all = ratios(&ASPECT_RATIOS);
        // At the bottom edge the extra height goes above.
        let natural = local_edit_crop(&box_mask(168, 1000, 648, 1100)).unwrap();
        assert_eq!(natural, crop(96, 716, 720, 1100));
        assert_eq!(
            fit_crop_to_ratio(natural, (800, 1100), &all),
            crop(96, 684, 720, 1100)
        );
        // In the top left corner it stays at the corner.
        let natural = local_edit_crop(&box_mask(0, 0, 300, 60)).unwrap();
        assert_eq!(natural, crop(0, 0, 390, 384));
        assert_eq!(
            fit_crop_to_ratio(natural, (800, 1100), &all),
            crop(0, 0, 390, 390)
        );
    }

    #[test]
    fn a_crop_no_ratio_fits_reaches_past_the_image() {
        // The whole front view (8:11) fits none of the ratios: it takes the
        // nearest, 3:4, 25 px wider than the image, half on each side.
        let natural = local_edit_crop(&box_mask(0, 0, 800, 1100)).unwrap();
        assert_eq!(natural, crop(0, 0, 800, 1100));
        let fitted = fit_crop_to_ratio(natural, (800, 1100), &ratios(&ASPECT_RATIOS));
        assert_eq!(fitted, crop(-12, 0, 813, 1100));
        // A model with only 1:1 and a stroke down the whole height.
        let natural = local_edit_crop(&box_mask(300, 0, 500, 1100)).unwrap();
        assert_eq!(natural, crop(208, 0, 592, 1100));
        assert_eq!(
            fit_crop_to_ratio(natural, (800, 1100), &ratios(&["1:1"])),
            crop(-150, 0, 950, 1100)
        );
    }

    #[test]
    fn every_widened_crop_asks_for_its_own_ratio() {
        for list in [
            ratios(&ASPECT_RATIOS),
            ratios(&GPT_RATIOS),
            ratios(&NANO_BANANA_RATIOS),
            ratios(&["1:1", "2:3", "16:9"]),
        ] {
            for width in (384..=800).step_by(8) {
                for height in (384..=1100).step_by(8) {
                    let (left, top) = (i64::from(800 - width) / 3, i64::from(1100 - height) / 2);
                    let natural = crop(left, top, left + i64::from(width), top + i64::from(height));
                    let fitted = fit_crop_to_ratio(natural, (800, 1100), &list);
                    assert!(fitted.contains(natural), "{natural:?} -> {fitted:?}");
                    let asked = nearest_aspect_ratio(fitted.width(), fitted.height(), &list)
                        .and_then(|ratio| ratio_value(&ratio))
                        .unwrap();
                    let ratio = f64::from(fitted.width()) / f64::from(fitted.height());
                    assert!(
                        (ratio / asked - 1.0).abs() < 0.005,
                        "{natural:?} -> {fitted:?} asks for {asked}"
                    );
                    // Up to 1000 px high, 3:4 always fits.
                    if height <= 1000 && list.iter().any(|ratio| ratio == "3:4") {
                        assert!(on_front_view(fitted), "{natural:?} -> {fitted:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn without_a_usable_ratio_the_crop_is_kept() {
        let natural = crop(96, 364, 720, 748);
        assert_eq!(
            fit_crop_to_ratio(natural, (800, 1100), &ratios(&["auto"])),
            natural
        );
        assert_eq!(fit_crop_to_ratio(natural, (800, 1100), &[]), natural);
    }

    /// The middle of the dark pixels inside `area`.
    fn dark_centre(image: &RgbImage, area: Crop) -> (f64, f64) {
        let (mut sum_x, mut sum_y, mut count) = (0.0, 0.0, 0.0);
        for (x, y, pixel) in image.enumerate_pixels() {
            let inside = (area.left..area.right).contains(&i64::from(x))
                && (area.top..area.bottom).contains(&i64::from(y));
            if inside && pixel.0[0] < 128 {
                sum_x += f64::from(x);
                sum_y += f64::from(y);
                count += 1.0;
            }
        }
        (sum_x / count, sum_y / count)
    }

    #[test]
    fn an_answer_at_the_asked_ratio_lands_where_it_was_cut_out() {
        // A dark square between the eyes; the model gives back the cut-out
        // as it is, at GPT's 3:2 size.
        let mut base = RgbImage::from_pixel(800, 1100, Rgb([220, 220, 220]));
        for y in 540..580 {
            for x in 380..420 {
                base.put_pixel(x, y, Rgb([20, 20, 20]));
            }
        }
        let mask = box_mask(168, 474, 648, 638);
        let fitted = fit_crop_to_ratio(
            local_edit_crop(&mask).unwrap(),
            base.dimensions(),
            &ratios(&GPT_RATIOS),
        );
        let sent = cut_out(&base, fitted);
        assert_eq!(sent.dimensions(), (624, 416));
        let answer = imageops::resize(&sent, 1536, 1024, FilterType::Lanczos3);
        let prepared = Prepared {
            prompt: String::new(),
            images: vec![sent],
            finish: Finish::Local {
                base: base.clone(),
                mask: mask.clone(),
                crop: fitted,
                feather: 16,
            },
        };
        let result = prepared.finish(&answer).unwrap();
        let area = crop(168, 474, 648, 638);
        let (before, after) = (dark_centre(&base, area), dark_centre(&result, area));
        assert!(
            (before.0 - after.0).abs() < 0.5 && (before.1 - after.1).abs() < 0.5,
            "{before:?} -> {after:?}"
        );
        for (x, y, pixel) in result.enumerate_pixels() {
            if mask.get_pixel(x, y).0[0] == 0 {
                assert_eq!(pixel, base.get_pixel(x, y), "({x}, {y})");
            }
        }
    }

    #[test]
    fn a_crop_past_the_image_is_padded_and_pasted_back() {
        let base = RgbImage::from_fn(800, 1100, |x, _| {
            if x < 400 {
                Rgb([200, 200, 200])
            } else {
                Rgb([100, 100, 100])
            }
        });
        let mask = box_mask(300, 0, 500, 1100);
        let fitted = fit_crop_to_ratio(
            local_edit_crop(&mask).unwrap(),
            base.dimensions(),
            &ratios(&["1:1"]),
        );
        let sent = cut_out(&base, fitted);
        assert_eq!(sent.dimensions(), (1100, 1100));
        // The border's median colour on the left and right, the image
        // between.
        let border = edge_colour(&base);
        assert_eq!(*sent.get_pixel(149, 500), border);
        assert_eq!(sent.get_pixel(150, 500).0, [200, 200, 200]);
        assert_eq!(sent.get_pixel(949, 500).0, [100, 100, 100]);
        assert_eq!(*sent.get_pixel(950, 500), border);
        let answer = RgbImage::from_pixel(1024, 1024, Rgb([250, 0, 0]));
        let prepared = Prepared {
            prompt: String::new(),
            images: vec![sent],
            finish: Finish::Local {
                base: base.clone(),
                mask,
                crop: fitted,
                feather: 0,
            },
        };
        let result = prepared.finish(&answer).unwrap();
        assert_eq!(result.dimensions(), (800, 1100));
        assert_eq!(result.get_pixel(299, 500).0, [200, 200, 200]);
        assert_eq!(result.get_pixel(300, 500).0, [250, 0, 0]);
        assert_eq!(result.get_pixel(499, 0).0, [250, 0, 0]);
        assert_eq!(result.get_pixel(500, 1099).0, [100, 100, 100]);
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
        use crate::backend::chat_api::error_message;
        assert_eq!(
            error_message(r#"{"error": {"message": "bad prompt", "code": 400}}"#),
            "bad prompt"
        );
        assert_eq!(error_message(r#"{"error": {"code": "x"}}"#), "x");
        assert_eq!(error_message(r#"{"message": "busy"}"#), "busy");
        assert_eq!(error_message("plain text"), "plain text");
        assert_eq!(error_message(&"x".repeat(600)).len(), 500);
    }

    #[test]
    fn folder_names_are_made_safe() {
        assert_eq!(safe_segment("ab-12_c.d"), "ab-12_c.d");
        assert_eq!(safe_segment("../a b//c"), "a-b-c");
        assert_eq!(safe_segment("..."), "unknown");
    }

    /// A front view with a dark square on each eye, and an eye request for it.
    fn eye_front(dir: &Path, region: Option<[u32; 4]>) -> GenerationRequest {
        let mut base = RgbImage::from_pixel(800, 1100, Rgb([230, 220, 210]));
        for (left, right) in [(260, 340), (460, 540)] {
            for y in 500..560 {
                for x in left..right {
                    base.put_pixel(x, y, Rgb([30, 120, 60]));
                }
            }
        }
        let front = dir.join("front.png");
        base.save(&front).unwrap();
        let mut request = GenerationRequest::new("s", "ja", Mode::EyeDetail);
        request.eye_detail = Some(EyeDetail {
            base_image_path: front,
            region,
        });
        request
    }

    #[test]
    fn the_eye_close_up_sends_the_eye_band_enlarged() {
        let dir = tempfile::tempdir().unwrap();
        let request = eye_front(dir.path(), None);
        let ratios = ratios(&GPT_RATIOS);
        let prepared = prepare(dir.path(), &request, 4, None, Some(&ratios)).unwrap();
        // The band (120, 396)-(680, 660) is 560x264, widened to 2:1 and
        // enlarged to 1536 wide; both eyes are inside it.
        assert_eq!(prepared.images.len(), 1);
        assert_eq!(prepared.images[0].dimensions(), (1536, 768));
        let ratio = f64::from(1536) / f64::from(768);
        assert_eq!(
            nearest_aspect_ratio(1536, 768, &ratios).as_deref(),
            Some("2:1")
        );
        let green = |image: &RgbImage, x: u32, y: u32| image.get_pixel(x, y).0[1] > 100;
        let sent = &prepared.images[0];
        // (300, 530) and (500, 530) on the front view: 120 and 388 off.
        let scale = 1536.0 / 560.0;
        for x in [300.0, 500.0] {
            let (sent_x, sent_y) = ((x - 120.0) * scale, (530.0 - 388.0) * scale);
            assert!(green(sent, round_u32(sent_x), round_u32(sent_y)), "{x}");
        }
        assert!(!prepared.prompt.contains("Image 2"));
        assert!(prepared.prompt.contains("Keep the eye design exactly"));
        assert!(
            matches!(prepared.finish, Finish::EyeDetail { ratio: r } if (r - ratio).abs() < 0.01)
        );
    }

    #[test]
    fn the_eye_close_up_takes_a_given_region_and_the_art_as_image_2() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = eye_front(dir.path(), Some([250, 480, 550, 580]));
        let art = references::root(dir.path()).join("p").join("eyes.png");
        std::fs::create_dir_all(art.parent().unwrap()).unwrap();
        RgbImage::from_pixel(300, 100, Rgb([0, 200, 0]))
            .save(&art)
            .unwrap();
        request.reference_keys = vec!["supplemental:references/p/eyes.png".to_owned()];
        // Without ratios the region is sent as it is (3:1), enlarged.
        let prepared = prepare(dir.path(), &request, 4, None, None).unwrap();
        assert_eq!(prepared.images.len(), 2);
        assert_eq!(prepared.images[0].dimensions(), (1536, 512));
        assert_eq!(prepared.images[1].dimensions(), (300, 100));
        assert!(
            prepared
                .prompt
                .contains("Image 2 shows the character's eyes")
        );
        assert!(prepared.prompt.contains("follow Image 1"));
    }

    #[test]
    fn an_eye_close_up_needs_its_front_view_and_a_region() {
        let dir = tempfile::tempdir().unwrap();
        let request = GenerationRequest::new("s", "ja", Mode::EyeDetail);
        assert!(prepare(dir.path(), &request, 4, None, None).is_err());
        let empty = eye_front(dir.path(), Some([300, 500, 300, 600]));
        assert!(prepare(dir.path(), &empty, 4, None, None).is_err());
        let mut missing = eye_front(dir.path(), None);
        if let Some(eyes) = missing.eye_detail.as_mut() {
            eyes.base_image_path = dir.path().join("gone.png");
        }
        assert!(prepare(dir.path(), &missing, 4, None, None).is_err());
    }

    #[test]
    fn the_eye_close_up_keeps_the_model_resolution_up_to_its_cap() {
        let finish = |ratio: f64, raw: &RgbImage| {
            Prepared {
                prompt: String::new(),
                images: Vec::new(),
                finish: Finish::EyeDetail { ratio },
            }
            .finish(raw)
            .unwrap()
            .dimensions()
        };
        // GPT's 2:1 answer at 2K stays as it is; a 3:2 one is padded wider.
        let wide = RgbImage::from_pixel(2048, 1024, Rgb([1, 2, 3]));
        assert_eq!(finish(2.0, &wide), (2048, 1024));
        let small = RgbImage::from_pixel(1536, 1024, Rgb([1, 2, 3]));
        assert_eq!(finish(2.0, &small), (2048, 1024));
        // Never upscaled; a larger one comes down to 2400 wide.
        let tiny = RgbImage::from_pixel(600, 300, Rgb([1, 2, 3]));
        assert_eq!(finish(2.0, &tiny), (600, 300));
        let large = RgbImage::from_pixel(4096, 2048, Rgb([1, 2, 3]));
        assert_eq!(finish(2.0, &large), (2400, 1200));
    }
}

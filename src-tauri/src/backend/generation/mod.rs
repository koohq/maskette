//! Image generation and reference analysis, as the agent's tools see them.
//!
//! The real backends (OpenRouter, Ark, SiliconFlow, Codex, ...) and the job
//! records move over in the next step of the port. Until then only the
//! fixtures work: placeholder images and a canned analysis, in debug builds.
//! Every other backend answers that it has not moved yet.
//!
//! Generated images live in `generated/` under the data folder (the old
//! `runtime/generated`) and are served at `/api/generated/...`. The
//! unwatermarked copy that later jobs attach as a reference goes to
//! `clean-outputs/` (the old `runtime/clean-outputs`). Watermarks come with
//! the real backends; for now both copies are the same pixels.

pub mod analysis;

use std::path::{Component, Path, PathBuf};

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use image::{Rgb, RgbImage};
use serde::Serialize;
use serde_json::Value;

use super::SharedState;
use super::error::ApiError;
use super::settings::store::Settings;

pub const GENERATED_DIR: &str = "generated";
pub const CLEAN_DIR: &str = "clean-outputs";
pub const GENERATED_PREFIX: &str = "/api/generated/";

pub const FRONT_SIZE: (u32, u32) = (800, 1100);
pub const TURNAROUND_SIZE: (u32, u32) = (3000, 2000);

pub fn routes() -> Router<SharedState> {
    Router::new().route("/api/generated/{*asset_path}", get(generated_asset))
}

pub fn generated_root(data_dir: &Path) -> PathBuf {
    data_dir.join(GENERATED_DIR)
}

pub fn clean_root(data_dir: &Path) -> PathBuf {
    data_dir.join(CLEAN_DIR)
}

/// The URL that serves a path below `generated/` (with `/` separators).
pub fn generated_url(relative: &str) -> String {
    format!("{GENERATED_PREFIX}{relative}")
}

/// The file behind a `/api/generated/...` URL, if the URL is one and stays
/// inside the folder.
pub fn generated_file_from_url(data_dir: &Path, url: &str) -> Option<PathBuf> {
    let relative = url.strip_prefix(GENERATED_PREFIX)?;
    let relative = relative.split('?').next().unwrap_or_default();
    join_plain(&generated_root(data_dir), relative)
}

/// `root` joined with a relative `/`-path, or `None` when a part of it is not
/// a plain name, so the result always stays inside `root`.
pub fn join_plain(root: &Path, relative: &str) -> Option<PathBuf> {
    let parts = plain_parts(relative)?;
    Some(
        parts
            .iter()
            .fold(root.to_owned(), |path, part| path.join(part)),
    )
}

/// The parts of a relative `/`-path, or `None` when one of them is not a
/// plain name (`..`, a drive, a stream, ...).
fn plain_parts(relative: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = relative
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let plain = |part: &&str| {
        !part.contains(':')
            && matches!(
                Path::new(part).components().collect::<Vec<_>>().as_slice(),
                [Component::Normal(_)]
            )
    };
    (!parts.is_empty() && parts.iter().all(plain)).then_some(parts)
}

pub fn media_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// `GET /api/generated/<path>`.
async fn generated_asset(
    State(state): State<SharedState>,
    axum::extract::Path(asset_path): axum::extract::Path<String>,
) -> Result<Response, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "Generated asset not found");
    let path = generated_file_from_url(&state.data_dir, &generated_url(&asset_path))
        .ok_or_else(not_found)?;
    let media_type = media_type(&path).ok_or_else(not_found)?;
    let bytes = tokio::fs::read(&path).await.map_err(|_| not_found())?;
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static(media_type))],
        bytes,
    )
        .into_response())
}

/// The generation modes the tools ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    FrontDesign,
    FrontRevision,
    FrontLocalRevision,
    Turnaround,
    CharacterFront,
    CharacterRevision,
    CharacterTurnaround,
}

impl Mode {
    pub fn output_size(self) -> (u32, u32) {
        match self {
            Self::Turnaround | Self::CharacterTurnaround => TURNAROUND_SIZE,
            _ => FRONT_SIZE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceDescription {
    pub reference_key: String,
    pub description: String,
}

/// The inputs of a local regeneration (only the masked area changes).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalEdit {
    pub base_image_path: PathBuf,
    pub mask_image_path: PathBuf,
    pub edit_note: String,
    pub base_width: u32,
    pub base_height: u32,
    /// Narrow = hard lock outside the mask; wide blends the edit in.
    pub feather_radius_px: u32,
    /// `design` or `head_shell`.
    pub subject: &'static str,
}

/// One image to generate (the old `CreateJobRequest` plus its prompt
/// payload). Reference keys are `<kind>:<object key>`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GenerationRequest {
    pub character_session_id: String,
    pub free_text: String,
    pub locale: String,
    pub reference_keys: Vec<String>,
    pub reference_descriptions: Vec<ReferenceDescription>,
    pub detail_lock: Option<Value>,
    pub generation_mode: Mode,
    pub local_edit: Option<LocalEdit>,
}

impl GenerationRequest {
    pub fn new(character_session_id: &str, locale: &str, mode: Mode) -> Self {
        Self {
            character_session_id: character_session_id.to_owned(),
            free_text: String::new(),
            locale: locale.to_owned(),
            reference_keys: Vec::new(),
            reference_descriptions: Vec::new(),
            detail_lock: None,
            generation_mode: mode,
            local_edit: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub job_id: String,
    /// The public (watermarked) image's URL.
    pub url: String,
    /// The unwatermarked pixels, for attaching to later jobs.
    pub clean_path: PathBuf,
    pub width: u32,
    pub height: u32,
}

/// The configured image backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageBackend {
    Fixture,
}

/// The image backend the settings choose, or why there is none.
pub fn image_backend(settings: &Settings) -> Result<ImageBackend, String> {
    match settings.image_provider() {
        "fixture" if settings.allow_fixture => Ok(ImageBackend::Fixture),
        "fixture" => Err("No real generation backend is configured (image_provider).".to_owned()),
        other => Err(format!(
            "Image generation with {other} has not moved to the new backend yet."
        )),
    }
}

impl ImageBackend {
    pub fn supports_local_revision(self) -> bool {
        match self {
            Self::Fixture => true,
        }
    }

    pub async fn generate(
        self,
        data_dir: &Path,
        request: &GenerationRequest,
    ) -> Result<GeneratedImage, String> {
        match self {
            Self::Fixture => {
                let data_dir = data_dir.to_owned();
                let request = request.clone();
                tokio::task::spawn_blocking(move || fixture_image(&data_dir, &request))
                    .await
                    .map_err(|error| format!("fixture generation stopped: {error}"))?
            }
        }
    }
}

/// A flat placeholder of the mode's size; a local revision tints the masked
/// area of its base instead, so the editor flow can be tried by hand.
fn fixture_image(data_dir: &Path, request: &GenerationRequest) -> Result<GeneratedImage, String> {
    let job_id = super::ids::uuid4();
    let image = match &request.local_edit {
        Some(edit) => tinted_base(edit)?,
        None => {
            let (width, height) = request.generation_mode.output_size();
            let colour = match request.generation_mode {
                Mode::CharacterFront | Mode::CharacterRevision | Mode::CharacterTurnaround => {
                    Rgb([236, 226, 250])
                }
                _ => Rgb([250, 232, 226]),
            };
            RgbImage::from_pixel(width, height, colour)
        }
    };
    let relative = format!("{}/fixture/{job_id}.png", request.character_session_id);
    let public_path = generated_root(data_dir).join(&relative);
    let clean_path = clean_root(data_dir).join(&relative);
    for path in [&public_path, &clean_path] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        image
            .save_with_format(path, image::ImageFormat::Png)
            .map_err(|error| format!("cannot write the fixture image: {error}"))?;
    }
    Ok(GeneratedImage {
        job_id,
        url: generated_url(&relative),
        clean_path,
        width: image.width(),
        height: image.height(),
    })
}

fn tinted_base(edit: &LocalEdit) -> Result<RgbImage, String> {
    let mut base = image::open(&edit.base_image_path)
        .map_err(|error| format!("cannot read the base image: {error}"))?
        .to_rgb8();
    let mask = image::open(&edit.mask_image_path)
        .map_err(|error| format!("cannot read the mask: {error}"))?
        .to_luma8();
    for (x, y, pixel) in base.enumerate_pixels_mut() {
        if mask
            .get_pixel_checked(x, y)
            .is_some_and(|value| value.0[0] > 0)
        {
            let [r, g, b] = pixel.0;
            *pixel = Rgb([r / 2 + 120, g / 2, b / 2 + 120]);
        }
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_urls_map_to_files_inside_the_folder() {
        let data = Path::new("data");
        assert_eq!(
            generated_file_from_url(data, "/api/generated/s/fixture/a.png?x=1"),
            Some(
                data.join("generated")
                    .join("s")
                    .join("fixture")
                    .join("a.png")
            )
        );
        assert_eq!(
            generated_file_from_url(data, "/api/generated/../a.png"),
            None
        );
        assert_eq!(
            generated_file_from_url(data, "/api/generated/C:/a.png"),
            None
        );
        assert_eq!(generated_file_from_url(data, "/api/references/a.png"), None);
        assert_eq!(generated_file_from_url(data, "/api/generated/"), None);
    }
}

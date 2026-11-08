//! Reference images: the pictures a design starts from.
//!
//! Ported from the old backend's `app/references/router.py`. Uploads land in
//! `references/` under the data folder (the old `runtime/references`), each in
//! a folder of its own, and are named by an object key
//! `references/<id>/<file name>`. `GET /api/references/<object key>` serves
//! any image under that folder: this router's uploads, and the files other
//! routers keep there (agent conversations, detail crops, local revisions).
//!
//! While those other routers are still relayed, the old backend writes their
//! images into its own `runtime/`; so with the relay on, an image that is not
//! here is asked for there.

use std::path::{Component, Path, PathBuf};

use axum::extract::multipart::{Field, Multipart, MultipartRejection};
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use super::SharedState;
use super::error::ApiError;

/// The folder under the data folder, and the first part of every object key.
pub const DIR_NAME: &str = "references";

/// Uploads larger than this are refused (413). The old backend had no limit;
/// the UI scales reference images down before sending them.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

const IMAGE_MEDIA_TYPES: [(&str, &str); 4] = [
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".webp", "image/webp"),
];
const ALLOWED_UPLOAD_MIME_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

const INVALID_PATH: &str = "Invalid reference asset path";
const UNSUPPORTED_TYPE: &str = "Unsupported reference image type";

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route(
            "/api/references",
            post(upload_reference).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        // The old route also matched an empty path and refused it.
        .route("/api/references/", get(empty_asset_path))
        .route("/api/references/{*asset_path}", get(get_reference_asset))
}

/// Where reference images live under the data folder.
pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

/// The `/api/...` URL that serves an object key (`references/...`).
#[allow(dead_code)] // For the routers that hand out reference images.
pub fn asset_url(object_key: &str) -> String {
    format!("/api/references/{object_key}")
}

/// The media type for a file name's extension, if it is an image we serve.
fn media_type(file_name: &str) -> Option<&'static str> {
    let extension = suffix(file_name).to_ascii_lowercase();
    IMAGE_MEDIA_TYPES
        .iter()
        .find(|(known, _)| *known == extension)
        .map(|(_, media_type)| *media_type)
}

/// Python's `PurePath.suffix`: the last `.part`, but not a leading dot.
fn suffix(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) if index > 0 && index < name.len() - 1 => &name[index..],
        _ => "",
    }
}

/// Python's `PurePath.stem`.
fn stem(name: &str) -> &str {
    &name[..name.len() - suffix(name).len()]
}

/// A file name that is safe to store: the last path part, with anything but
/// ASCII letters, digits, `_`, `.` and `-` turned into `-`.
pub fn safe_filename(file_name: Option<&str>) -> Result<String, ApiError> {
    let raw = file_name
        .filter(|name| !name.is_empty())
        .unwrap_or("reference")
        .replace('\\', "/");
    let name = raw
        .split('/')
        .rfind(|part| !part.is_empty() && *part != ".")
        .unwrap_or("");
    let mut safe_stem = String::new();
    let mut replacing = false;
    for c in stem(name).chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            safe_stem.push(c);
            replacing = false;
        } else if !replacing {
            safe_stem.push('-');
            replacing = true;
        }
    }
    let safe_stem = safe_stem.trim_matches(['.', '-']);
    let safe_stem = if safe_stem.is_empty() {
        "reference"
    } else {
        safe_stem
    };
    let safe_name = format!("{safe_stem}{}", suffix(name).to_lowercase());
    if safe_name == "." || safe_name == ".." {
        return Err(ApiError::bad_request("Invalid reference filename"));
    }
    Ok(safe_name)
}

/// Accept only PNG, JPEG and WebP, by extension and by the declared type.
pub fn validate_image_upload(file_name: &str, content_type: Option<&str>) -> Result<(), ApiError> {
    if media_type(file_name).is_none() {
        return Err(ApiError::bad_request(UNSUPPORTED_TYPE));
    }
    match content_type {
        Some(declared)
            if !declared.is_empty() && !ALLOWED_UPLOAD_MIME_TYPES.contains(&declared) =>
        {
            Err(ApiError::bad_request(UNSUPPORTED_TYPE))
        }
        _ => Ok(()),
    }
}

fn valid_kind(kind: &str) -> bool {
    (1..=32).contains(&kind.len())
        && kind
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A random UUID (version 4) in the usual 8-4-4-4-12 form, like Python's
/// `str(uuid.uuid4())`.
fn uuid4() -> Result<String, ApiError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| ApiError::internal(format!("no randomness: {error}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

#[derive(Serialize)]
struct ReferenceUpload {
    object_key: String,
    file_name: String,
}

struct UploadedFile {
    file_name: Option<String>,
    content_type: Option<String>,
    bytes: axum::body::Bytes,
}

fn read_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    ApiError::new(error.status(), error.body_text())
}

fn form_error(detail: &str) -> ApiError {
    ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, detail)
}

async fn read_text(field: Field<'_>) -> Result<String, ApiError> {
    if field.file_name().is_some() {
        return Err(form_error("kind: expected a text field"));
    }
    field.text().await.map_err(read_error)
}

async fn read_file(field: Field<'_>) -> Result<UploadedFile, ApiError> {
    let Some(file_name) = field.file_name().map(str::to_owned) else {
        return Err(form_error("file: expected an uploaded file"));
    };
    let content_type = field.content_type().map(str::to_owned);
    let bytes = field.bytes().await.map_err(read_error)?;
    Ok(UploadedFile {
        file_name: Some(file_name),
        content_type,
        bytes,
    })
}

/// `POST /api/references`: a form with `kind` (`[A-Za-z0-9_-]{1,32}`) and
/// `file` (a PNG, JPEG or WebP image).
async fn upload_reference(
    State(state): State<SharedState>,
    form: Result<Multipart, MultipartRejection>,
) -> Result<Json<ReferenceUpload>, ApiError> {
    let mut form = form.map_err(|rejection| form_error(&rejection.body_text()))?;
    let mut kind = None;
    let mut file = None;
    // Like the old form parsing, a repeated field keeps its last value.
    while let Some(field) = form.next_field().await.map_err(read_error)? {
        match field.name() {
            Some("kind") => kind = Some(read_text(field).await?),
            Some("file") => file = Some(read_file(field).await?),
            _ => {}
        }
    }
    // An empty text field counts as missing, as it did before.
    let kind = kind.filter(|kind| !kind.is_empty());
    let (Some(kind), Some(file)) = (kind, file) else {
        return Err(form_error("kind and file are required"));
    };

    if !valid_kind(&kind) {
        return Err(ApiError::bad_request("Invalid reference kind"));
    }
    let file_name = safe_filename(file.file_name.as_deref())?;
    validate_image_upload(&file_name, file.content_type.as_deref())?;

    let upload_id = uuid4()?;
    let upload_dir = root(&state.data_dir).join(&upload_id);
    let save = async {
        tokio::fs::create_dir_all(&upload_dir).await?;
        tokio::fs::write(upload_dir.join(&file_name), &file.bytes).await
    };
    save.await.map_err(|error| {
        log::error!("saving a reference image failed: {error}");
        ApiError::internal(format!("reference_not_saved: {error}"))
    })?;

    Ok(Json(ReferenceUpload {
        object_key: format!("{DIR_NAME}/{upload_id}/{file_name}"),
        file_name,
    }))
}

/// The parts of an object key below `references/`, checked the way the old
/// backend checked them, plus anything Windows would read as a drive or a
/// stream.
fn key_parts(asset_path: &str) -> Result<Vec<&str>, ApiError> {
    let invalid = || ApiError::bad_request(INVALID_PATH);
    if asset_path.starts_with(['/', '\\']) {
        return Err(invalid());
    }
    let parts: Vec<&str> = asset_path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if parts.len() < 2 || parts[0] != DIR_NAME || parts.contains(&"..") {
        return Err(invalid());
    }
    let below = &parts[1..];
    for part in below {
        let plain = matches!(
            Path::new(part).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        );
        if !plain || part.contains(':') {
            return Err(invalid());
        }
    }
    Ok(below.to_vec())
}

enum Asset {
    Found(PathBuf, &'static str),
    Missing,
}

fn find_asset(data_dir: &Path, asset_path: &str) -> Result<Asset, ApiError> {
    let parts = key_parts(asset_path)?;
    let file_name = parts.last().copied().unwrap_or_default();
    let media_type =
        media_type(file_name).ok_or_else(|| ApiError::bad_request(UNSUPPORTED_TYPE))?;

    // Join below the resolved folder, so Windows takes every name literally.
    let Ok(root) = root(data_dir).canonicalize() else {
        return Ok(Asset::Missing);
    };
    let Ok(candidate) = parts
        .iter()
        .fold(root.clone(), |path, part| path.join(part))
        .canonicalize()
    else {
        return Ok(Asset::Missing);
    };
    if !candidate.starts_with(&root) {
        return Err(ApiError::bad_request(INVALID_PATH));
    }
    if !candidate.is_file() {
        return Ok(Asset::Missing);
    }
    Ok(Asset::Found(candidate, media_type))
}

/// `GET /api/references/references/<id>/<file name>`.
async fn get_reference_asset(
    State(state): State<SharedState>,
    axum::extract::Path(asset_path): axum::extract::Path<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let data_dir = state.data_dir.clone();
    let lookup = asset_path.clone();
    let asset = tokio::task::spawn_blocking(move || find_asset(&data_dir, &lookup))
        .await
        .map_err(|error| ApiError::internal(format!("reference lookup failed: {error}")))??;
    match asset {
        Asset::Found(path, media_type) => {
            let bytes = tokio::fs::read(&path).await.map_err(|error| {
                log::warn!("reading a reference image failed: {error}");
                ApiError::new(StatusCode::NOT_FOUND, "Reference asset not found")
            })?;
            Ok((
                [(CONTENT_TYPE, HeaderValue::from_static(media_type))],
                bytes,
            )
                .into_response())
        }
        Asset::Missing => match &state.relay {
            // Images the old backend still writes (its routers are relayed).
            Some(relay) => Ok(relay.forward(request).await),
            None => Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "Reference asset not found",
            )),
        },
    }
}

async fn empty_asset_path() -> ApiError {
    ApiError::bad_request(INVALID_PATH)
}

#[cfg(test)]
mod tests;

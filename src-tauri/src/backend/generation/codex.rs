//! Image generation through the Codex CLI (the old `CodexImageProvider`).
//!
//! The images and prompt are the hosted APIs' ([`super::image_api`]),
//! with Codex's own lines in front ([`cli::codex::image_prompt`]); the
//! images go in as files, in prompt order. The answer is the newest image
//! Codex's image tool saved for the run, finished like an API's answer: a
//! local regeneration edits the crop around the mask and is pasted back
//! here, since Codex's image tool takes no mask.

use std::path::PathBuf;

use serde_json::json;

use super::image_api::{self, safe_segment};
use super::{GeneratedImage, GenerationRequest, blocking, jobs};
use crate::backend::AppState;
use crate::backend::cli::{self, codex::Purpose};
use crate::backend::failure::Failure;
use crate::backend::prompts::Overrides;

/// Codex takes any number of images; more than this go in as one sheet.
const MAX_IMAGES: usize = 16;

pub async fn generate(
    state: &AppState,
    job: &jobs::Job<'_>,
    job_id: &str,
    request: &GenerationRequest,
) -> Result<GeneratedImage, Failure> {
    let codex_home = state.cli.codex_home().ok_or_else(|| {
        Failure::coded(
            "codex_home_unknown",
            "Codex's home folder is unknown, so the image it makes cannot be collected (check CODEX_HOME).",
        )
    })?;
    let data_dir = state.data_dir.clone();
    let workspace = cli::codex_root(&data_dir)
        .join(safe_segment(&request.character_session_id))
        .join(safe_segment(job_id));
    let (prepared, prompt, images) = {
        let (data_dir, request, workspace) = (data_dir.clone(), request.clone(), workspace.clone());
        blocking(move || -> Result<_, String> {
            let prepared = image_api::prepare(&data_dir, &request, MAX_IMAGES, None)?;
            let prompt = cli::codex::image_prompt(
                &Overrides::load(&data_dir),
                &prepared.prompt,
                request.local_edit.is_some(),
            );
            std::fs::create_dir_all(&workspace)
                .map_err(|error| format!("cannot create the Codex working folder: {error}"))?;
            let images = prepared
                .images
                .iter()
                .enumerate()
                .map(|(index, image)| {
                    let path = workspace.join(format!("input-{}.png", index + 1));
                    image_api::save_png(image, &path)?;
                    Ok(path)
                })
                .collect::<Result<Vec<PathBuf>, String>>()?;
            Ok((prepared, prompt, images))
        })
        .await?
    };
    job.record_prompt(
        &prompt,
        json!({"cli": "codex exec", "images": images.len()}),
    );
    job.step("generating", 45, "Generating");
    let run = cli::codex::run(
        &state.cli,
        &workspace,
        "generate",
        &prompt,
        &images,
        Purpose::Image,
    )
    .await?;
    if let Some(usage) = run.usage {
        job.record_usage(usage, None);
    }
    job.step("saving_outputs", 90, "Saving outputs");
    let (job_id, request) = (job_id.to_owned(), request.clone());
    blocking(move || {
        let generated = run
            .thread_id
            .as_deref()
            .map(|thread| cli::codex::tool_images(&codex_home, thread))
            .unwrap_or_default();
        let Some(newest) = generated.last() else {
            let said = cli::shorten(&run.text);
            let mut english =
                "Codex did not make an image with its image generation tool. Try again.".to_owned();
            let mut failure = Failure::coded("codex_no_image", String::new());
            if !said.is_empty() {
                english.push_str(&format!(" Codex said: {said}"));
                failure = failure.with("detail", said);
            }
            failure.message = english;
            return Err(failure);
        };
        let raw = image_api::load_rgb(newest)?;
        let image = prepared.finish(&raw)?;
        Ok(image_api::store_output(
            &data_dir, &request, &job_id, &image,
        )?)
    })
    .await
}

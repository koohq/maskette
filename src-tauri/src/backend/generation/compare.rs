//! Side-by-side runs of real image models, for choosing the default one.
//!
//! Ignored by default: it calls OpenRouter with the key the app keeps in the
//! OS credential store (the image key, else the chat or analysis key), and
//! every run is billed. It goes through the same
//! `ImageBackend::generate` as the app, so the prompts match what users get.
//!
//! ```text
//! MASKETTE_COMPARE_STEP=front
//! MASKETTE_COMPARE_REFERENCE=<character image>
//! MASKETTE_COMPARE_MODELS=vendor/model-a,vendor/model-b
//! MASKETTE_COMPARE_OUT=<folder for the images and results.jsonl>
//! cargo test --lib compare_image_models -- --ignored --nocapture
//! ```
//!
//! `front` makes a front view per model. Look at them, then run `follow`
//! with `MASKETTE_COMPARE_MASKS=vendor/model-a=x0,y0,x1,y1;...` (an ellipse
//! on the 800x1100 front view) to make the turnaround from each front view
//! and a local revision inside that ellipse. `MASKETTE_COMPARE_EDIT_NOTE`
//! replaces the default edit (the star hairpin becomes a heart).
//!
//! `back` checks a user back view tagged "back"
//! (`MASKETTE_COMPARE_BACK=<back view image>`): it makes the head shell
//! turnaround from the front view with the back view added, and a character
//! design turnaround from the character and the back view.
//!
//! Each step rebuilds `index.html` in the output folder, which shows the
//! runs side by side. `MASKETTE_COMPARE_STEP=report` only rebuilds it (no
//! model is called and no key is read).

mod report;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde_json::{Value, json};

use super::{GenerationRequest, LocalEdit, Mode, image_backend};
use crate::backend::settings::secrets::{KeyringSecrets, SERVICE, SecretStore as _};
use crate::backend::tests::TestBackend;

const DEFAULT_EDIT_NOTE: &str = "星のヘアピンを、同じ金色のハート形のヘアピンに変えてください。";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

/// A file-name-safe form of a model id.
fn slug(model: &str) -> String {
    model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn ellipse_mask(path: &Path, (x0, y0, x1, y1): (f64, f64, f64, f64)) {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    let (width, height) = super::FRONT_SIZE;
    let mask = image::GrayImage::from_fn(width, height, |x, y| {
        let dx = (f64::from(x) + 0.5 - cx) / rx;
        let dy = (f64::from(y) + 0.5 - cy) / ry;
        image::Luma([if dx * dx + dy * dy <= 1.0 { 255 } else { 0 }])
    });
    mask.save(path).expect("mask written");
}

fn parse_masks(text: &str) -> BTreeMap<String, (f64, f64, f64, f64)> {
    text.split(';')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (model, numbers) = part.rsplit_once('=').expect("model=x0,y0,x1,y1");
            let values: Vec<f64> = numbers
                .split(',')
                .map(|value| value.trim().parse().expect("a number"))
                .collect();
            let [x0, y0, x1, y1] = values[..] else {
                panic!("four numbers for {model}");
            };
            (model.trim().to_owned(), (x0, y0, x1, y1))
        })
        .collect()
}

struct Run<'a> {
    backend: &'a TestBackend,
    out: PathBuf,
}

impl Run<'_> {
    fn use_model(&self, model: &str) {
        self.backend
            .state
            .settings
            .update(&[("openrouter_image_model".to_owned(), Some(model.to_owned()))])
            .expect("model set");
    }

    /// Copies an image into the data folder's references and returns its key.
    fn reference(&self, kind: &str, source: &Path, name: &str) -> String {
        let folder = self.backend.dir.path().join("references").join("compare");
        std::fs::create_dir_all(&folder).expect("reference folder");
        std::fs::copy(source, folder.join(name)).expect("reference copied");
        format!("{kind}:references/compare/{name}")
    }

    /// Runs one job, keeps its clean output as `<model>/<label>.png` and
    /// appends what happened to `results.jsonl`.
    async fn job(&self, model: &str, label: &str, request: &GenerationRequest) -> Option<PathBuf> {
        self.use_model(model);
        let started = Instant::now();
        let result = async {
            image_backend(&self.backend.state.settings.settings())?
                .generate(
                    &self.backend.state,
                    request,
                    &AtomicBool::new(false),
                    &|_, _| {},
                )
                .await
        }
        .await;
        let seconds = (started.elapsed().as_secs_f64() * 10.0).round() / 10.0;
        let record = self.backend.state.generation.jobs.list().pop();
        let folder = self.out.join(slug(model));
        std::fs::create_dir_all(&folder).expect("output folder");
        let (output, size, error) = match &result {
            Ok(image) => {
                let path = folder.join(format!("{label}.png"));
                std::fs::copy(&image.clean_path, &path).expect("output copied");
                (Some(path), json!([image.width, image.height]), Value::Null)
            }
            Err(failure) => (None, Value::Null, json!(failure.message)),
        };
        if let Some(record) = &record {
            std::fs::write(folder.join(format!("{label}-prompt.md")), &record.prompt)
                .expect("prompt written");
        }
        let line = json!({
            "model": model,
            "mode": label,
            "ok": result.is_ok(),
            "seconds": seconds,
            "size": size,
            "request": record.as_ref().map(|record| record.request.clone()),
            "cost_usd": record
                .as_ref()
                .and_then(|record| record.token_usage.as_ref())
                .and_then(|usage| usage.get("cost").cloned()),
            "error": error,
        });
        println!("{line}");
        let mut results = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.out.join("results.jsonl"))
            .expect("results file");
        writeln!(results, "{line}").expect("result written");
        output
    }
}

#[tokio::test]
#[ignore = "calls OpenRouter with the stored key; every run is billed"]
async fn compare_image_models() {
    let step = env("MASKETTE_COMPARE_STEP");
    let out = PathBuf::from(env("MASKETTE_COMPARE_OUT"));
    std::fs::create_dir_all(&out).expect("output folder");
    if step == "report" {
        println!("report: {}", report::write(&out).display());
        return;
    }
    let reference = PathBuf::from(env("MASKETTE_COMPARE_REFERENCE"));
    let models: Vec<String> = env("MASKETTE_COMPARE_MODELS")
        .split(',')
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect();
    // The report shows the character next to the results.
    let extension = reference
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("png");
    std::fs::copy(&reference, out.join(format!("reference.{extension}")))
        .expect("reference copied");

    // The app falls back to the chat and analysis keys when they point at
    // OpenRouter; the keys are assumed to be OpenRouter's here.
    let secrets = KeyringSecrets::new(SERVICE);
    let key = [
        "openrouter_api_key",
        "agent_llm_api_key",
        "analysis_llm_api_key",
    ]
    .into_iter()
    .find_map(|name| secrets.get(name).expect("credential store readable"))
    .expect("an OpenRouter key saved in Maskette's settings");
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            ("image_provider".to_owned(), Some("openrouter".to_owned())),
            ("openrouter_api_key".to_owned(), Some(key)),
        ])
        .expect("settings set");
    let run = Run {
        backend: &backend,
        out: out.clone(),
    };
    let reference_name = format!("character.{extension}");

    match step.as_str() {
        "front" => {
            for model in &models {
                let mut request = GenerationRequest::new("compare", "ja", Mode::FrontDesign);
                request.reference_keys = vec![run.reference("front", &reference, &reference_name)];
                run.job(model, "front", &request).await;
            }
        }
        "follow" => {
            let masks = parse_masks(&env("MASKETTE_COMPARE_MASKS"));
            let note = std::env::var("MASKETTE_COMPARE_EDIT_NOTE")
                .unwrap_or_else(|_| DEFAULT_EDIT_NOTE.to_owned());
            for model in &models {
                let front = out.join(slug(model)).join("front.png");
                assert!(
                    front.is_file(),
                    "no front view for {model}; run the front step first"
                );
                let character = run.reference("supplemental", &reference, &reference_name);

                let mut request = GenerationRequest::new("compare", "ja", Mode::Turnaround);
                request.reference_keys = vec![
                    run.reference("front", &front, &format!("{}-front.png", slug(model))),
                    character.clone(),
                ];
                run.job(model, "turnaround", &request).await;

                let Some(&ellipse) = masks.get(model) else {
                    println!("no mask for {model}; skipping its local revision");
                    continue;
                };
                let mask = out.join(slug(model)).join("mask.png");
                ellipse_mask(&mask, ellipse);
                let (base_width, base_height) = super::FRONT_SIZE;
                let mut request = GenerationRequest::new("compare", "ja", Mode::FrontLocalRevision);
                request.reference_keys = vec![character];
                request.local_edit = Some(LocalEdit {
                    base_image_path: front,
                    mask_image_path: mask,
                    edit_note: note.clone(),
                    base_width,
                    base_height,
                    feather_radius_px: 3,
                    subject: "head_shell",
                });
                run.job(model, "local", &request).await;
            }
        }
        "back" => {
            // A user back view tagged "back": once in the head shell
            // turnaround (approved front view + character + back view) and
            // once in the character design turnaround (character + back view).
            let back = PathBuf::from(env("MASKETTE_COMPARE_BACK"));
            for model in &models {
                let front = out.join(slug(model)).join("front.png");
                assert!(
                    front.is_file(),
                    "no front view for {model}; run the front step first"
                );
                let back_key = run.reference("back", &back, "back.png");

                let mut request = GenerationRequest::new("compare", "ja", Mode::Turnaround);
                request.reference_keys = vec![
                    run.reference("front", &front, &format!("{}-front.png", slug(model))),
                    run.reference("supplemental", &reference, &reference_name),
                    back_key.clone(),
                ];
                run.job(model, "turnaround-back", &request).await;

                let mut request =
                    GenerationRequest::new("compare", "ja", Mode::CharacterTurnaround);
                request.reference_keys = vec![
                    run.reference("front", &reference, &reference_name),
                    back_key,
                ];
                run.job(model, "design-turnaround-back", &request).await;
            }
        }
        other => panic!("MASKETTE_COMPARE_STEP is front, follow, back or report, not {other:?}"),
    }
    println!("report: {}", report::write(&out).display());
}

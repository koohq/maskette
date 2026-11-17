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
//! `local` makes only the local revision of `follow`, as the editor makes it
//! by default: `MASKETTE_COMPARE_EDIT_NOTE` (required here) inside the
//! ellipse of `MASKETTE_COMPARE_MASKS` on `<model>/front.png` in the output
//! folder (put any 800x1100 front view there), the edge blended into its
//! surroundings (`MASKETTE_COMPARE_LOCK_OUTSIDE=1` keeps the outside
//! instead, as `follow` does), and the character as the optional reference,
//! tagged "supplemental" like the editor's.
//! `MASKETTE_COMPARE_EDIT_REFERENCE=<image>` attaches that image instead,
//! and `none` attaches nothing. Like the editor, it gets no analysis.
//!
//! `turnaround` makes only the head shell turnaround of `follow`. With
//! `MASKETTE_COMPARE_PROMPTS=<prompts.json>`, any step uses those prompt
//! overrides (the app's `prompts.json` format), to try a text change
//! without rebuilding.
//!
//! In the app, the character design and the head shell front view also get
//! the details the reference analysis found ("Confirmed design facts" in the
//! prompt). Without `MASKETTE_COMPARE_ANALYSIS=<analysis.json>` they get
//! none, so the runs differ from what users get. With it, those runs carry
//! the analysis in that file (a project's stored `analysis`, which may be
//! edited by hand); when the file does not exist yet, the analysis is made
//! once as the chat's `analyze_references` makes it (the default analysis
//! model on OpenRouter, billed) and saved there, so every later run and
//! prompt variant gets the same details.
//!
//! `MASKETTE_COMPARE_EXTRA=<tag>=<image>` adds a reference to the `front`
//! step with a tag a user can give it in the app (`side`, `back`,
//! `expression`, `accessory` or `supplemental`); several are separated like
//! `PATH`. They follow the character as the user's other references do when
//! the chat makes the front view without a design (`skip_design`), each sent
//! as it is and described by its tag, and an analysis made for the run sees
//! them too, as the chat's `analyze_references` does. A stored analysis file
//! is used as it is. The other steps refuse them.
//!
//! Each step rebuilds `index.html` in the output folder, which shows the
//! runs side by side. `MASKETTE_COMPARE_STEP=report` only rebuilds it (no
//! model is called and no key is read).
//!
//! `samples` tries photos of real finished masks as examples of materials
//! and finish (`MASKETTE_COMPARE_SAMPLES=<photo>;<photo>`, separated like
//! `PATH`). It makes `front` with the app's prompt, then `front-samples1`,
//! `front-samples2`, ... with the first one, two, ... photos added (see
//! `samples.rs`; `MASKETTE_COMPARE_SAMPLE_COUNTS=0,1,2` picks the counts).
//! `MASKETTE_COMPARE_SAMPLE_STAGE=turnaround` makes the head shell
//! turnaround from each of those front views instead.
//! `MASKETTE_COMPARE_SAMPLE_TAG=parts` sends close-up crops of a lens eye,
//! the shell and the wig instead of whole photos (`front-parts3`), and
//! `partsgray` the same crops in black and white (`front-partsgray3`), so
//! that no face design or colour of the sample mask carries over. Give each character
//! its own output folder under one parent folder. `judge` compares pairs of
//! those runs with a vision model (see `judge.rs`), and `samples-report`
//! with the parent folder as `MASKETTE_COMPARE_OUT` builds the page that
//! puts every character side by side.

mod judge;
mod report;
mod samples;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde_json::{Value, json};

use super::analysis::{self, AnalysisRequest, AnalysisResult, Feature};
use super::{GenerationRequest, LocalEdit, Mode, image_api, image_backend};
use crate::backend::agent::store::REFERENCE_TAGS;
use crate::backend::ids;
use crate::backend::settings::secrets::{KeyringSecrets, SERVICE, SecretStore as _};
use crate::backend::tests::TestBackend;

const DEFAULT_EDIT_NOTE: &str = "星のヘアピンを、同じ金色のハート形のヘアピンに変えてください。";

/// The generations the app gives the analysed details to (`detail_lock` in
/// `agent/tools.rs`): the character design and the head shell front view.
const LOCKED_MODES: [Mode; 3] = [
    Mode::CharacterFront,
    Mode::CharacterTurnaround,
    Mode::FrontDesign,
];

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

/// The extra references of `MASKETTE_COMPARE_EXTRA`, as (tag, image).
fn parse_extras(text: &std::ffi::OsStr) -> Vec<(String, PathBuf)> {
    std::env::split_paths(text)
        .filter(|part| !part.as_os_str().is_empty())
        .map(|part| {
            let part = part.to_string_lossy();
            let (tag, image) = part.split_once('=').unwrap_or_else(|| {
                panic!("MASKETTE_COMPARE_EXTRA takes <tag>=<image>, not {part:?}")
            });
            let tag = tag.trim();
            assert!(
                REFERENCE_TAGS.contains(&tag) && tag != "front",
                "an extra reference is tagged one of {:?} but front, not {tag:?}",
                REFERENCE_TAGS
            );
            let image = PathBuf::from(image.trim());
            assert!(image.is_file(), "no extra reference at {}", image.display());
            (tag.to_owned(), image)
        })
        .collect()
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
    /// The analysed details for `LOCKED_MODES`, from `MASKETTE_COMPARE_ANALYSIS`.
    detail_lock: Option<Value>,
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

    /// Copies the extra references into the data folder and returns their
    /// keys, each with its tag.
    fn extra_references(&self, extras: &[(String, PathBuf)]) -> Vec<String> {
        extras
            .iter()
            .enumerate()
            .map(|(index, (tag, image))| {
                let extension = image
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("png");
                self.reference(tag, image, &format!("extra{}.{extension}", index + 1))
            })
            .collect()
    }

    /// The front view request: the character, then the extra references.
    fn front_request(&self, character: &Path, name: &str, extras: &[String]) -> GenerationRequest {
        let mut request = GenerationRequest::new("compare", "ja", Mode::FrontDesign);
        request.reference_keys = std::iter::once(self.reference("front", character, name))
            .chain(extras.iter().cloned())
            .collect();
        request
    }

    /// The editor's local revision of `front` (`run_local_revision` in
    /// `agent/tools.rs`): `ellipse` masked, `reference` (a key) as the
    /// optional reference, and the edge blended over 16 pixels, or 3 with
    /// `lock_outside`.
    fn local_request(
        &self,
        front: &Path,
        ellipse: (f64, f64, f64, f64),
        note: &str,
        reference: Option<String>,
        lock_outside: bool,
    ) -> GenerationRequest {
        let (base_width, base_height) = super::FRONT_SIZE;
        assert_eq!(
            image::image_dimensions(front).expect("front view readable"),
            (base_width, base_height),
            "the mask is drawn on a {base_width}x{base_height} front view, not {}",
            front.display()
        );
        let mask = front.with_file_name("mask.png");
        ellipse_mask(&mask, ellipse);
        let mut request = GenerationRequest::new("compare", "ja", Mode::FrontLocalRevision);
        request.free_text = note.to_owned();
        request.reference_keys = reference.into_iter().collect();
        request.local_edit = Some(LocalEdit {
            base_image_path: front.to_owned(),
            mask_image_path: mask,
            edit_note: note.to_owned(),
            base_width,
            base_height,
            feather_radius_px: if lock_outside { 3 } else { 16 },
            subject: "head_shell",
        });
        request
    }

    /// The stored analysis in `path`, or a new one of the character and the
    /// extra references made as the chat's `analyze_references` makes it and
    /// saved to `path`.
    async fn analysis(
        &self,
        path: &Path,
        character: &Path,
        name: &str,
        extras: &[String],
    ) -> Value {
        if path.is_file() {
            let text = std::fs::read_to_string(path).expect("analysis readable");
            println!("analysis read: {}", path.display());
            return serde_json::from_str(&text).expect("the analysis file is JSON");
        }
        let state = &self.backend.state;
        let backend =
            analysis::analysis_backend(&state.settings.settings()).expect("an analysis backend");
        let analysis_id = ids::uuid4();
        let request = AnalysisRequest {
            locale: "ja".to_owned(),
            reference_keys: std::iter::once(self.reference("front", character, name))
                .chain(extras.iter().cloned())
                .collect(),
            character_session_id: "compare".to_owned(),
            analysis_id: analysis_id.clone(),
            ..AnalysisRequest::default()
        };
        let result = match backend.analyze(state, &request).await {
            Ok(result) => result,
            Err(error) => panic!("the analysis failed: {error:?}"),
        };
        let result =
            analysis::ensure_user_requirement(analysis::filter_head_details(result), "", "ja");
        let stored = analysis::stored_analysis(&analysis_id, "", &result);
        let text = serde_json::to_string_pretty(&stored).expect("analysis as JSON");
        std::fs::write(path, text).expect("analysis written");
        println!("analysis saved: {}", path.display());
        stored
    }

    /// Runs one job, keeps its clean output as `<model>/<label>.png` and
    /// appends what happened to `results.jsonl`.
    async fn job(&self, model: &str, label: &str, request: &GenerationRequest) -> Option<PathBuf> {
        self.use_model(model);
        let mut request = request.clone();
        if request.detail_lock.is_none() && LOCKED_MODES.contains(&request.generation_mode) {
            request.detail_lock.clone_from(&self.detail_lock);
        }
        let started = Instant::now();
        let result = async {
            image_backend(&self.backend.state.settings.settings())?
                .generate(
                    &self.backend.state,
                    &request,
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
    if step == "samples-report" {
        println!("page: {}", samples::write_page(&out).display());
        return;
    }
    let models: Vec<String> = env("MASKETTE_COMPARE_MODELS")
        .split(',')
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect();
    let extras = std::env::var_os("MASKETTE_COMPARE_EXTRA")
        .map(|text| parse_extras(&text))
        .unwrap_or_default();
    assert!(
        extras.is_empty() || step == "front",
        "MASKETTE_COMPARE_EXTRA only goes with the front step"
    );
    // An analysis would be made (and billed) for nothing.
    assert!(
        step != "local" || std::env::var_os("MASKETTE_COMPARE_ANALYSIS").is_none(),
        "the local step takes no MASKETTE_COMPARE_ANALYSIS; the editor's local revision gets none"
    );

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
    if step == "judge" {
        let judge_model = std::env::var("MASKETTE_COMPARE_JUDGE")
            .unwrap_or_else(|_| judge::DEFAULT_MODEL.to_owned());
        let pairs = judge::parse_pairs(&env("MASKETTE_COMPARE_PAIRS"));
        let kinds = judge::parse_kinds(
            std::env::var("MASKETTE_COMPARE_JUDGE_KINDS")
                .ok()
                .as_deref(),
        );
        judge::run(
            &reqwest::Client::new(),
            &key,
            &judge_model,
            &kinds,
            &out,
            &models,
            &pairs,
        )
        .await;
        return;
    }

    let reference = PathBuf::from(env("MASKETTE_COMPARE_REFERENCE"));
    // The report shows the character next to the results.
    let extension = reference
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("png");
    std::fs::copy(&reference, out.join(format!("reference.{extension}")))
        .expect("reference copied");
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            ("image_provider".to_owned(), Some("openrouter".to_owned())),
            ("openrouter_api_key".to_owned(), Some(key.clone())),
            // The analysis goes to the chat assistant's endpoint, OpenRouter by default.
            ("agent_llm_api_key".to_owned(), Some(key)),
        ])
        .expect("settings set");
    // Prompt text to try, in the format of the app's prompts.json.
    if let Ok(prompts) = std::env::var("MASKETTE_COMPARE_PROMPTS") {
        std::fs::copy(
            &prompts,
            backend.dir.path().join(crate::backend::prompts::FILE_NAME),
        )
        .expect("prompts file copied");
    }
    let mut run = Run {
        backend: &backend,
        out: out.clone(),
        detail_lock: None,
    };
    let reference_name = format!("character.{extension}");
    let extra_keys = run.extra_references(&extras);
    if let Ok(path) = std::env::var("MASKETTE_COMPARE_ANALYSIS") {
        let stored = run
            .analysis(Path::new(&path), &reference, &reference_name, &extra_keys)
            .await;
        run.detail_lock = Some(analysis::detail_lock(&stored));
    }

    match step.as_str() {
        "front" => {
            for model in &models {
                let request = run.front_request(&reference, &reference_name, &extra_keys);
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
                let request = run.local_request(&front, ellipse, &note, Some(character), true);
                run.job(model, "local", &request).await;
            }
        }
        "local" => {
            let masks = parse_masks(&env("MASKETTE_COMPARE_MASKS"));
            let note = env("MASKETTE_COMPARE_EDIT_NOTE");
            let lock_outside =
                std::env::var("MASKETTE_COMPARE_LOCK_OUTSIDE").is_ok_and(|value| value == "1");
            // Everything is checked before the first billed call.
            for model in &models {
                assert!(
                    out.join(slug(model)).join("front.png").is_file(),
                    "no front view for {model}; put one at <out>/{}/front.png",
                    slug(model)
                );
                assert!(masks.contains_key(model), "no mask for {model}");
            }
            let edit_reference = match std::env::var_os("MASKETTE_COMPARE_EDIT_REFERENCE") {
                None => Some(run.reference("supplemental", &reference, &reference_name)),
                Some(value) if value == "none" => None,
                Some(value) => {
                    let image = PathBuf::from(value);
                    assert!(image.is_file(), "no edit reference at {}", image.display());
                    let extension = image
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("png");
                    Some(run.reference(
                        "supplemental",
                        &image,
                        &format!("edit-reference.{extension}"),
                    ))
                }
            };
            for model in &models {
                let front = out.join(slug(model)).join("front.png");
                let request = run.local_request(
                    &front,
                    masks[model],
                    &note,
                    edit_reference.clone(),
                    lock_outside,
                );
                run.job(model, "local", &request).await;
            }
        }
        "turnaround" => {
            // Only the head shell turnaround of the follow step, for trying
            // prompt text on the four-view sheet.
            for model in &models {
                let front = out.join(slug(model)).join("front.png");
                assert!(
                    front.is_file(),
                    "no front view for {model}; run the front step first"
                );
                let mut request = GenerationRequest::new("compare", "ja", Mode::Turnaround);
                request.reference_keys = vec![
                    run.reference("front", &front, &format!("{}-front.png", slug(model))),
                    run.reference("supplemental", &reference, &reference_name),
                ];
                run.job(model, "turnaround", &request).await;
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
        "samples" => {
            let paths: Vec<PathBuf> =
                std::env::split_paths(&env("MASKETTE_COMPARE_SAMPLES")).collect();
            let tag = std::env::var("MASKETTE_COMPARE_SAMPLE_TAG")
                .unwrap_or_else(|_| "samples".to_owned());
            assert!(
                samples::TAGS.contains(&tag.as_str()),
                "MASKETTE_COMPARE_SAMPLE_TAG is one of {:?}, not {tag:?}",
                samples::TAGS
            );
            let samples = samples::load_samples(&paths, &tag, &out);
            let counts: Vec<usize> = std::env::var("MASKETTE_COMPARE_SAMPLE_COUNTS").map_or_else(
                |_| (0..=samples.images.len()).collect(),
                |text| {
                    text.split(',')
                        .map(|count| count.trim().parse().expect("a count"))
                        .collect()
                },
            );
            assert!(
                counts.iter().all(|count| *count <= samples.images.len()),
                "more samples asked for than given"
            );
            let stage = std::env::var("MASKETTE_COMPARE_SAMPLE_STAGE")
                .unwrap_or_else(|_| "front".to_owned());
            samples::run_stage(
                &run,
                &models,
                &stage,
                &reference,
                &reference_name,
                &samples,
                &counts,
            )
            .await;
            if let Some(parent) = out.parent() {
                println!("page: {}", samples::write_page(parent).display());
            }
            return;
        }
        other => panic!(
            "MASKETTE_COMPARE_STEP is front, follow, local, turnaround, back, samples, judge, report or samples-report, not {other:?}"
        ),
    }
    println!("report: {}", report::write(&out).display());
}

/// The harness gives the analysed details to the same generations as the
/// app, with no model called: the analysis comes from a file and the images
/// from the fixture backend.
#[tokio::test]
async fn analysis_reaches_only_the_locked_generations() {
    let backend = TestBackend::new(true);
    let out = tempfile::tempdir().expect("output folder");
    let character = out.path().join("character.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([240, 180, 150]))
        .save(&character)
        .expect("character written");
    let result = AnalysisResult {
        features: vec![Feature {
            id: "hair-1".to_owned(),
            kind: "hair".to_owned(),
            label: "髪".to_owned(),
            description: "明るいピーチ色のボブ".to_owned(),
        }],
        warnings: Vec::new(),
    };
    let file = out.path().join("analysis.json");
    let stored = analysis::stored_analysis("analysis-1", "", &result);
    std::fs::write(&file, stored.to_string()).expect("analysis written");

    let mut run = Run {
        backend: &backend,
        out: out.path().to_owned(),
        detail_lock: None,
    };
    let read = run.analysis(&file, &character, "character.png", &[]).await;
    assert_eq!(read, stored);
    run.detail_lock = Some(analysis::detail_lock(&read));

    let front_key = run.reference("front", &character, "character.png");
    let mut front = GenerationRequest::new("compare", "ja", Mode::FrontDesign);
    front.reference_keys = vec![front_key.clone()];
    assert!(run.job("vendor/model", "front", &front).await.is_some());
    let mut turnaround = GenerationRequest::new("compare", "ja", Mode::Turnaround);
    turnaround.reference_keys = vec![front_key];
    assert!(
        run.job("vendor/model", "turnaround", &turnaround)
            .await
            .is_some()
    );

    // The fixture backend writes no prompt text, but each job keeps its
    // request, which the prompt is built from.
    let lock_of = |mode: &str| {
        let records = backend.state.generation.jobs.list();
        let record = records
            .iter()
            .find(|record| record.generation_mode == mode)
            .unwrap_or_else(|| panic!("a {mode} job"));
        record.prompt_payload["detail_lock"].clone()
    };
    assert_eq!(lock_of("front_design"), analysis::detail_lock(&stored));
    assert_eq!(
        lock_of("front_design")["features"][0]["description"],
        "明るいピーチ色のボブ"
    );
    assert_eq!(lock_of("turnaround"), Value::Null);
    // The prompt's "Confirmed design facts" section is built from that lock.
    let facts = super::prompt::format_detail_lock(Some(&lock_of("front_design")));
    assert!(facts.contains("- hair: 明るいピーチ色のボブ"), "{facts}");
}

/// An extra reference goes after the character, uncut and described by its
/// tag, as a tagged user reference does in the app; no model is called.
#[tokio::test]
async fn extra_references_follow_the_character_with_their_tags() {
    let backend = TestBackend::new(true);
    let out = tempfile::tempdir().expect("output folder");
    let character = out.path().join("character.png");
    image::RgbImage::from_pixel(64, 64, image::Rgb([240, 180, 150]))
        .save(&character)
        .expect("character written");
    let eyes = out.path().join("eyes.png");
    image::RgbImage::from_pixel(96, 40, image::Rgb([80, 160, 90]))
        .save(&eyes)
        .expect("eyes written");
    let text =
        std::env::join_paths([format!("expression={}", eyes.display())]).expect("a path list");
    let extras = parse_extras(&text);
    assert_eq!(extras, vec![("expression".to_owned(), eyes)]);

    let run = Run {
        backend: &backend,
        out: out.path().to_owned(),
        detail_lock: None,
    };
    let keys = run.extra_references(&extras);
    assert_eq!(keys, vec!["expression:references/compare/extra1.png"]);
    let request = run.front_request(&character, "character.png", &keys);
    let prepared =
        image_api::prepare(backend.dir.path(), &request, 4, None, None).expect("images prepared");
    assert_eq!(prepared.images.len(), 2);
    assert_eq!(prepared.images[1].dimensions(), (96, 40));
    assert!(
        prepared
            .prompt
            .starts_with("Image 1 is the character reference.")
            && prepared
                .prompt
                .contains("\nImage 2 shows other expressions"),
        "{}",
        prepared.prompt
    );
    assert!(run.job("vendor/model", "front", &request).await.is_some());
}

/// The local step sends what the editor sends by default: the cut-out of
/// the masked area, the optional reference as Image 2, an edge blended over
/// 16 pixels and no analysis; no model is called.
#[tokio::test]
async fn the_local_step_sends_what_the_editor_sends() {
    let backend = TestBackend::new(true);
    let out = tempfile::tempdir().expect("output folder");
    let folder = out.path().join(slug("vendor/model"));
    std::fs::create_dir_all(&folder).expect("model folder");
    let front = folder.join("front.png");
    let (width, height) = super::FRONT_SIZE;
    image::RgbImage::from_pixel(width, height, image::Rgb([230, 220, 210]))
        .save(&front)
        .expect("front view written");
    let eyes = out.path().join("eyes.png");
    image::RgbImage::from_pixel(96, 40, image::Rgb([80, 160, 90]))
        .save(&eyes)
        .expect("eyes written");

    let run = Run {
        backend: &backend,
        out: out.path().to_owned(),
        detail_lock: Some(json!({"features": [{"kind": "eyes", "description": "丸い瞳孔"}]})),
    };
    let key = run.reference("supplemental", &eyes, "edit-reference.png");
    let request = run.local_request(
        &front,
        (250.0, 400.0, 550.0, 520.0),
        "Redraw only the eyes.",
        Some(key),
        false,
    );
    let edit = request.local_edit.as_ref().expect("a local edit");
    assert_eq!(edit.feather_radius_px, 16);
    assert_eq!(edit.subject, "head_shell");
    assert_eq!(edit.mask_image_path, folder.join("mask.png"));
    assert_eq!(request.free_text, "Redraw only the eyes.");

    let prepared =
        image_api::prepare(backend.dir.path(), &request, 4, None, None).expect("images prepared");
    assert_eq!(prepared.images.len(), 2);
    assert_eq!(prepared.images[1].dimensions(), (96, 40));
    assert!(
        prepared
            .prompt
            .contains("Apply this local edit to Image 1: Redraw only the eyes.")
            && prepared
                .prompt
                .contains("\nImage 2 is a supplemental reference"),
        "{}",
        prepared.prompt
    );
    assert!(run.job("vendor/model", "local", &request).await.is_some());
    let records = backend.state.generation.jobs.list();
    let record = records.last().expect("a job");
    assert_eq!(record.prompt_payload["detail_lock"], Value::Null);

    let locked = run.local_request(&front, (250.0, 400.0, 550.0, 520.0), "x", None, true);
    assert_eq!(
        locked.local_edit.expect("a local edit").feather_radius_px,
        3
    );
    assert!(locked.reference_keys.is_empty());
}

#[test]
#[should_panic(expected = "but front")]
fn an_extra_reference_cannot_be_a_second_front() {
    let out = tempfile::tempdir().expect("output folder");
    let image = out.path().join("other.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([0, 0, 0]))
        .save(&image)
        .expect("image written");
    let text = std::env::join_paths([format!("front={}", image.display())]).expect("a path list");
    parse_extras(&text);
}

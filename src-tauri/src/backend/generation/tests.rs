//! The real backends against stand-in servers: an OpenAI-compatible chat
//! API for analysis and the OpenRouter Image API for generation.

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use base64::Engine as _;
use http_body_util::BodyExt;
use image::{GenericImageView, Rgb, RgbImage};
use serde_json::{Value, json};

use super::analysis::{self, AnalysisError, AnalysisRequest};
use super::{GenerationRequest, LocalEdit, Mode, image_backend};
use crate::backend::tests::TestBackend;

/// A canned answer.
#[derive(Clone)]
enum Answer {
    Json(u16, Value),
    /// A streamed chat completion with this text.
    Chat(String),
    /// Never answers.
    Hang,
    /// An image URL on this server.
    DownloadUrl,
}

#[derive(Default)]
struct Recorded {
    images: Vec<Value>,
    chats: Vec<Value>,
    model_listings: usize,
}

struct Fake {
    base_url: String,
    recorded: Arc<Mutex<Recorded>>,
}

async fn fake(models: Value, answers: Vec<Answer>) -> Fake {
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
    let recorded = Arc::new(Mutex::new(Recorded::default()));
    let next = {
        let answers = answers.clone();
        move || answers.lock().unwrap().pop_front().expect("an answer left")
    };
    let respond = |answer: Answer, host: String| async move {
        match answer {
            Answer::DownloadUrl => {
                axum::Json(json!({"data": [{"url": format!("http://{host}/files/out.png")}]}))
                    .into_response()
            }
            Answer::Json(status, body) => {
                (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response()
            }
            Answer::Chat(text) => {
                let delta = json!({"choices": [{"delta": {"content": text}}]});
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    format!("data: {delta}\n\ndata: [DONE]\n\n"),
                )
                    .into_response()
            }
            Answer::Hang => {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                StatusCode::GATEWAY_TIMEOUT.into_response()
            }
        }
    };
    let body_of = |request: Request| async move {
        let bytes = request.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let app = Router::new()
        .route(
            "/api/v1/images/models",
            get({
                let recorded = recorded.clone();
                move || {
                    recorded.lock().unwrap().model_listings += 1;
                    let models = models.clone();
                    async move { axum::Json(models) }
                }
            }),
        )
        .route(
            "/api/v1/images",
            post({
                let (recorded, next) = (recorded.clone(), next.clone());
                move |request: Request| async move {
                    let host = host_of(&request);
                    let body = body_of(request).await;
                    recorded.lock().unwrap().images.push(body);
                    respond(next(), host).await
                }
            }),
        )
        .route(
            "/api/v1/chat/completions",
            post({
                let recorded = recorded.clone();
                move |request: Request| async move {
                    let host = host_of(&request);
                    let body = body_of(request).await;
                    recorded.lock().unwrap().chats.push(body);
                    respond(next(), host).await
                }
            }),
        )
        .route(
            "/files/out.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], png(300, 200)) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/api/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fake { base_url, recorded }
}

fn host_of(request: &Request) -> String {
    request
        .headers()
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgb([200, 120, 120])
        } else {
            Rgb([120, 120, 200])
        }
    });
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn b64_image(width: u32, height: u32) -> Answer {
    let data = base64::engine::general_purpose::STANDARD.encode(png(width, height));
    Answer::Json(
        200,
        json!({"data": [{"b64_json": data}], "usage": {"cost": 0.04}}),
    )
}

fn models() -> Value {
    json!({"data": [{
        "id": "vendor/image-model",
        "supported_parameters": {
            "aspect_ratio": {"values": ["1:1", "2:3", "3:2", "3:4"]},
            "resolution": {"values": ["1K", "2K"]},
            "input_references": {"max": 3},
        },
    }]})
}

/// A backend set up for OpenRouter on the fake, with one front reference.
fn openrouter_backend(fake: &Fake) -> (TestBackend, String) {
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            ("image_provider".to_owned(), Some("openrouter".to_owned())),
            (
                "openrouter_base_url".to_owned(),
                Some(fake.base_url.clone()),
            ),
            ("openrouter_api_key".to_owned(), Some("or-key".to_owned())),
            (
                "openrouter_image_model".to_owned(),
                Some("vendor/image-model".to_owned()),
            ),
        ])
        .unwrap();
    let key = write_reference(&backend, "front.png", 640, 480);
    (backend, key)
}

fn write_reference(backend: &TestBackend, name: &str, width: u32, height: u32) -> String {
    let folder = backend.dir.path().join("references").join("upload-1");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(name), png(width, height)).unwrap();
    format!("references/upload-1/{name}")
}

fn front_request(key: &str) -> GenerationRequest {
    let mut request = GenerationRequest::new("session-1", "ja", Mode::FrontDesign);
    request.reference_keys = vec![format!("front:{key}")];
    request.free_text = "keep the bow".to_owned();
    request
}

async fn generate(
    backend: &TestBackend,
    request: &GenerationRequest,
) -> Result<super::GeneratedImage, crate::backend::failure::Failure> {
    let image_backend = image_backend(&backend.state.settings.settings())?;
    image_backend
        .generate(&backend.state, request, &AtomicBool::new(false), &|_, _| {})
        .await
}

#[tokio::test]
async fn job_steps_reach_the_observer_and_usage_is_recorded() {
    let usage = json!({
        "prompt_tokens": 3573,
        "completion_tokens": 3052,
        "completion_tokens_details": {"image_tokens": 1680, "reasoning_tokens": 0},
        "cost": 0.066,
    });
    let data = base64::engine::general_purpose::STANDARD.encode(png(768, 1024));
    let answer = Answer::Json(200, json!({"data": [{"b64_json": data}], "usage": usage}));
    let fake = fake(models(), vec![answer]).await;
    let (backend, key) = openrouter_backend(&fake);
    let steps = std::sync::Mutex::new(Vec::new());
    let observe =
        |progress: u32, phase: &str| steps.lock().unwrap().push((progress, phase.to_owned()));
    let image_backend = image_backend(&backend.state.settings.settings()).unwrap();
    let output = image_backend
        .generate(
            &backend.state,
            &front_request(&key),
            &AtomicBool::new(false),
            &observe,
        )
        .await
        .unwrap();

    assert_eq!(
        *steps.lock().unwrap(),
        vec![
            (15, "preparing_references".to_owned()),
            (45, "generating".to_owned()),
            (90, "saving_outputs".to_owned()),
        ]
    );
    let record = backend.state.generation.jobs.get(&output.job_id).unwrap();
    let stored = record.token_usage.unwrap();
    assert_eq!(stored["input_tokens"], 3573);
    assert_eq!(stored["output_tokens"], 3052);
    assert_eq!(stored["reasoning_output_tokens"], 0);
    assert_eq!(stored["total_tokens"], 6625);
    assert_eq!(stored["cost"], 0.066);
    assert!(
        record
            .events
            .iter()
            .any(|event| event.kind == "token_usage")
    );
    assert_eq!(record.status, "succeeded");
}

#[tokio::test]
async fn answers_without_token_counts_leave_usage_empty() {
    // `b64_image` reports only a cost.
    let fake = fake(models(), vec![b64_image(768, 1024)]).await;
    let (backend, key) = openrouter_backend(&fake);
    let output = generate(&backend, &front_request(&key)).await.unwrap();
    let record = backend.state.generation.jobs.get(&output.job_id).unwrap();
    assert_eq!(record.token_usage, None);
}

#[tokio::test]
async fn openrouter_generates_a_labelled_front_view() {
    let fake = fake(models(), vec![b64_image(768, 1024)]).await;
    let (backend, key) = openrouter_backend(&fake);
    let mut request = front_request(&key);
    let extra = write_reference(&backend, "side.png", 200, 200);
    request.reference_keys.push(format!("supplemental:{extra}"));
    let output = generate(&backend, &request).await.unwrap();

    let recorded = fake.recorded.lock().unwrap();
    let sent = &recorded.images[0];
    assert_eq!(sent["model"], "vendor/image-model");
    assert_eq!(sent["aspect_ratio"], "3:4");
    assert_eq!(sent["resolution"], "1K");
    assert!(sent.get("n").is_none());
    let references = sent["input_references"].as_array().unwrap();
    assert_eq!(references.len(), 2);
    assert!(
        references[0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    let prompt = sent["prompt"].as_str().unwrap();
    assert!(prompt.starts_with("Image 1 is the character reference."));
    assert!(prompt.contains("Image 2 contains supplemental references"));
    assert!(prompt.contains("porcelain-pale skin paint"));
    assert!(prompt.ends_with("keep the bow"));

    // The public image carries the label; the clean copy does not.
    assert_eq!((output.width, output.height), (800, 1100));
    assert!(output.url.starts_with("/api/generated/session-1/"));
    let public = super::generated_file_from_url(backend.dir.path(), &output.url).unwrap();
    let public = image::open(public).unwrap().to_rgb8();
    let clean = image::open(&output.clean_path).unwrap().to_rgb8();
    assert_eq!(public.dimensions(), clean.dimensions());
    assert_ne!(public, clean);
    assert_eq!(clean.get_pixel(20, 550).0, [200, 120, 120]);

    let job = backend.state.generation.jobs.get(&output.job_id).unwrap();
    assert_eq!(job.status, "succeeded");
    assert_eq!(job.provider, "openrouter");
    assert_eq!(job.model, "vendor/image-model");
    assert_eq!(job.prompt, prompt);
    assert_eq!(job.request["payload"]["input_references"], "[2 image(s)]");
    assert_eq!(job.outputs[0].image_url, output.url);
    let steps: Vec<&str> = job.events.iter().map(|event| event.kind.as_str()).collect();
    assert_eq!(
        steps,
        [
            "queued",
            "preparing_references",
            "generating",
            "saving_outputs",
            "candidate_ready",
            "succeeded"
        ]
    );
}

#[tokio::test]
async fn openrouter_retries_failed_generations_and_reuses_the_model_list() {
    let fake = fake(
        models(),
        vec![
            Answer::Json(502, json!({"error": {"message": "upstream failed"}})),
            b64_image(64, 64),
            Answer::Json(400, json!({"error": {"message": "bad prompt"}})),
        ],
    )
    .await;
    let (backend, key) = openrouter_backend(&fake);
    generate(&backend, &front_request(&key)).await.unwrap();
    let error = generate(&backend, &front_request(&key)).await.unwrap_err();
    assert!(
        error
            .message
            .contains("OpenRouter image generation failed: HTTP 400")
            && error.message.contains("bad prompt"),
        "{error}"
    );
    // The screen gets the status and the API's own words.
    assert_eq!(error.code, Some("api_http_error"));
    assert_eq!(
        Value::Object(error.params),
        json!({"service": "OpenRouter", "status": 400, "detail": "bad prompt"})
    );
    let recorded = fake.recorded.lock().unwrap();
    assert_eq!(recorded.images.len(), 3);
    assert_eq!(recorded.model_listings, 1);
    let failed: Vec<_> = backend
        .state
        .generation
        .jobs
        .list()
        .into_iter()
        .filter(|job| job.status == "failed")
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0]
            .events
            .last()
            .unwrap()
            .message
            .contains("bad prompt")
    );
}

#[tokio::test]
async fn turnarounds_are_padded_to_three_by_two_without_upscaling() {
    // An unknown model: every documented option goes out.
    let fake = fake(json!({"data": []}), vec![b64_image(1200, 1000)]).await;
    let (backend, key) = openrouter_backend(&fake);
    let mut request = GenerationRequest::new("session-1", "ja", Mode::Turnaround);
    request.reference_keys = vec![format!("front:{key}")];
    let output = generate(&backend, &request).await.unwrap();
    assert_eq!((output.width, output.height), (1500, 1000));
    let recorded = fake.recorded.lock().unwrap();
    let sent = &recorded.images[0];
    assert_eq!(sent["n"], 1);
    assert_eq!(sent["aspect_ratio"], "3:2");
    assert_eq!(sent["resolution"], "2K");
    assert!(
        sent["prompt"]
            .as_str()
            .unwrap()
            .contains("Image 1 is the approved front-view")
    );
}

#[tokio::test]
async fn image_urls_in_the_answer_are_downloaded() {
    let fake = fake(models(), vec![Answer::DownloadUrl]).await;
    let (backend, key) = openrouter_backend(&fake);
    let output = generate(&backend, &front_request(&key)).await.unwrap();
    let clean = image::open(&output.clean_path).unwrap();
    assert_eq!(clean.dimensions(), (800, 1100));
}

#[tokio::test]
async fn a_local_revision_only_changes_the_masked_area() {
    let fake = fake(models(), vec![b64_image(512, 512)]).await;
    let (backend, key) = openrouter_backend(&fake);
    let folder = backend.dir.path().join("local");
    std::fs::create_dir_all(&folder).unwrap();
    let base = RgbImage::from_pixel(800, 1100, Rgb([30, 30, 30]));
    base.save(folder.join("base.png")).unwrap();
    let mask = image::GrayImage::from_fn(800, 1100, |x, y| {
        image::Luma([if (300..400).contains(&x) && (500..600).contains(&y) {
            255
        } else {
            0
        }])
    });
    mask.save(folder.join("mask.png")).unwrap();
    let mut request = GenerationRequest::new("session-1", "ja", Mode::FrontLocalRevision);
    request.reference_keys = vec![format!("supplemental:{key}")];
    request.local_edit = Some(LocalEdit {
        base_image_path: folder.join("base.png"),
        mask_image_path: folder.join("mask.png"),
        edit_note: "redden the cheek".to_owned(),
        base_width: 800,
        base_height: 1100,
        feather_radius_px: 3,
        subject: "head_shell",
    });
    let output = generate(&backend, &request).await.unwrap();
    let clean = image::open(&output.clean_path).unwrap().to_rgb8();
    assert_eq!(clean.dimensions(), (800, 1100));
    assert_eq!(clean.get_pixel(100, 100).0, [30, 30, 30]);
    assert_eq!(clean.get_pixel(299, 550).0, [30, 30, 30]);
    assert_ne!(clean.get_pixel(350, 550).0, [30, 30, 30]);

    let recorded = fake.recorded.lock().unwrap();
    let sent = &recorded.images[0];
    let prompt = sent["prompt"].as_str().unwrap();
    assert!(prompt.contains("Apply this local edit to Image 1: redden the cheek"));
    assert!(prompt.contains("Image 2 is a supplemental reference"));
    assert_eq!(sent["input_references"].as_array().unwrap().len(), 2);
    // The crop: the 100 px square plus margins, grown to 384 px.
    assert_eq!(sent["aspect_ratio"], "1:1");
}

/// Both eyes of a front view: the box with its margins is 624x384, which
/// goes out widened to 624x416 for the 3:2 the model answers in.
#[tokio::test]
async fn a_local_revision_sends_its_crop_at_the_ratio_it_asks_for() {
    let fake = fake(models(), vec![b64_image(1536, 1024)]).await;
    let (backend, _) = openrouter_backend(&fake);
    let folder = backend.dir.path().join("local");
    std::fs::create_dir_all(&folder).unwrap();
    RgbImage::from_pixel(800, 1100, Rgb([30, 30, 30]))
        .save(folder.join("base.png"))
        .unwrap();
    image::GrayImage::from_fn(800, 1100, |x, y| {
        image::Luma([if (168..648).contains(&x) && (474..638).contains(&y) {
            255
        } else {
            0
        }])
    })
    .save(folder.join("mask.png"))
    .unwrap();
    let mut request = GenerationRequest::new("session-1", "ja", Mode::FrontLocalRevision);
    request.local_edit = Some(LocalEdit {
        base_image_path: folder.join("base.png"),
        mask_image_path: folder.join("mask.png"),
        edit_note: "redraw the irises".to_owned(),
        base_width: 800,
        base_height: 1100,
        feather_radius_px: 16,
        subject: "head_shell",
    });
    let output = generate(&backend, &request).await.unwrap();
    let clean = image::open(&output.clean_path).unwrap().to_rgb8();
    assert_eq!(clean.dimensions(), (800, 1100));
    assert_eq!(clean.get_pixel(400, 473).0, [30, 30, 30]);
    assert_ne!(clean.get_pixel(400, 556).0, [30, 30, 30]);

    let recorded = fake.recorded.lock().unwrap();
    let sent = &recorded.images[0];
    assert_eq!(sent["aspect_ratio"], "3:2");
    let url = sent["input_references"][0]["image_url"]["url"]
        .as_str()
        .unwrap();
    let data = url.strip_prefix("data:image/png;base64,").unwrap();
    let crop = image::load_from_memory(
        &base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(crop.dimensions(), (624, 416));
    let job = backend.state.generation.jobs.list().pop().unwrap();
    assert_eq!(job.request["image_sizes"], json!(["624x416"]));
}

#[tokio::test]
async fn a_cancelled_job_stops_waiting() {
    let fake = fake(models(), vec![Answer::Hang]).await;
    let (backend, key) = openrouter_backend(&fake);
    let image_backend = image_backend(&backend.state.settings.settings()).unwrap();
    let cancelled = AtomicBool::new(false);
    let request = front_request(&key);
    let run = image_backend.generate(&backend.state, &request, &cancelled, &|_, _| {});
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
    };
    let (result, ()) =
        tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(run, cancel) })
            .await
            .expect("the job stopped");
    let error = result.unwrap_err();
    assert_eq!(error.message, "Cancelled by the user.");
    assert_eq!(error.code, Some("cancelled"));
    let jobs = backend.state.generation.jobs.list();
    assert_eq!(jobs[0].status, "cancelled");
}

#[tokio::test]
async fn openrouter_needs_a_model_and_a_key() {
    let backend = TestBackend::new(false);
    let settings = |changes: &[(&str, &str)]| {
        let changes: Vec<(String, Option<String>)> = changes
            .iter()
            .map(|(key, value)| ((*key).to_owned(), Some((*value).to_owned())))
            .collect();
        backend.state.settings.update(&changes).unwrap();
        image_backend(&backend.state.settings.settings()).map(|_| ())
    };
    // Release builds generate with OpenRouter and Nano Banana 2.1 unless set.
    let error = settings(&[]).unwrap_err();
    assert!(error.message.contains("openrouter_api_key is required"));
    assert_eq!(error.code, Some("api_key_missing"));
    assert_eq!(error.params["settings"], "openrouter_api_key");
    let error = settings(&[
        ("image_provider", "openrouter"),
        ("openrouter_image_model", ""),
    ])
    .unwrap_err();
    assert!(error.message.contains("openrouter_image_model is required"));
    assert_eq!(error.code, Some("setting_missing"));
    assert_eq!(error.params["settings"], "openrouter_image_model");
    assert!(
        settings(&[("openrouter_image_model", "m")])
            .unwrap_err()
            .message
            .contains("openrouter_api_key is required")
    );
    // The analysis key is reused when analysis also goes to OpenRouter.
    settings(&[
        ("analysis_llm_base_url", "https://openrouter.ai/api/v1/"),
        ("analysis_llm_api_key", "k"),
    ])
    .unwrap();
    settings(&[("image_provider", "codex")]).unwrap();
    assert!(
        settings(&[("image_provider", "ark")])
            .unwrap_err()
            .message
            .contains("ark_api_key is required")
    );
}

// --- Analysis -----------------------------------------------------------------

fn analysis_backend(fake: &Fake) -> (TestBackend, String) {
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            (
                "llm_provider".to_owned(),
                Some("openai_compatible".to_owned()),
            ),
            (
                "analysis_llm_base_url".to_owned(),
                Some(fake.base_url.clone()),
            ),
            ("analysis_llm_api_key".to_owned(), Some("an-key".to_owned())),
            (
                "analysis_llm_model".to_owned(),
                Some("vision-model".to_owned()),
            ),
            (
                "analysis_llm_extra_body".to_owned(),
                Some(r#"{"reasoning": {"effort": "medium"}}"#.to_owned()),
            ),
        ])
        .unwrap();
    let key = write_reference(&backend, "front.png", 2000, 1000);
    (backend, key)
}

fn analysis_request(key: &str) -> AnalysisRequest {
    AnalysisRequest {
        free_text: "keep the cat ears".to_owned(),
        locale: "ja".to_owned(),
        reference_keys: vec![format!("front:{key}")],
        reference_descriptions: Vec::new(),
        character_session_id: "session-1".to_owned(),
        analysis_id: "analysis-1".to_owned(),
    }
}

async fn analyze(
    backend: &TestBackend,
    request: &AnalysisRequest,
) -> Result<analysis::AnalysisResult, AnalysisError> {
    let analysis_backend = analysis::analysis_backend(&backend.state.settings.settings())
        .map_err(AnalysisError::Failed)?;
    analysis_backend.analyze(&backend.state, request).await
}

#[tokio::test]
async fn analysis_checks_safety_then_lists_the_details() {
    let fake = fake(
        Value::Null,
        vec![
            Answer::Chat(r#"{"allowed": true, "reason": "ok", "message": ""}"#.to_owned()),
            Answer::Chat(
                "Here you go:\n```json\n{\"features\": [{\"id\": \"f1\", \"kind\": \"cat_ears\", \"label\": \"Ears\", \"description\": \"black cat ears\"}], \"crops\": [{\"id\": \"c\", \"kind\": \"ears\", \"description\": \"x\", \"source_reference_key\": \"k\", \"box_2d\": [0, 0, 500, 500]}], \"warnings\": []}\n```"
                    .to_owned(),
            ),
        ],
    )
    .await;
    let (backend, key) = analysis_backend(&fake);
    let result = analyze(&backend, &analysis_request(&key)).await.unwrap();
    assert_eq!(result.features.len(), 1);
    assert_eq!(result.features[0].kind, "ears");
    assert_eq!(result.features[0].description, "black cat ears");

    let recorded = fake.recorded.lock().unwrap();
    assert_eq!(recorded.chats.len(), 2);
    let first = &recorded.chats[0];
    assert_eq!(first["model"], "vision-model");
    assert_eq!(first["stream"], true);
    assert_eq!(first["reasoning"]["effort"], "medium");
    let content = first["messages"][0]["content"].as_array().unwrap();
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Decide whether the uploaded images are usable")
    );
    // The reference is shrunk to 1536 px on the longer side.
    let url = content[1]["image_url"]["url"].as_str().unwrap();
    let encoded = url.strip_prefix("data:image/png;base64,").unwrap();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().dimensions(),
        (1536, 768)
    );
    let second = recorded.chats[1]["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(second.contains("Write every feature, crop, and warning in Japanese."));
    assert!(second.contains("\"free_text\": \"keep the cat ears\""));
}

#[tokio::test]
async fn analysis_reports_rejected_and_broken_answers() {
    let fake = fake(
        Value::Null,
        vec![
            Answer::Chat(
                r#"{"allowed": false, "reason": "adult_explicit", "message": "explicit"}"#
                    .to_owned(),
            ),
            Answer::Chat("I cannot help with that.".to_owned()),
            Answer::Chat(r#"{"allowed": true, "reason": "ok"}"#.to_owned()),
            Answer::Chat("[]".to_owned()),
        ],
    )
    .await;
    let (backend, key) = analysis_backend(&fake);
    let request = analysis_request(&key);
    match analyze(&backend, &request).await {
        Err(AnalysisError::Rejected { reason, message }) => {
            assert_eq!(reason, "reference_adult_explicit");
            assert_eq!(message, "explicit");
        }
        other => panic!("{other:?}"),
    }
    match analyze(&backend, &request).await {
        Err(AnalysisError::Failed(message)) => {
            assert_eq!(
                message.message,
                "Analysis LLM returned an invalid safety result"
            );
            assert_eq!(message.code, Some("analysis_invalid"));
        }
        other => panic!("{other:?}"),
    }
    match analyze(&backend, &request).await {
        Err(AnalysisError::Failed(message)) => {
            assert_eq!(
                message.message,
                "Analysis LLM returned invalid detail analysis JSON"
            );
        }
        other => panic!("{other:?}"),
    }
    let missing = AnalysisRequest {
        reference_keys: vec!["front:references/none/x.png".to_owned()],
        ..analysis_request(&key)
    };
    match analyze(&backend, &missing).await {
        Err(AnalysisError::Failed(message)) => {
            assert!(
                message
                    .message
                    .starts_with("Detail analysis reference image not found")
            );
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn analysis_needs_a_model() {
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            (
                "llm_provider".to_owned(),
                Some("openai_compatible".to_owned()),
            ),
            ("agent_llm_model".to_owned(), Some(String::new())),
        ])
        .unwrap();
    let error = analysis::analysis_backend(&backend.state.settings.settings()).unwrap_err();
    assert_eq!(error.message, "Analysis LLM base URL and model must be set");
    assert_eq!(error.code, Some("setting_missing"));
    assert_eq!(error.params["settings"], "analysis_llm_model");
}

#[tokio::test]
async fn analysis_api_errors_carry_a_code() {
    let fake = fake(
        Value::Null,
        vec![Answer::Json(
            401,
            json!({"error": {"message": "No auth credentials found"}}),
        )],
    )
    .await;
    let (backend, key) = analysis_backend(&fake);
    match analyze(&backend, &analysis_request(&key)).await {
        Err(AnalysisError::Failed(failure)) => {
            assert!(failure.message.starts_with("Analysis LLM HTTP 401"));
            assert_eq!(failure.code, Some("api_unauthorized"));
            assert_eq!(failure.params["service"], "127.0.0.1");
            assert_eq!(failure.params["detail"], "No auth credentials found");
        }
        other => panic!("{other:?}"),
    }
}

// --- Failures of the hosted APIs ----------------------------------------------

#[tokio::test]
async fn image_api_failures_are_sorted_by_status() {
    for (status, code) in [
        (401, "api_unauthorized"),
        (403, "api_unauthorized"),
        (429, "api_rate_limited"),
        (500, "api_server_error"),
        (404, "api_http_error"),
    ] {
        // Retryable statuses are sent again; every try gets the same answer.
        let answers = vec![Answer::Json(status, json!({"error": {"message": "nope"}})); 4];
        let fake = fake(models(), answers).await;
        let (backend, key) = openrouter_backend(&fake);
        let error = generate(&backend, &front_request(&key)).await.unwrap_err();
        assert_eq!(error.code, Some(code), "{status}: {error}");
        assert_eq!(error.params["status"], status);
        assert_eq!(error.params["service"], "OpenRouter");
        assert_eq!(error.params["detail"], "nope");
    }
}

#[tokio::test]
async fn an_image_answer_without_an_image_has_a_code() {
    let fake = fake(models(), vec![Answer::Json(200, json!({"data": []}))]).await;
    let (backend, key) = openrouter_backend(&fake);
    let error = generate(&backend, &front_request(&key)).await.unwrap_err();
    assert_eq!(error.code, Some("api_no_image"));
    assert_eq!(error.params["service"], "OpenRouter");
}

/// One streamed chat request to `base_url`, waiting `seconds` for it.
async fn chat_failure(base_url: &str, seconds: u64) -> crate::backend::failure::Failure {
    let chat = crate::backend::chat_api::ChatApi::with_retry_delays(Vec::new());
    let payload = json!({"model": "m", "messages": []});
    let Value::Object(payload) = payload else {
        unreachable!()
    };
    chat.stream_chat_completion(
        base_url,
        "key",
        &payload,
        &serde_json::Map::new(),
        Duration::from_secs(seconds),
    )
    .await
    .unwrap_err()
    .to_failure(base_url, "Agent LLM ")
}

#[tokio::test]
async fn chat_api_failures_are_sorted_by_status() {
    for (status, code) in [
        (401, "api_unauthorized"),
        (429, "api_rate_limited"),
        (503, "api_server_error"),
        (400, "api_http_error"),
    ] {
        let fake = fake(
            Value::Null,
            vec![Answer::Json(status, json!({"error": {"message": "nope"}}))],
        )
        .await;
        let failure = chat_failure(&fake.base_url, 10).await;
        assert_eq!(failure.code, Some(code), "{status}: {failure}");
        assert!(
            failure
                .message
                .starts_with(&format!("Agent LLM HTTP {status}"))
        );
        assert_eq!(
            Value::Object(failure.params),
            json!({"service": "127.0.0.1", "status": status, "detail": "nope"})
        );
    }
}

#[tokio::test]
async fn a_silent_chat_api_times_out_and_a_closed_port_is_unreachable() {
    let fake = fake(Value::Null, vec![Answer::Hang]).await;
    let failure = chat_failure(&fake.base_url, 1).await;
    assert_eq!(failure.code, Some("api_timeout"), "{failure}");
    assert_eq!(
        Value::Object(failure.params),
        json!({"service": "127.0.0.1", "seconds": 1})
    );

    // Windows takes about two seconds to give up on a closed port.
    let failure = chat_failure("http://127.0.0.1:9/v1", 30).await;
    assert_eq!(failure.code, Some("api_unreachable"), "{failure}");
    assert_eq!(failure.params["service"], "127.0.0.1");
    assert!(failure.params.contains_key("detail"));
}

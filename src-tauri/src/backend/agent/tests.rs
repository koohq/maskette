//! Ported from the old backend's `tests/test_agent.py` and
//! `tests/test_agent_history.py`, plus the live events, project files and
//! what a release build (no fixtures) says.
//!
//! The model is a stand-in OpenAI-compatible server that streams the turns a
//! test's policy picks from the transcript it receives.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::backend::generation::{GenerationRequest, Mode};
use crate::backend::tests::{TestBackend, authed, json_body};

// --- The stand-in model -------------------------------------------------------

enum Reply {
    Turn(&'static str, Vec<(&'static str, Value)>),
    Status(u16, &'static str),
}

fn say(text: &'static str) -> Reply {
    Reply::Turn(text, Vec::new())
}

fn call(text: &'static str, name: &'static str, arguments: Value) -> Reply {
    Reply::Turn(text, vec![(name, arguments)])
}

type Policy = Box<dyn Fn(&[Value]) -> Reply + Send + Sync>;

struct FakeLlm {
    base_url: String,
    policy: Arc<Mutex<Policy>>,
    calls: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl FakeLlm {
    fn set(&self, policy: impl Fn(&[Value]) -> Reply + Send + Sync + 'static) {
        *self.policy.lock().unwrap() = Box::new(policy);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn streamed(text: &str, calls: &[(&str, Value)], counter: usize) -> String {
    let mut body = String::new();
    let mut event = |delta: Value| {
        body.push_str(&format!(
            "data: {}\n\n",
            json!({"choices": [{"delta": delta}]})
        ));
    };
    if !text.is_empty() {
        event(json!({"content": text}));
    }
    for (index, (name, arguments)) in calls.iter().enumerate() {
        event(json!({"tool_calls": [{
            "index": index,
            "id": format!("call-{name}-{counter}-{index}"),
            "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()},
        }]}));
    }
    body.push_str(": done\n\ndata: [DONE]\n\n");
    body
}

async fn fake_llm(policy: impl Fn(&[Value]) -> Reply + Send + Sync + 'static) -> FakeLlm {
    let policy: Arc<Mutex<Policy>> = Arc::new(Mutex::new(Box::new(policy)));
    let calls = Arc::new(AtomicUsize::new(0));
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let (shared, counter, recorded) = (policy.clone(), calls.clone(), bodies.clone());
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |request: Request| {
            let (policy, counter, recorded) = (shared.clone(), counter.clone(), recorded.clone());
            async move {
                let bytes = request.into_body().collect().await.unwrap().to_bytes();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                let count = counter.fetch_add(1, Ordering::SeqCst);
                let messages = body["messages"].as_array().cloned().unwrap_or_default();
                recorded.lock().unwrap().push(body);
                let reply = (policy.lock().unwrap())(&messages);
                match reply {
                    Reply::Turn(text, calls) => (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        streamed(text, &calls, count),
                    )
                        .into_response(),
                    Reply::Status(status, text) => (
                        StatusCode::from_u16(status).unwrap(),
                        axum::Json(json!({"error": {"message": text}})),
                    )
                        .into_response(),
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeLlm {
        base_url,
        policy,
        calls,
        bodies,
    }
}

/// The names of the tools that have answered, in order.
fn tool_names(messages: &[Value]) -> Vec<String> {
    let mut names = Vec::new();
    for message in messages {
        if message["role"] == "tool" {
            let id = message["tool_call_id"].as_str().unwrap_or_default();
            let name = messages
                .iter()
                .flat_map(|other| other["tool_calls"].as_array().cloned().unwrap_or_default())
                .find(|call| call["id"] == id)
                .and_then(|call| call["function"]["name"].as_str().map(str::to_owned))
                .unwrap_or_default();
            names.push(name);
        }
    }
    names
}

fn has(messages: &[Value], tool: &str) -> bool {
    tool_names(messages).iter().any(|name| name == tool)
}

fn last_is_user(messages: &[Value]) -> bool {
    messages
        .last()
        .is_some_and(|message| message["role"] == "user")
}

// --- The backend --------------------------------------------------------------

struct Setup {
    backend: TestBackend,
    llm: FakeLlm,
}

async fn setup(policy: impl Fn(&[Value]) -> Reply + Send + Sync + 'static) -> Setup {
    let llm = fake_llm(policy).await;
    let backend = TestBackend::new(true);
    backend
        .state
        .settings
        .update(&[
            ("agent_llm_base_url".to_owned(), Some(llm.base_url.clone())),
            ("agent_llm_api_key".to_owned(), Some("key".to_owned())),
            ("agent_llm_model".to_owned(), Some("test-model".to_owned())),
        ])
        .unwrap();
    Setup { backend, llm }
}

impl Setup {
    async fn call(&self, request: Request) -> Response {
        self.backend.app().oneshot(request).await.unwrap()
    }

    async fn json(&self, method: Method, uri: &str, body: Value) -> Response {
        let request = authed(method, uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        self.call(request).await
    }

    async fn get(&self, uri: &str) -> Response {
        self.call(authed(Method::GET, uri).body(Body::empty()).unwrap())
            .await
    }

    async fn post(&self, uri: &str) -> Response {
        self.call(authed(Method::POST, uri).body(Body::empty()).unwrap())
            .await
    }

    async fn form(&self, uri: &str, parts: &[Part]) -> Response {
        let request = authed(Method::POST, uri)
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .body(Body::from(multipart(parts)))
            .unwrap();
        self.call(request).await
    }

    async fn start(&self) -> String {
        let response = self
            .json(
                Method::POST,
                "/api/agent/conversations",
                json!({"locale": "zh-CN"}),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await["id"].as_str().unwrap().to_owned()
    }

    async fn conversation(&self, id: &str) -> Value {
        json_body(self.get(&format!("/api/agent/conversations/{id}")).await).await
    }

    /// The conversation once its run has ended.
    async fn settled(&self, id: &str) -> Value {
        for _ in 0..600 {
            let body = self.conversation(id).await;
            if body["running"] == false {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the run did not end");
    }

    async fn send(&self, id: &str, text: &str, files: usize, fields: &[(&str, &str)]) -> Value {
        let mut parts: Vec<Part> = vec![Part::text("text", text)];
        parts.extend(fields.iter().map(|(name, value)| Part::text(name, value)));
        let image = png(64, 80, [255, 192, 203]);
        for index in 0..files {
            parts.push(Part::file(
                "files",
                &format!("ref{index}.png"),
                "image/png",
                image.clone(),
            ));
        }
        let response = self
            .form(&format!("/api/agent/conversations/{id}/messages"), &parts)
            .await;
        let status = response.status();
        assert_eq!(status, StatusCode::OK, "{}", json_body(response).await);
        self.settled(id).await
    }

    fn transcript(&self, id: &str) -> Vec<Value> {
        self.backend
            .state
            .agent
            .store
            .get(id)
            .unwrap()
            .lock()
            .llm_messages
            .clone()
    }

    fn job(&self, body: &Value, image_id: &str) -> GenerationRequest {
        let image = images(body)
            .into_iter()
            .find(|image| image["id"] == image_id)
            .unwrap_or_else(|| panic!("no image {image_id}"));
        let job_id = image["job_id"].as_str().unwrap();
        let log = self.backend.state.generation_log.lock().unwrap();
        log.iter()
            .find(|(id, _)| id == job_id)
            .map(|(_, request)| request.clone())
            .expect("a recorded job")
    }
}

fn events<'a>(body: &'a Value, kind: &str) -> Vec<&'a Value> {
    body["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == kind)
        .collect()
}

fn tools(body: &Value) -> Vec<(String, String)> {
    events(body, "tool")
        .iter()
        .map(|event| {
            (
                event["tool"].as_str().unwrap().to_owned(),
                event["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn tool_event<'a>(body: &'a Value, tool: &str) -> &'a Value {
    events(body, "tool")
        .into_iter()
        .find(|event| event["tool"] == tool)
        .unwrap_or_else(|| panic!("no {tool} event"))
}

fn images(body: &Value) -> Vec<Value> {
    body["state"]["images"].as_array().unwrap().clone()
}

fn replies(body: &Value) -> Vec<String> {
    events(body, "assistant_message")
        .iter()
        .filter(|event| event.get("deleted").is_none())
        .map(|event| event["text"].as_str().unwrap().to_owned())
        .collect()
}

// --- Forms --------------------------------------------------------------------

const BOUNDARY: &str = "maskette-agent-boundary";

struct Part {
    name: String,
    file: Option<(String, String)>,
    bytes: Vec<u8>,
}

impl Part {
    fn text(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            file: None,
            bytes: value.as_bytes().to_vec(),
        }
    }

    fn file(name: &str, file_name: &str, content_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            name: name.to_owned(),
            file: Some((file_name.to_owned(), content_type.to_owned())),
            bytes,
        }
    }
}

fn multipart(parts: &[Part]) -> Vec<u8> {
    let mut body = Vec::new();
    for part in parts {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match &part.file {
            None => body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", part.name).as_bytes(),
            ),
            Some((file_name, content_type)) => body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n",
                    part.name
                )
                .as_bytes(),
            ),
        }
        body.extend_from_slice(&part.bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb(rgb));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A mask the size of `base` with a filled square.
fn square_mask(base: &[u8]) -> Vec<u8> {
    let base = image::load_from_memory(base).unwrap();
    let mask = image::GrayImage::from_fn(base.width(), base.height(), |x, y| {
        image::Luma([if (10..60).contains(&x) && (10..60).contains(&y) {
            255
        } else {
            0
        }])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    mask.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

async fn bytes(response: Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

// --- Policies -----------------------------------------------------------------

/// analyse -> head shell front view (design stage skipped) -> ask the user.
fn auto_front(messages: &[Value]) -> Reply {
    if !has(messages, "analyze_references") {
        call("我先分析一下参考图。", "analyze_references", json!({}))
    } else if !has(messages, "generate_front_view") {
        call(
            "",
            "generate_front_view",
            json!({"instructions": "soft eyes", "skip_design": true}),
        )
    } else {
        say("正视图好了，看看可以吗？")
    }
}

/// analyse -> character design -> ask the user.
fn auto_design(messages: &[Value]) -> Reply {
    if !has(messages, "analyze_references") {
        call("我先分析一下参考图。", "analyze_references", json!({}))
    } else if !has(messages, "generate_design") {
        call("", "generate_design", json!({"instructions": "soft eyes"}))
    } else {
        say("设定图好了，看看可以吗？")
    }
}

fn no_llm(_: &[Value]) -> Reply {
    panic!("this action must not call the model")
}

// --- Tests --------------------------------------------------------------------

#[tokio::test]
async fn upload_runs_analysis_and_design_then_waits() {
    let setup = setup(auto_design).await;
    let id = setup.start().await;
    let body = setup.send(&id, "帮我做这个角色", 2, &[]).await;

    assert_eq!(body["status"], "idle");
    let kinds: Vec<&str> = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap())
        .collect();
    assert_eq!(kinds[..2], ["user_message", "run_state"]);
    assert_eq!(
        tools(&body),
        [
            ("analyze_references".to_owned(), "succeeded".to_owned()),
            ("generate_design".to_owned(), "succeeded".to_owned()),
        ]
    );
    let shown = events(&body, "image");
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0]["role"], "design");
    assert_eq!(kinds[kinds.len() - 2], "assistant_message");
    let kinds_of_refs: Vec<&str> = body["state"]["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds_of_refs, ["front", "supplemental"]);
    assert_eq!(body["state"]["current_design_id"], "design-1");
    assert_eq!(body["state"]["approved_design_id"], Value::Null);
    assert_eq!(body["state"]["current_front_id"], Value::Null);
    assert_eq!(
        body["state"]["analysis"]["features"][0]["id"],
        "feature-hair"
    );
    let job = setup.job(&body, "design-1");
    assert_eq!(job.generation_mode, Mode::CharacterFront);
    assert!(job.reference_keys[0].starts_with("front:"));
    assert!(job.detail_lock.is_some());
    // The design is copied next to the references, so later jobs can attach it.
    let key = images(&body)[0]["reference_key"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = super::tools::reference_path(setup.backend.dir.path(), &key).unwrap();
    assert!(path.is_file());
    // The public image is served.
    let url = images(&body)[0]["url"].as_str().unwrap().to_owned();
    let served = setup.get(&url).await;
    assert_eq!(served.status(), StatusCode::OK);
    assert_eq!(served.headers()[header::CONTENT_TYPE], "image/png");
    let decoded = image::load_from_memory(&bytes(served).await).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (800, 1100));
}

#[tokio::test]
async fn the_model_gets_a_streamed_request_in_the_openai_shape() {
    let setup = setup(|_| say("好的")).await;
    let id = setup.start().await;
    let body = setup.send(&id, "hi", 0, &[]).await;
    assert_eq!(replies(&body), ["好的"]);

    let sent = setup.llm.bodies.lock().unwrap()[0].clone();
    assert_eq!(sent["model"], "test-model");
    assert_eq!(sent["stream"], true);
    assert_eq!(sent["reasoning"], json!({"effort": "low"}));
    assert_eq!(sent["tool_choice"], "auto");
    assert_eq!(sent["messages"][0]["role"], "system");
    let prompt = sent["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("Reply in Simplified Chinese."));
    assert!(prompt.contains("\"stage\": \"design\""));
    assert_eq!(
        sent["messages"][1],
        json!({"role": "user", "content": "hi"})
    );
    let names: Vec<&str> = sent["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"generate_front_view") && names.contains(&"generate_turnaround"));
}

#[tokio::test]
async fn prompt_overrides_reach_the_model_and_the_job_record() {
    let setup = setup(auto_design).await;
    std::fs::write(
        setup
            .backend
            .dir
            .path()
            .join(crate::backend::prompts::FILE_NAME),
        json!({
            "agent.system_prompt": "TEST ASSISTANT in {language}. State: {state}",
            "constraints.character_front": ["Custom design constraint."],
        })
        .to_string(),
    )
    .unwrap();
    let id = setup.start().await;
    let body = setup.send(&id, "make the design", 1, &[]).await;

    let sent = setup.llm.bodies.lock().unwrap()[0].clone();
    let prompt = sent["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.starts_with("TEST ASSISTANT in Simplified Chinese. State: {"));

    let design = images(&body)
        .into_iter()
        .find(|image| image["role"] == "design")
        .expect("a design");
    let record = setup
        .backend
        .state
        .generation
        .jobs
        .get(design["job_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        record.prompt_payload["system_constraints"],
        json!(["Custom design constraint."])
    );
}

#[tokio::test]
async fn tool_results_reach_the_model_without_their_name() {
    let setup = setup(|messages| {
        if last_is_user(messages) {
            call("", "approve_front_view", json!({"image_id": "nope"}))
        } else {
            say("ok")
        }
    })
    .await;
    let id = setup.start().await;
    setup.send(&id, "go", 0, &[]).await;
    let second = setup.llm.bodies.lock().unwrap()[1].clone();
    let tool = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()
        .clone();
    assert!(tool.get("name").is_none());
    assert!(
        tool["content"]
            .as_str()
            .unwrap()
            .contains("Unknown front image id.")
    );
    // The stored transcript keeps the name.
    let stored = setup.transcript(&id);
    assert_eq!(
        stored.iter().find(|m| m["role"] == "tool").unwrap()["name"],
        "approve_front_view"
    );
}

#[tokio::test]
async fn head_shell_requires_an_approved_design() {
    let setup = setup(|messages| {
        let names = tool_names(messages);
        if names.is_empty() {
            call("", "generate_design", json!({}))
        } else if !has(messages, "approve_design") {
            // Approving its own design in the same run must be refused.
            call("", "approve_design", json!({"image_id": "design-1"}))
        } else if !has(messages, "generate_front_view") {
            call("", "generate_front_view", json!({}))
        } else {
            say("等你确认。")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;

    assert_eq!(tool_event(&body, "approve_design")["status"], "failed");
    let front = tool_event(&body, "generate_front_view");
    assert_eq!(front["status"], "failed");
    assert!(
        front["error"]
            .as_str()
            .unwrap()
            .contains("approved character design")
    );
    let roles: Vec<Value> = images(&body)
        .iter()
        .map(|image| image["role"].clone())
        .collect();
    assert_eq!(roles, [json!("design")]);
}

#[tokio::test]
async fn approved_design_drives_head_shell_and_turnaround() {
    let setup = setup(|messages| {
        if last_is_user(messages) {
            call("", "generate_design", json!({"view": "turnaround"}))
        } else {
            say("四视图设定好了。")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;
    assert_eq!(
        setup.job(&body, "design_turnaround-1").generation_mode,
        Mode::CharacterTurnaround
    );

    setup.llm.set(|messages| {
        if last_is_user(messages) {
            call("开始做头壳。", "generate_front_view", json!({}))
        } else {
            say("头壳正视图好了。")
        }
    });
    let body = setup
        .send(
            &id,
            "",
            0,
            &[
                ("action", "approve_design"),
                ("image_id", "design_turnaround-1"),
            ],
        )
        .await;
    assert_eq!(body["state"]["approved_design_id"], "design_turnaround-1");
    let front = images(&body).last().unwrap().clone();
    assert_eq!(front["role"], "front");
    assert_eq!(front["parent_id"], "design_turnaround-1");
    let job = setup.job(&body, front["id"].as_str().unwrap());
    assert_eq!(job.generation_mode, Mode::FrontDesign);
    // Only the approved design goes in: raw references would undo its edits.
    assert_eq!(job.reference_keys.len(), 1);
    assert!(job.reference_keys[0].starts_with("design:references/agent/"));
    assert!(
        job.reference_descriptions[0]
            .description
            .contains("approved 2D character design")
    );
    let user = events(&body, "user_message")
        .into_iter()
        .last()
        .unwrap()
        .clone();
    assert_eq!(user["action"], "approve_design");
    assert_eq!(events(&body, "design_approved").len(), 1);

    setup.llm.set(|messages| {
        if last_is_user(messages) {
            call("", "generate_turnaround", json!({}))
        } else {
            say("完成。")
        }
    });
    let front_id = front["id"].as_str().unwrap().to_owned();
    let body = setup
        .send(
            &id,
            "",
            0,
            &[("action", "approve_front"), ("image_id", &front_id)],
        )
        .await;
    let keys = setup.job(&body, "turnaround-1").reference_keys;
    assert!(keys[0].starts_with("front:") && keys[1].starts_with("design_extra:"));
    let sheet = images(&body).last().unwrap().clone();
    assert_eq!(
        (sheet["width"].as_u64(), sheet["height"].as_u64()),
        (Some(3000), Some(2000))
    );
}

#[tokio::test]
async fn revise_design_and_expand_a_front_design_to_four_views() {
    let setup = setup(auto_design).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;

    setup.llm.set(|messages| {
        let names = tool_names(messages);
        if !has(messages, "revise_design") {
            call(
                "",
                "revise_design",
                json!({"instructions": "shorter bangs"}),
            )
        } else if names
            .iter()
            .filter(|name| *name == "generate_design")
            .count()
            < 2
        {
            call(
                "",
                "generate_design",
                json!({"view": "turnaround", "base_image_id": "design-2"}),
            )
        } else {
            say("好了。")
        }
    });
    let body = setup.send(&id, "刘海短一点，然后出四视图", 0, &[]).await;
    assert_eq!(
        setup.job(&body, "design-2").generation_mode,
        Mode::CharacterRevision
    );
    let expanded = setup.job(&body, "design_turnaround-1");
    assert!(expanded.reference_keys[0].starts_with("design:"));
    assert!(
        expanded.reference_keys[1..]
            .iter()
            .all(|key| key.starts_with("supplemental:"))
    );
    assert_eq!(body["state"]["current_design_id"], "design_turnaround-1");
}

#[tokio::test]
async fn turnaround_requires_user_approval() {
    let setup = setup(|messages| {
        if tool_names(messages).is_empty() {
            call("", "generate_front_view", json!({"skip_design": true}))
        } else if !has(messages, "approve_front_view") {
            call("", "approve_front_view", json!({"image_id": "front-1"}))
        } else if !has(messages, "generate_turnaround") {
            call("", "generate_turnaround", json!({}))
        } else {
            say("等你确认。")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;
    assert_eq!(tool_event(&body, "approve_front_view")["status"], "failed");
    let turnaround = tool_event(&body, "generate_turnaround");
    assert_eq!(turnaround["status"], "failed");
    assert!(turnaround["error"].as_str().unwrap().contains("approved"));
    assert_eq!(body["state"]["approved_front_id"], Value::Null);
    assert!(
        images(&body)
            .iter()
            .all(|image| image["role"] != "turnaround")
    );
}

#[tokio::test]
async fn approve_button_then_turnaround() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    setup.llm.set(|messages| {
        if last_is_user(messages) {
            call("好的，开始四视图。", "generate_turnaround", json!({}))
        } else {
            say("四视图完成。")
        }
    });
    let body = setup
        .send(
            &id,
            "",
            0,
            &[("action", "approve_front"), ("image_id", "front-1")],
        )
        .await;
    assert_eq!(body["state"]["approved_front_id"], "front-1");
    let turnarounds: Vec<Value> = images(&body)
        .into_iter()
        .filter(|image| image["role"] == "turnaround")
        .collect();
    assert_eq!(turnarounds.len(), 1);
    assert_eq!(turnarounds[0]["parent_id"], "front-1");
}

#[tokio::test]
async fn text_approval_after_a_user_reply_is_allowed() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    setup.llm.set(|messages| {
        if last_is_user(messages) {
            call("", "approve_front_view", json!({"image_id": "front-1"}))
        } else {
            say("已确认。")
        }
    });
    let body = setup.send(&id, "可以，就这张", 0, &[]).await;
    assert_eq!(body["state"]["approved_front_id"], "front-1");
    assert_eq!(events(&body, "front_approved").len(), 1);
}

#[tokio::test]
async fn generations_are_capped_per_turn() {
    let setup = setup(|messages| {
        if tool_names(messages).len() < 5 {
            call("", "generate_front_view", json!({"skip_design": true}))
        } else {
            say("done")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;
    let fronts = images(&body)
        .into_iter()
        .filter(|image| image["role"] == "front")
        .count();
    assert_eq!(fronts, super::runner::MAX_GENERATIONS_PER_TURN);
    let limited: Vec<Value> = setup
        .transcript(&id)
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| serde_json::from_str(message["content"].as_str().unwrap()).unwrap())
        .skip(super::runner::MAX_GENERATIONS_PER_TURN)
        .collect();
    assert_eq!(limited.len(), 2);
    assert!(
        limited
            .iter()
            .all(|result: &Value| result["error"].as_str().unwrap().contains("limit"))
    );
}

#[tokio::test]
async fn messages_must_say_something_and_wait_for_the_run() {
    let setup = setup(|_| say("hi")).await;
    let id = setup.start().await;
    let uri = format!("/api/agent/conversations/{id}/messages");
    let response = setup.form(&uri, &[Part::text("text", "  ")]).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["detail"], "empty_message");
    let response = setup
        .form(
            &uri,
            &[
                Part::text("action", "approve_front"),
                Part::text("image_id", "front-9"),
            ],
        )
        .await;
    assert_eq!(json_body(response).await["detail"], "unknown_image");
    let response = setup.form(&uri, &[Part::text("action", "launch")]).await;
    assert_eq!(json_body(response).await["detail"], "unknown_action");
    let response = setup
        .form(
            &uri,
            &[Part::file(
                "files",
                "a.gif",
                "image/gif",
                b"GIF89a".to_vec(),
            )],
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["detail"],
        "unsupported_image_type"
    );

    // A slow model keeps the run going while a second message arrives.
    setup.llm.set(|_| {
        std::thread::sleep(Duration::from_millis(400));
        say("slow")
    });
    assert_eq!(
        setup
            .form(&uri, &[Part::text("text", "first")])
            .await
            .status(),
        StatusCode::OK
    );
    let busy = setup.form(&uri, &[Part::text("text", "second")]).await;
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(busy).await["detail"], "conversation_running");
    setup.settled(&id).await;
}

#[tokio::test]
async fn model_errors_are_reported_in_the_chat() {
    let setup = setup(|_| Reply::Status(401, "bad key")).await;
    let id = setup.start().await;
    let body = setup.send(&id, "hello", 0, &[]).await;
    assert_eq!(body["status"], "error");
    let errors = events(&body, "error");
    assert!(errors[0]["message"].as_str().unwrap().contains("401"));
    // The screen says it in the user's language, with the API's own words.
    assert_eq!(errors[0]["code"], "api_unauthorized");
    assert_eq!(
        errors[0]["params"],
        json!({"service": "127.0.0.1", "status": 401, "detail": "bad key"})
    );
    assert_eq!(
        events(&body, "run_state").last().unwrap()["status"],
        "error"
    );
}

#[tokio::test]
async fn the_claude_code_assistant_reports_a_missing_cli() {
    let setup = setup(no_llm).await;
    setup
        .backend
        .state
        .settings
        .update(&[(
            "agent_llm_provider".to_owned(),
            Some("claude_code".to_owned()),
        )])
        .unwrap();
    let id = setup.start().await;
    let body = setup.send(&id, "hello", 0, &[]).await;
    // The screen says it in the user's language from the code.
    let error = &events(&body, "error")[0];
    assert_eq!(error["code"], "cli_not_found");
    assert_eq!(
        error["params"],
        json!({"name": "Claude Code", "program": "claude"})
    );
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("The Claude Code CLI (claude) was not found"),
        "{error}"
    );
}

#[tokio::test]
async fn cli_tool_failures_carry_a_code_for_the_screen() {
    let llm = fake_llm(|messages| {
        if last_is_user(messages) {
            Reply::Turn("", vec![("analyze_references", json!({}))])
        } else if !has(messages, "generate_front_view") {
            call("", "generate_front_view", json!({"skip_design": true}))
        } else {
            say("done")
        }
    })
    .await;
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            ("agent_llm_base_url".to_owned(), Some(llm.base_url.clone())),
            ("agent_llm_model".to_owned(), Some("test-model".to_owned())),
            ("llm_provider".to_owned(), Some("codex".to_owned())),
            ("image_provider".to_owned(), Some("codex".to_owned())),
        ])
        .unwrap();
    let setup = Setup { backend, llm };
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;
    for tool in ["analyze_references", "generate_front_view"] {
        let event = tool_event(&body, tool);
        assert_eq!(event["status"], "failed", "{event}");
        assert_eq!(event["error_code"], "cli_not_found", "{event}");
        assert_eq!(
            event["error_params"],
            json!({"name": "Codex", "program": "codex"})
        );
        assert!(event["error"].as_str().unwrap().contains("was not found"));
    }
}

#[tokio::test]
async fn conversations_are_listed_renamed_and_deleted() {
    let setup = setup(|_| say("hi")).await;
    let id = setup.start().await;
    setup.send(&id, "第一条消息", 1, &[]).await;
    let listed = json_body(setup.get("/api/agent/conversations").await).await;
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["title"], "第一条消息");
    assert!(
        listed[0]["thumbnail_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/references/references/agent/")
    );

    let renamed = setup
        .json(
            Method::PATCH,
            &format!("/api/agent/conversations/{id}"),
            json!({"title": "新名字"}),
        )
        .await;
    assert_eq!(json_body(renamed).await["title"], "新名字");
    let empty = setup
        .json(
            Method::PATCH,
            &format!("/api/agent/conversations/{id}"),
            json!({"title": ""}),
        )
        .await;
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let uploads = super::tools::conversation_references(setup.backend.dir.path(), &id);
    assert!(uploads.is_dir());
    let deleted = setup
        .call(
            authed(Method::DELETE, &format!("/api/agent/conversations/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(json_body(deleted).await, json!({"deleted": true}));
    assert_eq!(
        setup
            .get(&format!("/api/agent/conversations/{id}"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(!uploads.exists());
}

#[tokio::test]
async fn conversations_survive_a_restart() {
    let setup = setup(|_| say("hi")).await;
    let id = setup.start().await;
    setup.send(&id, "记住我", 0, &[]).await;
    let restarted = setup.backend.restart();
    let entry = restarted.agent.store.get(&id).unwrap();
    let conversation = entry.lock();
    assert_eq!(conversation.title, "记住我");
    assert_eq!(conversation.llm_messages.len(), 2);
    assert_eq!(conversation.status, "idle");
}

#[tokio::test]
async fn a_failed_tool_skips_the_rest_of_its_batch() {
    let setup = setup(|messages| {
        if last_is_user(messages) {
            Reply::Turn(
                "",
                vec![
                    ("analyze_references", json!({})),
                    ("generate_front_view", json!({})),
                ],
            )
        } else {
            say("先传一张图吧。")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "没有图", 0, &[]).await;
    assert_eq!(
        tools(&body),
        [("analyze_references".to_owned(), "failed".to_owned())]
    );
    assert!(images(&body).is_empty());
    let results: Vec<Value> = setup
        .transcript(&id)
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| serde_json::from_str(message["content"].as_str().unwrap()).unwrap())
        .collect();
    assert!(results[1]["error"].as_str().unwrap().contains("Skipped"));
}

#[tokio::test]
async fn a_manual_version_keeps_its_clean_source_and_recipe() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("note", "eyes bigger"),
                Part::text("recipe", r#"{"eyes": {"eyeSize": 0.2}}"#),
                Part::file(
                    "image",
                    "edit.png",
                    "image/png",
                    png(80, 110, [10, 200, 30]),
                ),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let manual = images(&body).last().unwrap().clone();
    assert_eq!(manual["id"], "front-2");
    assert_eq!(manual["source"], "manual");
    assert_eq!(manual["parent_id"], "front-1");
    assert_eq!(manual["recipe"], json!({"eyes": {"eyeSize": 0.2}}));
    assert_eq!(body["state"]["current_front_id"], "front-2");

    let source = setup
        .get(&format!(
            "/api/agent/conversations/{id}/versions/front-2/source"
        ))
        .await;
    assert_eq!(source.status(), StatusCode::OK);
    assert_eq!(
        source.headers()[header::CACHE_CONTROL],
        "private, max-age=3600"
    );
    let clean = image::load_from_memory(&bytes(source).await)
        .unwrap()
        .to_rgb8();
    assert_eq!(clean.get_pixel(5, 105).0, [10, 200, 30]);
    let public = setup.get(manual["url"].as_str().unwrap()).await;
    assert_eq!(public.status(), StatusCode::OK);

    let download = setup
        .get(&format!(
            "/api/agent/conversations/{id}/versions/front-2/download"
        ))
        .await;
    assert_eq!(
        download.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"front-2.png\""
    );
    assert!(image::load_from_memory(&bytes(download).await).is_ok());
    assert!(
        setup.transcript(&id).last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("front-2")
    );

    let broken = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("recipe", "[1]"),
                Part::file("image", "edit.png", "image/png", png(8, 8, [0, 0, 0])),
            ],
        )
        .await;
    assert_eq!(json_body(broken).await["detail"], "recipe_invalid");
    let not_an_image = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::file("image", "edit.png", "image/png", b"nope".to_vec()),
            ],
        )
        .await;
    assert_eq!(json_body(not_an_image).await["detail"], "image_invalid");
}

#[tokio::test]
async fn an_eye_correction_is_its_own_version_and_the_assistant_hears_of_it() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("note", "eyes levelled"),
                Part::text("recipe", "{}"),
                Part::text("source", "auto_symmetry"),
                Part::file(
                    "image",
                    "corrected.png",
                    "image/png",
                    png(80, 110, [5, 5, 200]),
                ),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let corrected = images(&body).last().unwrap().clone();
    assert_eq!(corrected["source"], "auto_symmetry");
    assert_eq!(corrected["parent_id"], "front-1");
    // The corrected pixels are what the next stage builds on.
    assert_eq!(body["state"]["current_front_id"], corrected["id"]);
    let said = setup.transcript(&id).last().unwrap()["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(said.starts_with("[app action]"), "{said}");
    assert!(said.contains("mirror-symmetric"), "{said}");

    // The screen may only claim its own two sources.
    let claimed = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("source", "agent"),
                Part::file("image", "edit.png", "image/png", png(8, 8, [0, 0, 0])),
            ],
        )
        .await;
    assert_eq!(claimed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(claimed).await["detail"], "unknown_version_source");
}

#[tokio::test]
async fn references_can_be_added_without_chatting() {
    let setup = setup(no_llm).await;
    let response = setup
        .json(
            Method::POST,
            "/api/agent/conversations",
            json!({"title": "猫娘头壳", "locale": "xx"}),
        )
        .await;
    let created = json_body(response).await;
    assert_eq!(created["title"], "猫娘头壳");
    assert_eq!(created["locale"], "zh-CN");
    let id = created["id"].as_str().unwrap().to_owned();
    let image = png(64, 80, [1, 2, 3]);
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/references"),
            &[
                Part::file("files", "a.png", "image/png", image.clone()),
                Part::file("files", "b.png", "image/png", image.clone()),
            ],
        )
        .await;
    let body = json_body(response).await;
    let kinds: Vec<&str> = body["state"]["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["front", "supplemental"]);
    assert_eq!(events(&body, "references_added").len(), 1);
    let url = body["state"]["references"][0]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(bytes(setup.get(&url).await).await, image);
    assert!(
        setup.transcript(&id)[0]["content"]
            .as_str()
            .unwrap()
            .ends_with("(added from the explorer)")
    );
    assert_eq!(setup.llm.calls(), 0);
}

#[tokio::test]
async fn references_can_be_tagged_with_what_they_show() {
    let setup = setup(no_llm).await;
    let id = setup.start().await;
    let image = png(64, 80, [1, 2, 3]);
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/references"),
            &[
                Part::file("files", "a.png", "image/png", image.clone()),
                Part::file("files", "b.png", "image/png", image.clone()),
                Part::file("files", "c.png", "image/png", image),
            ],
        )
        .await;
    let body = json_body(response).await;
    assert_eq!(body["state"]["missing_views"], json!(["side", "back"]));
    let ids: Vec<String> = body["state"]["references"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect();
    let tag = async |reference: &str, kind: &str| {
        setup
            .json(
                Method::PATCH,
                &format!("/api/agent/conversations/{id}/references/{reference}"),
                json!({ "kind": kind }),
            )
            .await
    };
    let kinds = |body: &Value| -> Vec<String> {
        body["state"]["references"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["kind"].as_str().unwrap().to_owned())
            .collect()
    };

    let body = json_body(tag(&ids[1], "back").await).await;
    assert_eq!(kinds(&body), ["front", "back", "supplemental"]);
    assert_eq!(body["state"]["missing_views"], json!(["side"]));

    // A new front takes the place of the old one.
    let body = json_body(tag(&ids[2], "front").await).await;
    assert_eq!(kinds(&body), ["supplemental", "back", "front"]);

    // The front keeps its tag until another image takes it.
    let response = tag(&ids[2], "side").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["detail"],
        "front_reference_required"
    );
    let response = tag(&ids[0], "outfit").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await["detail"],
        "invalid_reference_kind"
    );
    let response = tag("ref-9-000000", "side").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = json_body(tag(&ids[0], "side").await).await;
    assert_eq!(kinds(&body), ["side", "back", "front"]);
    assert_eq!(body["state"]["missing_views"], json!([]));
    assert_eq!(
        kinds(&setup.conversation(&id).await),
        ["side", "back", "front"]
    );
    assert_eq!(setup.llm.calls(), 0);
}

#[tokio::test]
async fn local_revision_runs_as_a_direct_action() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    let base = bytes(
        setup
            .get(&format!(
                "/api/agent/conversations/{id}/versions/front-1/source"
            ))
            .await,
    )
    .await;
    setup.llm.set(no_llm);
    let calls = setup.llm.calls();
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/local-revision"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("edit_note", "把发夹去掉"),
                Part::text("lock_outside", "true"),
                Part::file("mask", "mask.png", "image/png", square_mask(&base)),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = setup.settled(&id).await;
    assert_eq!(
        tools(&body).last().unwrap(),
        &("local_revision".to_owned(), "succeeded".to_owned())
    );
    let latest = images(&body).last().unwrap().clone();
    assert_eq!(latest["source"], "local_revision");
    assert_eq!(latest["parent_id"], "front-1");
    let job = setup.job(&body, latest["id"].as_str().unwrap());
    assert_eq!(job.generation_mode, Mode::FrontLocalRevision);
    let edit = job.local_edit.unwrap();
    assert_eq!(edit.feather_radius_px, 3);
    assert_eq!(edit.subject, "head_shell");
    assert!(edit.mask_image_path.is_file());
    assert!(
        setup.transcript(&id).last().unwrap()["content"]
            .as_str()
            .unwrap()
            .starts_with("[editor action]")
    );
    assert_eq!(setup.llm.calls(), calls);
}

#[tokio::test]
async fn an_empty_mask_fails_the_local_revision() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    let mask = {
        let mask = image::GrayImage::new(800, 1100);
        let mut bytes = std::io::Cursor::new(Vec::new());
        mask.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    };
    setup
        .form(
            &format!("/api/agent/conversations/{id}/local-revision"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("edit_note", "x"),
                Part::file("mask", "mask.png", "image/png", mask),
            ],
        )
        .await;
    let body = setup.settled(&id).await;
    let failed = tool_event(&body, "local_revision");
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("local_revision_mask_empty")
    );
}

#[tokio::test]
async fn annotated_revision_uses_the_annotated_export_as_the_front() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    setup.llm.set(no_llm);
    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/annotated-revision"),
            &[
                Part::text("base_image_id", "front-1"),
                Part::text("instructions", "标注 1: 刘海剪短"),
                Part::file(
                    "image",
                    "annotated.png",
                    "image/png",
                    png(80, 110, [255, 0, 0]),
                ),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = setup.settled(&id).await;
    let latest = images(&body).last().unwrap().clone();
    assert_eq!(latest["id"], "front-2");
    assert_eq!(latest["parent_id"], "front-1");
    let job = setup.job(&body, "front-2");
    assert_eq!(job.generation_mode, Mode::FrontRevision);
    assert!(job.reference_keys[0].starts_with("front:references/agent/"));
}

#[tokio::test]
async fn design_versions_take_manual_annotated_and_local_edits() {
    let setup = setup(auto_design).await;
    let id = setup.start().await;
    setup.send(&id, "", 1, &[]).await;
    setup.llm.set(no_llm);
    let edited = png(80, 110, [10, 200, 30]);
    let manual = setup
        .form(
            &format!("/api/agent/conversations/{id}/versions"),
            &[
                Part::text("base_image_id", "design-1"),
                Part::text("note", "eyes bigger"),
                Part::file("image", "edit.png", "image/png", edited.clone()),
            ],
        )
        .await;
    let manual = json_body(manual).await;
    assert_eq!(images(&manual).last().unwrap()["id"], "design-2");
    assert_eq!(manual["state"]["current_design_id"], "design-2");

    let response = setup
        .form(
            &format!("/api/agent/conversations/{id}/annotated-revision"),
            &[
                Part::text("base_image_id", "design-2"),
                Part::text("instructions", "标注 1: 刘海剪短"),
                Part::file("image", "annotated.png", "image/png", edited),
            ],
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = setup.settled(&id).await;
    assert_eq!(images(&body).last().unwrap()["id"], "design-3");
    assert_eq!(
        setup.job(&body, "design-3").generation_mode,
        Mode::CharacterRevision
    );

    let base = bytes(
        setup
            .get(&format!(
                "/api/agent/conversations/{id}/versions/design-3/source"
            ))
            .await,
    )
    .await;
    setup
        .form(
            &format!("/api/agent/conversations/{id}/local-revision"),
            &[
                Part::text("base_image_id", "design-3"),
                Part::text("edit_note", "去掉发夹"),
                Part::file("mask", "mask.png", "image/png", square_mask(&base)),
            ],
        )
        .await;
    let body = setup.settled(&id).await;
    let latest = images(&body).last().unwrap().clone();
    assert_eq!(latest["id"], "design-4");
    assert_eq!(latest["source"], "local_revision");
    let edit = setup.job(&body, "design-4").local_edit.unwrap();
    assert_eq!(edit.subject, "design");
    assert_eq!(edit.feather_radius_px, 16);
}

#[tokio::test]
async fn deleting_a_reply_hides_it_and_drops_it_from_the_context() {
    let answers = Arc::new(Mutex::new(vec!["第二条回复", "第一条回复"]));
    let setup = setup(move |_| say(answers.lock().unwrap().pop().unwrap())).await;
    let id = setup.start().await;
    setup.send(&id, "你好", 0, &[]).await;
    let body = setup.send(&id, "再说一句", 0, &[]).await;
    let first = events(&body, "assistant_message")[0]["seq"]
        .as_u64()
        .unwrap();

    let uri = format!("/api/agent/conversations/{id}/messages/{first}");
    let response = setup
        .call(authed(Method::DELETE, &uri).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(replies(&body), ["第二条回复"]);
    // The event stays, marked deleted, so sequence numbers are never reused.
    assert!(
        events(&body, "assistant_message")
            .iter()
            .any(|event| event["seq"] == first && event["deleted"] == true)
    );
    let texts: Vec<Value> = setup
        .transcript(&id)
        .iter()
        .filter(|m| m["role"] == "assistant")
        .map(|m| m["content"].clone())
        .collect();
    assert_eq!(texts, [json!("第二条回复")]);

    let again = setup
        .call(authed(Method::DELETE, &uri).body(Body::empty()).unwrap())
        .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_a_reply_with_tool_calls_keeps_the_call() {
    let setup = setup(|messages| {
        if last_is_user(messages) {
            call("我先看看状态。", "no_such_tool", json!({}))
        } else {
            say("好了。")
        }
    })
    .await;
    let id = setup.start().await;
    let body = setup.send(&id, "开始", 0, &[]).await;
    let seq = events(&body, "assistant_message")[0]["seq"]
        .as_u64()
        .unwrap();
    let uri = format!("/api/agent/conversations/{id}/messages/{seq}");
    let response = setup
        .call(authed(Method::DELETE, &uri).body(Body::empty()).unwrap())
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let messages = setup.transcript(&id);
    let with_calls: Vec<&Value> = messages
        .iter()
        .filter(|m| m["role"] == "assistant" && m.get("tool_calls").is_some())
        .collect();
    assert_eq!(with_calls.len(), 1);
    assert_eq!(with_calls[0]["content"], "");
    assert!(messages.iter().any(|m| m["role"] == "tool"));
}

#[tokio::test]
async fn regenerate_runs_the_last_turn_again() {
    let answers = Arc::new(Mutex::new(vec!["新回复", "旧回复"]));
    let setup = setup(move |_| say(answers.lock().unwrap().pop().unwrap())).await;
    let id = setup.start().await;
    setup.send(&id, "出一版", 0, &[]).await;
    let response = setup
        .post(&format!("/api/agent/conversations/{id}/regenerate"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["running"], true);
    let body = setup.settled(&id).await;
    assert_eq!(replies(&body), ["新回复"]);
    assert_eq!(setup.llm.calls(), 2);
    let roles: Vec<Value> = setup
        .transcript(&id)
        .iter()
        .map(|m| m["role"].clone())
        .collect();
    assert_eq!(roles, [json!("user"), json!("assistant")]);
    let seqs: Vec<u64> = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seqs, sorted);
}

#[tokio::test]
async fn regenerate_needs_a_user_turn() {
    let setup = setup(no_llm).await;
    let id = setup.start().await;
    let response = setup
        .post(&format!("/api/agent/conversations/{id}/regenerate"))
        .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(json_body(response).await["detail"], "nothing_to_regenerate");
}

#[tokio::test]
async fn events_stream_live_after_the_cursor() {
    let setup = setup(|_| say("你好呀")).await;
    let id = setup.start().await;
    setup.send(&id, "早上好", 0, &[]).await;
    let first = setup.conversation(&id).await["events"][0]["seq"]
        .as_u64()
        .unwrap();

    let response = setup
        .get(&format!(
            "/api/agent/conversations/{id}/events?after={first}&maskette_token=x"
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut body = response.into_body();
    let mut text = String::new();
    while !text.contains("你好呀") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("an event in time")
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(text.contains("event: agent"));
    assert!(text.contains(&format!("id: {}", first + 1)));
    assert!(
        !text.contains("早上好"),
        "events up to the cursor are not sent again"
    );

    // A message sent now shows up on the open stream.
    setup.llm.set(|_| say("第二句"));
    setup.send(&id, "再来", 0, &[]).await;
    while !text.contains("第二句") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("a live event in time")
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            text.push_str(&String::from_utf8_lossy(&data));
        }
    }
    assert!(text.contains("event_patch") || text.contains("run_state"));
}

#[tokio::test]
async fn a_project_file_round_trips_as_a_copy() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "导出这个", 1, &[]).await;
    let response = setup
        .get(&format!("/api/agent/conversations/{id}/export"))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    let disposition = response.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        disposition.ends_with(&format!("-{}.maskette\"", &id[..8])),
        "{disposition}"
    );
    let payload = bytes(response).await;

    let imported = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "copy.maskette",
                "application/octet-stream",
                payload.clone(),
            )],
        )
        .await;
    assert_eq!(imported.status(), StatusCode::OK);
    let copy = json_body(imported).await;
    let copy_id = copy["id"].as_str().unwrap().to_owned();
    assert_ne!(copy_id, id);
    assert_eq!(copy["title"], "导出这个");
    assert_eq!(copy["status"], "idle");
    let front = images(&copy)[0].clone();
    assert!(front["url"].as_str().unwrap().contains(&copy_id));
    assert!(front["reference_key"].as_str().unwrap().contains(&copy_id));
    // Every file came along under the new id.
    assert_eq!(
        setup.get(front["url"].as_str().unwrap()).await.status(),
        StatusCode::OK
    );
    let source = setup
        .get(&format!(
            "/api/agent/conversations/{copy_id}/versions/front-1/source"
        ))
        .await;
    assert_eq!(source.status(), StatusCode::OK);
    let reference = copy["state"]["references"][0]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(setup.get(&reference).await.status(), StatusCode::OK);

    let wrong_name = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "copy.zip",
                "application/zip",
                payload.clone(),
            )],
        )
        .await;
    assert_eq!(json_body(wrong_name).await["detail"], "not_a_project_file");
    let not_zip = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "x.maskette",
                "application/octet-stream",
                b"nope".to_vec(),
            )],
        )
        .await;
    assert_eq!(json_body(not_zip).await["detail"], "not_a_zip");
    // A `.maskette` renamed to `.vkp` carries the wrong marker for its name.
    let renamed = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "copy.vkp",
                "application/octet-stream",
                payload,
            )],
        )
        .await;
    assert_eq!(json_body(renamed).await["detail"], "unknown_format");
}

/// The bundle with its marker swapped for Visual Studio Kigurumi's, as its
/// old backend wrote a `.vkp`.
fn as_vkp(maskette: &[u8]) -> Vec<u8> {
    use std::io::{Cursor, Read, Write};
    let mut archive = zip::ZipArchive::new(Cursor::new(maskette)).unwrap();
    let mut bundle = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let mut name = file.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        if name == "maskette-project.json" {
            let mut manifest: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(manifest["format"], "maskette-project");
            manifest["format"] = json!("vsk-project");
            bytes = manifest.to_string().into_bytes();
            name = "vsk-project.json".to_owned();
        }
        bundle
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        bundle.write_all(&bytes).unwrap();
    }
    bundle.finish().unwrap().into_inner()
}

#[tokio::test]
async fn a_visual_studio_kigurumi_project_file_still_imports() {
    let setup = setup(auto_front).await;
    let id = setup.start().await;
    setup.send(&id, "旧工程", 1, &[]).await;
    let payload = bytes(
        setup
            .get(&format!("/api/agent/conversations/{id}/export"))
            .await,
    )
    .await;
    let vkp = as_vkp(&payload);

    let imported = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "old.VKP",
                "application/octet-stream",
                vkp.clone(),
            )],
        )
        .await;
    assert_eq!(imported.status(), StatusCode::OK);
    let copy = json_body(imported).await;
    let copy_id = copy["id"].as_str().unwrap().to_owned();
    assert_ne!(copy_id, id);
    assert_eq!(copy["title"], "旧工程");
    let front = images(&copy)[0].clone();
    assert!(front["url"].as_str().unwrap().contains(&copy_id));
    assert_eq!(
        setup.get(front["url"].as_str().unwrap()).await.status(),
        StatusCode::OK
    );

    // A `.vkp` renamed to `.maskette` is not taken as the new format.
    let renamed = setup
        .form(
            "/api/agent/conversations/import",
            &[Part::file(
                "file",
                "old.maskette",
                "application/octet-stream",
                vkp,
            )],
        )
        .await;
    assert_eq!(json_body(renamed).await["detail"], "unknown_format");
}

#[tokio::test]
async fn without_fixtures_the_tools_say_what_is_missing() {
    let llm = fake_llm(|messages| {
        if last_is_user(messages) {
            Reply::Turn("", vec![("analyze_references", json!({}))])
        } else if !has(messages, "generate_front_view") {
            call("", "generate_front_view", json!({"skip_design": true}))
        } else {
            say("配置一下吧。")
        }
    })
    .await;
    let backend = TestBackend::new(false);
    backend
        .state
        .settings
        .update(&[
            ("agent_llm_base_url".to_owned(), Some(llm.base_url.clone())),
            ("agent_llm_model".to_owned(), Some("test-model".to_owned())),
            // Nothing answers here.
            (
                "analysis_llm_base_url".to_owned(),
                Some("http://127.0.0.1:9/v1".to_owned()),
            ),
        ])
        .unwrap();
    let setup = Setup { backend, llm };
    let id = setup.start().await;
    let body = setup.send(&id, "", 1, &[]).await;
    // A release build analyses and generates with OpenRouter by default.
    let analysis = tool_event(&body, "analyze_references");
    assert!(
        analysis["error"]
            .as_str()
            .unwrap()
            .starts_with("Analysis LLM")
    );
    assert_eq!(analysis["error_code"], "api_unreachable");
    assert_eq!(analysis["error_params"]["service"], "127.0.0.1");
    // The batch stopped, so the model asked again and got the image error.
    let generation = tool_event(&body, "generate_front_view");
    assert!(
        generation["error"]
            .as_str()
            .unwrap()
            .contains("openrouter_api_key is required")
    );
    assert_eq!(generation["error_code"], "api_key_missing");
    assert_eq!(
        generation["error_params"],
        json!({"settings": "openrouter_api_key"})
    );
}

#[tokio::test]
async fn generated_assets_stay_inside_their_folder() {
    let setup = setup(no_llm).await;
    for uri in [
        "/api/generated/../settings.json",
        "/api/generated/a.txt",
        "/api/generated/none.png",
    ] {
        assert_eq!(
            setup.get(uri).await.status(),
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
}

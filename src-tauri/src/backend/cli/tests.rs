//! The CLI paths without the real CLIs: a stand-in [`Runner`] answers with
//! canned output (and writes what the CLI would write), and small scripts
//! made during the test check the real process runner.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::{Rgb, RgbImage};
use serde_json::{Value, json};

use super::{Cli, Invocation, Output, ProcessRunner, RunError, RunFuture, Runner, claude, codex};
use crate::backend::agent::llm::{self, ToolSpec};
use crate::backend::failure::Failure;
use crate::backend::generation::analysis::{self, AnalysisError, AnalysisRequest};
use crate::backend::generation::{
    GenerationRequest, LocalEdit, Mode, generated_file_from_url, image_backend,
};
use crate::backend::tests::TestBackend;

type Respond = dyn Fn(&Invocation) -> Result<Output, RunError> + Send + Sync;

/// Answers every run with `respond` and keeps the invocations.
struct FakeRunner {
    calls: Mutex<Vec<Invocation>>,
    respond: Box<Respond>,
}

impl Runner for FakeRunner {
    fn run(&self, invocation: Invocation) -> RunFuture<'_> {
        let answer = (self.respond)(&invocation);
        self.calls.lock().unwrap().push(invocation);
        Box::pin(async move { answer })
    }
}

fn fake(
    respond: impl Fn(&Invocation) -> Result<Output, RunError> + Send + Sync + 'static,
) -> Arc<FakeRunner> {
    Arc::new(FakeRunner {
        calls: Mutex::new(Vec::new()),
        respond: Box::new(respond),
    })
}

/// A backend whose CLI runs go to `runner`.
fn backend_with(runner: &Arc<FakeRunner>) -> TestBackend {
    TestBackend::with_cli(false, Cli::with_runner(runner.clone()))
}

fn ok(stdout: impl Into<Vec<u8>>) -> Result<Output, RunError> {
    Ok(Output {
        code: Some(0),
        stdout: stdout.into(),
        stderr: Vec::new(),
    })
}

fn args(invocation: &Invocation) -> Vec<String> {
    invocation
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn arg_after(invocation: &Invocation, flag: &str) -> PathBuf {
    let args = args(invocation);
    let at = args.iter().position(|arg| arg == flag).unwrap();
    PathBuf::from(&args[at + 1])
}

fn stdin(invocation: &Invocation) -> String {
    String::from_utf8(invocation.stdin.clone()).unwrap()
}

fn claude_result(text: &str) -> Vec<u8> {
    json!({"type": "result", "subtype": "success", "is_error": false, "result": text})
        .to_string()
        .into_bytes()
}

fn png(path: &Path, width: u32, height: u32, colour: [u8; 3]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    RgbImage::from_pixel(width, height, Rgb(colour))
        .save(path)
        .unwrap();
}

fn write_reference(backend: &TestBackend, name: &str) -> String {
    png(
        &backend
            .dir
            .path()
            .join("references")
            .join("upload-1")
            .join(name),
        2000,
        1000,
        [200, 120, 120],
    );
    format!("front:references/upload-1/{name}")
}

const SAFE: &str = r#"{"allowed": true, "reason": "ok", "message": ""}"#;
const DETAILS: &str = r#"{"features": [{"id": "f1", "kind": "ears", "label": "Ears", "description": "black cat ears"}], "warnings": []}"#;

fn analysis_request(key: &str) -> AnalysisRequest {
    AnalysisRequest {
        free_text: "keep the ears".to_owned(),
        locale: "ja".to_owned(),
        reference_keys: vec![key.to_owned()],
        character_session_id: "session-1".to_owned(),
        analysis_id: "a1".to_owned(),
        ..AnalysisRequest::default()
    }
}

async fn analyze(
    backend: &TestBackend,
    provider: &str,
    request: &AnalysisRequest,
) -> Result<analysis::AnalysisResult, AnalysisError> {
    backend
        .state
        .settings
        .update(&[("llm_provider".to_owned(), Some(provider.to_owned()))])
        .unwrap();
    let analysis_backend = analysis::analysis_backend(&backend.state.settings.settings())
        .map_err(AnalysisError::Failed)?;
    analysis_backend.analyze(&backend.state, request).await
}

// --- Claude Code ---------------------------------------------------------------

#[test]
fn claude_code_runs_restricted_without_tools() {
    let args: Vec<OsString> = claude::arguments(claude::Tools::None, " sonnet ");
    let args: Vec<&str> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
    for expected in [
        "-p",
        "--restricted",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ] {
        assert!(args.contains(&expected), "{expected}");
    }
    let at = args.iter().position(|arg| *arg == "--tools").unwrap();
    assert_eq!(args[at + 1], "");
    assert!(!args.contains(&"--allowedTools"));
    assert!(
        args.windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"])
    );
    assert!(args.ends_with(&["--model", "sonnet"]));

    let args = claude::arguments(claude::Tools::ReadImages, "");
    let args: Vec<&str> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
    assert!(
        args.windows(4)
            .any(|four| four == ["--tools", "Read", "--allowedTools", "Read"])
    );
    assert!(!args.contains(&"--model"));
}

#[test]
fn only_a_nested_claude_code_session_loses_its_variables() {
    let outside = [("PATH", "x"), ("ANTHROPIC_BASE_URL", "y")];
    assert!(claude::nested_session_variables(outside).is_empty());
    let nested = [
        ("PATH", "x"),
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_ENTRYPOINT", "x"),
        ("CLAUDE_CONFIG_DIR", "x"),
        ("ANTHROPIC_BASE_URL", "x"),
        ("ANTHROPIC_API_KEY", "x"),
    ];
    assert_eq!(
        claude::nested_session_variables(nested),
        ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "ANTHROPIC_BASE_URL"]
    );
}

#[test]
fn claude_code_output_is_read_and_failures_are_explained() {
    let read = |stdout: &str, stderr: &str, code| {
        claude::parse_output(stdout.as_bytes(), stderr.as_bytes(), code, "claude")
    };
    assert_eq!(
        read(
            r#"{"result": "```json\n{\"a\": 1}\n```", "is_error": false}"#,
            "",
            Some(0)
        )
        .unwrap(),
        "{\"a\": 1}"
    );
    let not_logged_in = read(
        r#"{"result": "Not logged in · Please run /login", "is_error": true}"#,
        "",
        Some(1),
    )
    .unwrap_err();
    assert_eq!(not_logged_in.code, Some("cli_login_required"));
    assert_eq!(
        Value::Object(not_logged_in.params.clone()),
        json!({"name": "Claude Code", "command": "claude"})
    );
    assert!(not_logged_in.message.contains("not logged in"));
    let limited = read(
        r#"{"result": "", "is_error": true, "api_error_status": 429}"#,
        "",
        Some(1),
    )
    .unwrap_err();
    assert_eq!(limited.code, Some("cli_usage_limit"));
    let empty = read(r#"{"result": "  ", "is_error": false}"#, "", Some(0)).unwrap_err();
    assert_eq!(empty.code, Some("cli_empty_reply"));
    let old = read("", "error: unknown option '--restricted'", Some(1)).unwrap_err();
    assert_eq!(old.code, Some("cli_failed"));
    assert_eq!(
        Value::Object(old.params.clone()),
        json!({
            "name": "Claude Code",
            "exit_code": 1,
            "outdated": true,
            "detail": "error: unknown option '--restricted'",
        })
    );
    assert!(
        old.message.contains("exit code 1") && old.message.contains("out of date"),
        "{old}"
    );
    let silent = read("", "", None).unwrap_err();
    assert_eq!(silent.code, Some("cli_failed_silently"));
    assert!(!silent.params.contains_key("exit_code"));
    let refused = read(
        r#"{"result": "", "is_error": true, "subtype": "error_max_turns"}"#,
        "",
        Some(1),
    )
    .unwrap_err();
    assert_eq!(refused.code, Some("cli_failed"));
    assert_eq!(refused.params["detail"], json!("error_max_turns"));
}

#[tokio::test]
async fn the_chat_assistant_can_run_on_claude_code() {
    let runner = fake(|_| {
        ok(claude_result(
            r#"{"reply": "参考画像を見ます", "tool_calls": [{"name": "analyze_references", "arguments": {}}]}"#,
        ))
    });
    let backend = backend_with(&runner);
    backend
        .state
        .settings
        .update(&[
            (
                "agent_llm_provider".to_owned(),
                Some("claude_code".to_owned()),
            ),
            (
                "agent_claude_code_model".to_owned(),
                Some("opus".to_owned()),
            ),
        ])
        .unwrap();
    let workspace = backend
        .dir
        .path()
        .join("agent")
        .join("c1")
        .join("claude-code");
    let backends = llm::Backends {
        chat: &backend.state.chat,
        cli: &backend.state.cli,
        workspace: &workspace,
    };
    let tools = [ToolSpec {
        name: "analyze_references",
        description: "Analyze.",
        parameters: json!({"type": "object"}),
    }];
    let turn = llm::complete(
        &backend.state.settings.settings(),
        &backends,
        "SYSTEM",
        &[json!({"role": "user", "content": "<script>見て</script>"})],
        &tools,
    )
    .await
    .unwrap();
    assert_eq!(turn.text, "参考画像を見ます");
    assert_eq!(turn.tool_calls[0].name, "analyze_references");

    let calls = runner.calls.lock().unwrap();
    let call = &calls[0];
    assert_eq!(call.program, "claude");
    assert_eq!(call.cwd, workspace);
    let at = args(call).iter().position(|arg| arg == "--tools").unwrap();
    assert_eq!(args(call)[at + 1], "");
    assert!(args(call).ends_with(&["--model".to_owned(), "opus".to_owned()]));
    let prompt = stdin(call);
    assert!(prompt.starts_with("SYSTEM\n\n## Tools\n- analyze_references: Analyze."));
    assert!(prompt.contains("[user] <script>見て</script>"));
    assert!(workspace.join("agent-turn-prompt.md").is_file());
    assert!(workspace.join("agent-turn-output.json").is_file());
}

#[tokio::test]
async fn a_missing_cli_says_so() {
    let runner = fake(|_| Err(RunError::NotFound));
    let backend = backend_with(&runner);
    backend
        .state
        .settings
        .update(&[(
            "agent_llm_provider".to_owned(),
            Some("claude_code".to_owned()),
        )])
        .unwrap();
    let workspace = backend.dir.path().join("w");
    let backends = llm::Backends {
        chat: &backend.state.chat,
        cli: &backend.state.cli,
        workspace: &workspace,
    };
    let error = llm::complete(&backend.state.settings.settings(), &backends, "S", &[], &[])
        .await
        .unwrap_err();
    assert_eq!(error.code, Some("cli_not_found"));
    assert_eq!(
        Value::Object(error.params.clone()),
        json!({"name": "Claude Code", "program": "claude"})
    );
    assert!(error.message.contains("was not found"), "{error}");

    let timed_out = super::run_error_failure(
        "Codex",
        "codex",
        &RunError::TimedOut(Duration::from_secs(240)),
    );
    assert_eq!(timed_out.code, Some("cli_timeout"));
    assert_eq!(
        Value::Object(timed_out.params.clone()),
        json!({"name": "Codex", "seconds": 240})
    );
    let not_started = super::run_error_failure(
        "Codex",
        "codex",
        &RunError::Start("access denied".to_owned()),
    );
    assert_eq!(not_started.code, Some("cli_start_failed"));
    assert_eq!(not_started.params["detail"], json!("access denied"));
}

#[tokio::test]
async fn claude_code_analysis_reads_staged_references() {
    let answers = Mutex::new(vec![claude_result(DETAILS), claude_result(SAFE)]);
    let runner = fake(move |invocation| {
        // The staged image is there when the CLI runs.
        assert!(invocation.cwd.join("refs").join("ref-1.png").is_file());
        ok(answers.lock().unwrap().pop().unwrap())
    });
    let backend = backend_with(&runner);
    let key = write_reference(&backend, "front.png");
    let result = analyze(&backend, "claude_code", &analysis_request(&key))
        .await
        .unwrap();
    assert_eq!(result.features[0].description, "black cat ears");

    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let workspace = backend
        .dir
        .path()
        .join("claude-code")
        .join("session-1")
        .join("analysis-a1");
    assert_eq!(calls[0].cwd, workspace);
    assert!(
        args(&calls[0])
            .windows(2)
            .any(|pair| pair == ["--allowedTools", "Read"])
    );
    let safety = stdin(&calls[0]);
    assert!(
        safety.starts_with("The uploaded reference images are files in the current directory.")
    );
    assert!(safety.contains("\n- refs/ref-1.png\n"));
    assert!(safety.contains("Decide whether the uploaded images are usable"));
    assert!(stdin(&calls[1]).contains("Analyze the uploaded character references"));
    // Shrunk like the chat API's references.
    let staged = image::open(workspace.join("refs").join("ref-1.png")).unwrap();
    assert_eq!((staged.width(), staged.height()), (1536, 768));
}

// --- Codex -----------------------------------------------------------------------

#[test]
fn codex_runs_read_only_without_the_shell() {
    let workspace = Path::new("w");
    let images = [PathBuf::from("a.png"), PathBuf::from("b.png")];
    let image_args = codex::arguments(
        codex::Purpose::Image,
        workspace,
        &images,
        Path::new("w/last.txt"),
    );
    let image_args: Vec<&str> = image_args.iter().map(|arg| arg.to_str().unwrap()).collect();
    assert_eq!(image_args[0], "exec");
    for pair in [
        ["--sandbox", "read-only"],
        ["--disable", "shell_tool"],
        ["--disable", "apps"],
        ["--disable", "plugins"],
        ["-c", "web_search=disabled"],
        ["-C", "w"],
        ["-o", "w/last.txt"],
        ["--image", "a.png"],
        ["--image", "b.png"],
    ] {
        assert!(image_args.windows(2).any(|found| found == pair), "{pair:?}");
    }
    assert!(image_args.contains(&"--ignore-user-config"));
    assert!(image_args.ends_with(&["--", "-"]));
    assert!(!image_args.contains(&"--ephemeral"));
    assert!(!image_args.contains(&"image_generation"));

    let analysis_args =
        codex::arguments(codex::Purpose::Analysis, workspace, &images, Path::new("l"));
    let analysis_args: Vec<&str> = analysis_args
        .iter()
        .map(|arg| arg.to_str().unwrap())
        .collect();
    assert!(analysis_args.contains(&"--ephemeral"));
    assert!(
        analysis_args
            .windows(2)
            .any(|pair| pair == ["--disable", "image_generation"])
    );
}

#[test]
fn codex_events_give_the_thread_the_answer_and_any_commands() {
    let events = b"{\"type\":\"thread.started\",\"thread_id\":\"t-1\"}\nnot json\n\
{\"type\":\"item.started\",\"item\":{\"type\":\"agent_message\",\"text\":\"partial\"}}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\" done \"}}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"command\":\"python draw.py\"}}\n";
    let run = codex::read_events(events);
    assert_eq!(run.thread_id.as_deref(), Some("t-1"));
    assert_eq!(run.text, "done");
    assert_eq!(run.commands, ["python draw.py"]);
}

#[test]
fn codex_failures_are_explained() {
    let detail = |stdout: &str, stderr: &str| {
        codex::failure_detail(stdout.as_bytes(), stderr.as_bytes(), "codex")
    };
    let limited = detail(
        r#"{"type":"error","message":"You've hit your usage limit. Try again at 5pm."}"#,
        "",
    );
    assert_eq!(limited.code, Some("cli_usage_limit"));
    assert_eq!(limited.params["name"], json!("Codex"));
    let logged_out = detail("", "Error: 401 Unauthorized");
    assert_eq!(logged_out.code, Some("cli_login_required"));
    assert_eq!(logged_out.params["command"], json!("codex login"));
    let offline = detail(
        r#"{"type":"turn.failed","error":{"message":"failed to connect to api"}}"#,
        "",
    );
    assert_eq!(offline.code, Some("cli_offline"));
    let other = detail(
        "",
        "Reading additional input from stdin...\nError: unexpected argument '--ignore-rules' found",
    );
    assert_eq!(other.code, Some("cli_failed"));
    assert_eq!(other.params["outdated"], json!(true));
    assert_eq!(
        other.params["detail"],
        json!("Error: unexpected argument '--ignore-rules' found")
    );
    assert!(
        other.message.contains("out of date") && other.message.contains("--ignore-rules"),
        "{other}"
    );
    assert_eq!(detail("", "").code, Some("cli_failed_silently"));
}

#[test]
fn only_the_runs_own_tool_images_are_collected() {
    let home = tempfile::tempdir().unwrap();
    let folder = home.path().join("generated_images").join("t-1");
    png(&folder.join("first.png"), 4, 4, [0, 0, 0]);
    std::thread::sleep(Duration::from_millis(30));
    png(&folder.join("second.webp.png"), 4, 4, [0, 0, 0]);
    std::fs::write(folder.join("notes.txt"), "x").unwrap();
    let found = codex::tool_images(home.path(), "t-1");
    let names: Vec<_> = found
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(names, ["first.png", "second.webp.png"]);
    assert!(codex::tool_images(home.path(), "../t-1").is_empty());
    assert!(codex::tool_images(home.path(), "").is_empty());
    assert!(codex::tool_images(home.path(), "t-2").is_empty());
}

#[tokio::test]
async fn codex_analysis_attaches_the_references() {
    let runner = fake(|invocation| {
        // The answer comes from the -o file when there is one.
        let answer = if stdin(invocation).starts_with("Decide whether") {
            SAFE
        } else {
            DETAILS
        };
        std::fs::write(arg_after(invocation, "-o"), answer).unwrap();
        ok("{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n")
    });
    let backend = backend_with(&runner);
    let key = write_reference(&backend, "front.png");
    let result = analyze(&backend, "codex", &analysis_request(&key))
        .await
        .unwrap();
    assert_eq!(result.features[0].kind, "ears");
    let calls = runner.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].program, "codex");
    let image = arg_after(&calls[0], "--image");
    assert!(
        image.is_absolute() && image.is_file(),
        "{}",
        image.display()
    );
    assert!(args(&calls[0]).contains(&"--ephemeral".to_owned()));
}

#[tokio::test]
async fn codex_analysis_fails_clearly() {
    // An empty answer, a command, a refused reference.
    let backend_for = |events: &'static str| backend_with(&fake(move |_| ok(events)));
    let empty = backend_for("{\"type\":\"thread.started\",\"thread_id\":\"t\"}\n");
    let key = write_reference(&empty, "front.png");
    match analyze(&empty, "codex", &analysis_request(&key)).await {
        Err(AnalysisError::Failed(failure)) => {
            assert_eq!(failure.code, Some("cli_empty_reply"), "{failure}");
        }
        other => panic!("{other:?}"),
    }
    let commands = backend_for(
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"command\":\"cat ~/.codex/auth.json\"}}\n",
    );
    let key = write_reference(&commands, "front.png");
    match analyze(&commands, "codex", &analysis_request(&key)).await {
        Err(AnalysisError::Failed(failure)) => {
            assert_eq!(failure.code, Some("cli_ran_command"), "{failure}");
            assert_eq!(failure.params["detail"], json!("cat ~/.codex/auth.json"));
        }
        other => panic!("{other:?}"),
    }
    let rejected = backend_for(
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"allowed\\\": false, \\\"reason\\\": \\\"adult_explicit\\\", \\\"message\\\": \\\"no\\\"}\"}}\n",
    );
    let key = write_reference(&rejected, "front.png");
    match analyze(&rejected, "codex", &analysis_request(&key)).await {
        Err(AnalysisError::Rejected { reason, .. }) => {
            assert_eq!(reason, "reference_adult_explicit")
        }
        other => panic!("{other:?}"),
    }
}

/// A runner that answers like Codex after its image tool saved an image of
/// `size` in `colour`.
fn codex_painter(home: PathBuf, size: (u32, u32), colour: [u8; 3]) -> Arc<FakeRunner> {
    fake(move |_| {
        png(
            &home
                .join("generated_images")
                .join("thread-9")
                .join("ig_1.png"),
            size.0,
            size.1,
            colour,
        );
        ok("{\"type\":\"thread.started\",\"thread_id\":\"thread-9\"}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Done.\"}}\n\
{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1200,\"cached_input_tokens\":200,\"output_tokens\":80}}\n")
    })
}

async fn generate_with_codex(
    runner: &Arc<FakeRunner>,
    home: &Path,
    build: impl FnOnce(&TestBackend) -> GenerationRequest,
) -> (
    TestBackend,
    Result<crate::backend::generation::GeneratedImage, Failure>,
) {
    let mut cli = Cli::with_runner(runner.clone());
    cli.codex_home = Some(home.to_owned());
    let backend = TestBackend::with_cli(false, cli);
    backend
        .state
        .settings
        .update(&[("image_provider".to_owned(), Some("codex".to_owned()))])
        .unwrap();
    let request = build(&backend);
    let image_backend = image_backend(&backend.state.settings.settings()).unwrap();
    let result = image_backend
        .generate(
            &backend.state,
            &request,
            &AtomicBool::new(false),
            &|_, _| {},
        )
        .await;
    (backend, result)
}

#[tokio::test]
async fn codex_generates_a_labelled_front_view() {
    let home = tempfile::tempdir().unwrap();
    let runner = codex_painter(home.path().to_owned(), (1024, 1536), [10, 200, 10]);
    let (backend, result) = generate_with_codex(&runner, home.path(), |backend| {
        let mut request = GenerationRequest::new("session-1", "ja", Mode::FrontDesign);
        request.reference_keys = vec![write_reference(backend, "front.png")];
        request.free_text = "keep the bow".to_owned();
        request
    })
    .await;
    let output = result.unwrap();
    assert_eq!((output.width, output.height), (800, 1100));
    let usage = backend
        .state
        .generation
        .jobs
        .get(&output.job_id)
        .unwrap()
        .token_usage
        .unwrap();
    assert_eq!(usage["input_tokens"], 1200);
    assert_eq!(usage["cached_input_tokens"], 200);
    assert_eq!(usage["total_tokens"], 1280);
    assert!(usage.get("cost").is_none());
    let clean = image::open(&output.clean_path).unwrap().to_rgb8();
    assert_eq!(clean.get_pixel(400, 550).0, [10, 200, 10]);
    let public = generated_file_from_url(backend.dir.path(), &output.url).unwrap();
    assert_ne!(image::open(public).unwrap().to_rgb8(), clean);

    let calls = runner.calls.lock().unwrap();
    let prompt = stdin(&calls[0]);
    assert!(prompt.starts_with(codex::TOOL_REQUIREMENT));
    assert!(prompt.contains("Image 1 is the character reference."));
    assert!(prompt.ends_with("keep the bow"));
    let image = arg_after(&calls[0], "--image");
    assert_eq!(image.file_name().unwrap(), "input-1.png");
    assert!(
        image.starts_with(backend.dir.path().join("codex").join("session-1")),
        "{}",
        image.display()
    );
    let job = backend.state.generation.jobs.get(&output.job_id).unwrap();
    assert_eq!(job.status, "succeeded");
    assert_eq!(job.provider, "codex");
    assert_eq!(job.prompt, prompt);
    assert_eq!(job.request, json!({"cli": "codex exec", "images": 1}));
}

#[tokio::test]
async fn a_codex_local_revision_edits_the_crop_and_is_pasted_back() {
    let home = tempfile::tempdir().unwrap();
    let runner = codex_painter(home.path().to_owned(), (300, 300), [250, 0, 0]);
    let (_backend, result) = generate_with_codex(&runner, home.path(), |backend| {
        let folder = backend.dir.path().join("local");
        png(&folder.join("base.png"), 800, 1100, [30, 30, 30]);
        image::GrayImage::from_fn(800, 1100, |x, y| {
            image::Luma([if (300..400).contains(&x) && (500..600).contains(&y) {
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
            edit_note: "redden the cheek".to_owned(),
            base_width: 800,
            base_height: 1100,
            feather_radius_px: 3,
            subject: "head_shell",
        });
        request
    })
    .await;
    let clean = image::open(result.unwrap().clean_path).unwrap().to_rgb8();
    assert_eq!(clean.dimensions(), (800, 1100));
    assert_eq!(clean.get_pixel(100, 100).0, [30, 30, 30]);
    assert_eq!(clean.get_pixel(350, 550).0, [250, 0, 0]);

    let calls = runner.calls.lock().unwrap();
    let prompt = stdin(&calls[0]);
    assert!(prompt.contains("ordinary image edit on Image 1"));
    assert!(prompt.contains("Apply this local edit to Image 1: redden the cheek"));
    // Only the crop goes in (the 100 px square grown to 384 px).
    let crop = image::open(arg_after(&calls[0], "--image")).unwrap();
    assert_eq!((crop.width(), crop.height()), (384, 384));
    assert_eq!(
        args(&calls[0])
            .iter()
            .filter(|arg| *arg == "--image")
            .count(),
        1
    );
}

#[tokio::test]
async fn codex_without_a_tool_image_fails() {
    let home = tempfile::tempdir().unwrap();
    let runner = fake(|_| {
        ok("{\"type\":\"thread.started\",\"thread_id\":\"t-empty\"}\n\
{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"I cannot use the image tool.\"}}\n")
    });
    let (backend, result) = generate_with_codex(&runner, home.path(), |backend| {
        let mut request = GenerationRequest::new("session-1", "ja", Mode::FrontDesign);
        request.reference_keys = vec![write_reference(backend, "front.png")];
        request
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(error.code, Some("codex_no_image"), "{error}");
    assert_eq!(
        error.params["detail"],
        json!("I cannot use the image tool.")
    );
    assert!(error.message.contains("I cannot use the image tool."));
    assert_eq!(backend.state.generation.jobs.list()[0].status, "failed");
}

// --- The real process runner ------------------------------------------------------

/// A script that behaves as `body` (cmd on Windows, sh elsewhere).
fn script(dir: &Path, name: &str, windows: &str, unix: &str) -> String {
    if cfg!(windows) {
        let path = dir.join(format!("{name}.cmd"));
        std::fs::write(&path, format!("@echo off\r\n{windows}\r\n")).unwrap();
        path.to_string_lossy().into_owned()
    } else {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{unix}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path.to_string_lossy().into_owned()
    }
}

fn invocation(program: String, dir: &Path, timeout: Duration) -> Invocation {
    Invocation {
        program,
        args: vec!["--flag".into()],
        stdin: b"hello from stdin\n".to_vec(),
        cwd: dir.to_owned(),
        remove_env: vec!["MASKETTE_TEST_SECRET".to_owned()],
        timeout,
    }
}

#[tokio::test]
async fn the_process_runner_passes_stdin_and_reads_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let program = script(
        dir.path(),
        "echo-cli",
        "findstr \"^\"\r\necho oops 1>&2\r\nexit /b 3",
        "cat\necho oops 1>&2\nexit 3",
    );
    let output = ProcessRunner
        .run(invocation(program, dir.path(), Duration::from_secs(30)))
        .await
        .unwrap();
    assert_eq!(output.code, Some(3));
    assert!(!output.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("hello from stdin"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("oops"));
}

#[tokio::test]
async fn the_process_runner_stops_a_slow_cli_and_reports_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    let program = script(
        dir.path(),
        "slow-cli",
        "ping -n 6 127.0.0.1 >nul",
        "sleep 5",
    );
    let started = std::time::Instant::now();
    let error = ProcessRunner
        .run(invocation(program, dir.path(), Duration::from_millis(300)))
        .await
        .unwrap_err();
    assert_eq!(error, RunError::TimedOut(Duration::from_millis(300)));
    assert!(started.elapsed() < Duration::from_secs(4));

    let missing = dir
        .path()
        .join("no-such-cli")
        .to_string_lossy()
        .into_owned();
    let error = ProcessRunner
        .run(invocation(missing, dir.path(), Duration::from_secs(5)))
        .await
        .unwrap_err();
    assert_eq!(error, RunError::NotFound);
    let error = ProcessRunner
        .run(invocation(
            "maskette-no-such-cli-0451".to_owned(),
            dir.path(),
            Duration::from_secs(5),
        ))
        .await
        .unwrap_err();
    assert_eq!(error, RunError::NotFound);
}

#[test]
fn codex_prompts_put_the_codex_lines_first() {
    let overrides = crate::backend::prompts::Overrides::from_value(json!({
        "codex.tool_requirement": "USE THE TOOL",
    }));
    let prompt = codex::image_prompt(&overrides, "SHARED", true);
    let lines: Vec<&str> = prompt.lines().collect();
    assert_eq!(lines[0], "USE THE TOOL");
    assert_eq!(lines[1], codex::TOOL_OUTPUT_NOTE);
    assert!(lines[2].contains("ordinary image edit"));
    assert_eq!(lines[3..], ["", "SHARED"]);
}

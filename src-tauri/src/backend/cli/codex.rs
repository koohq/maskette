//! Codex (`codex exec`): reference analysis and image generation (the old
//! `app/generation/backends/codex.py`).
//!
//! Each run reads the prompt from standard input, attaches the images with
//! `--image`, prints its events as JSON lines and writes its last message
//! to a file. It runs in the read-only sandbox with the shell tool, apps,
//! plugins, the browser, computer use, sub-agents and web search turned off,
//! without the user's `config.toml` (so without their MCP servers) or
//! command rules. Analysis also has the image tool off and keeps no session.
//!
//! The image comes from Codex's built-in image generation tool, which saves
//! it under `<CODEX_HOME>/generated_images/<thread id>/`. Only the tool
//! writes there (the sandbox lets the model write nothing), so the image
//! cannot have been drawn with code. Nothing else in Codex's home folder is
//! read. The old backend also accepted a `manifest.json` the model wrote
//! into a fully writable workspace; that path is gone with the write access.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Cli, Invocation, cli_failed, keep, login_required, run_error_failure, shorten, usage_limit,
};
use crate::backend::failure::Failure;
use crate::backend::generation::usage::TokenUsage;
use crate::backend::prompts::Overrides;

const NAME: &str = "Codex";
const IMAGE_EXTENSIONS: [&str; 4] = ["png", "webp", "jpg", "jpeg"];

pub const TOOL_REQUIREMENT: &str = "You must use the image generation tool to create the image output. If the image \
generation tool is unavailable, blocked, or cannot complete the request, fail the task and produce no image. Do not \
create, draw, render, approximate, or trace the output using Python, PIL, SVG, canvas, CSS, vector shapes, \
screenshots, drawing library code, or any other manual/code-based method.";

pub const TOOL_OUTPUT_NOTE: &str = "Run no commands and write no files: the service collects the image directly from \
the image generation tool's output. Produce exactly one image.";

const LOCAL_EDIT_NOTE: &str = "Use the image generation tool's ordinary image edit on Image 1, the cropped region. No \
mask is needed: the service pastes your result back into the full image and blends it inside the user's mask. Do \
not generate a new image from scratch, and keep the same aspect ratio and framing as Image 1.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Analysis,
    Image,
}

/// The Codex-only lines put before the shared image prompt.
pub fn image_prompt(overrides: &Overrides, shared_prompt: &str, local_edit: bool) -> String {
    let mut lines = vec![
        overrides.text("codex.tool_requirement", TOOL_REQUIREMENT),
        overrides.text("codex.tool_output_note", TOOL_OUTPUT_NOTE),
    ];
    if local_edit {
        lines.push(LOCAL_EDIT_NOTE);
    }
    lines.extend(["", shared_prompt]);
    lines.join("\n")
}

pub fn arguments(
    purpose: Purpose,
    workspace: &Path,
    images: &[PathBuf],
    last_message: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
        "--sandbox",
        "read-only",
        "--disable",
        "shell_tool",
        "--disable",
        "apps",
        "--disable",
        "plugins",
        "--disable",
        "browser_use",
        "--disable",
        "computer_use",
        "--disable",
        "multi_agent",
        // Bare words are not TOML, so Codex takes them as strings.
        "-c",
        "web_search=disabled",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if purpose == Purpose::Analysis {
        for arg in [
            "--ephemeral",
            "--disable",
            "image_generation",
            "-c",
            "model_reasoning_effort=high",
        ] {
            args.push(arg.into());
        }
    }
    args.push("-C".into());
    args.push(workspace.into());
    args.push("-o".into());
    args.push(last_message.into());
    for image in images {
        args.push("--image".into());
        args.push(image.into());
    }
    // `--image` takes several values; `--` ends them, and `-` reads the
    // prompt from standard input.
    args.push("--".into());
    args.push("-".into());
    args
}

/// A finished run.
#[derive(Debug, Clone, Default)]
pub struct Run {
    /// The last message, from the `-o` file or else the events.
    pub text: String,
    pub thread_id: Option<String>,
    /// Commands the model ran (there should be none).
    pub commands: Vec<String>,
    /// The tokens its turns used (`turn.completed`), added up.
    pub usage: Option<TokenUsage>,
}

/// Runs one prompt in `workspace` with `images` attached; `label` names the
/// kept prompt, event and error files.
pub async fn run(
    cli: &Cli,
    workspace: &Path,
    label: &str,
    prompt: &str,
    images: &[PathBuf],
    purpose: Purpose,
) -> Result<Run, Failure> {
    std::fs::create_dir_all(workspace)
        .map_err(|error| format!("cannot create the Codex working folder: {error}"))?;
    keep(workspace, &format!("{label}-prompt.md"), prompt.as_bytes());
    let last_message = workspace.join(format!("{label}-last-message.txt"));
    let _ = std::fs::remove_file(&last_message);
    let invocation = Invocation {
        program: cli.codex.clone(),
        args: arguments(purpose, workspace, images, &last_message),
        stdin: prompt.as_bytes().to_vec(),
        cwd: workspace.to_owned(),
        remove_env: Vec::new(),
        timeout: match purpose {
            Purpose::Analysis => cli.codex_analysis_timeout,
            Purpose::Image => cli.codex_image_timeout,
        },
    };
    let output = cli
        .runner
        .run(invocation)
        .await
        .map_err(|error| run_error_failure(NAME, &cli.codex, &error))?;
    keep(workspace, &format!("{label}-events.jsonl"), &output.stdout);
    keep(workspace, &format!("{label}-stderr.log"), &output.stderr);
    if !output.success() {
        return Err(failure_detail(&output.stdout, &output.stderr, &cli.codex));
    }
    let mut run = read_events(&output.stdout);
    run.usage = TokenUsage::from_codex_events(&output.stdout);
    if let Ok(text) = std::fs::read_to_string(&last_message)
        && !text.trim().is_empty()
    {
        text.trim().clone_into(&mut run.text);
    }
    if let Some(command) = run.commands.first() {
        let command = shorten(command);
        return Err(Failure::coded(
            "cli_ran_command",
            format!("Codex tried to run a command, so its result was not used: {command}"),
        )
        .with("name", NAME)
        .with("detail", command));
    }
    Ok(run)
}

fn events(stdout: &[u8]) -> impl Iterator<Item = serde_json::Map<String, Value>> + '_ {
    stdout.split(|byte| *byte == b'\n').filter_map(|line| {
        match serde_json::from_slice::<Value>(line.trim_ascii()) {
            Ok(Value::Object(event)) => Some(event),
            _ => None,
        }
    })
}

pub fn read_events(stdout: &[u8]) -> Run {
    let mut run = Run::default();
    for event in events(stdout) {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "thread.started" {
            run.thread_id = event
                .get("thread_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            continue;
        }
        let Some(item) = event.get("item").and_then(Value::as_object) else {
            continue;
        };
        match item.get("type").and_then(Value::as_str) {
            Some("command_execution") => run.commands.push(
                item.get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("(command)")
                    .to_owned(),
            ),
            Some("agent_message") if kind == "item.completed" => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    text.trim().clone_into(&mut run.text);
                }
            }
            _ => {}
        }
    }
    run
}

/// The images the image tool saved for `thread_id`, oldest first. A thread
/// id that is not a plain name finds nothing.
pub fn tool_images(codex_home: &Path, thread_id: &str) -> Vec<PathBuf> {
    let plain = !thread_id.is_empty()
        && thread_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !plain {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(codex_home.join("generated_images").join(thread_id)) else {
        return Vec::new();
    };
    let mut images: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
                    })
        })
        .map(|path| {
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (modified, path)
        })
        .collect();
    images.sort();
    images.into_iter().map(|(_, path)| path).collect()
}

/// What a failed run says, in the user's terms where it is a known case.
/// `program` is the command that starts Codex (to log in with).
pub fn failure_detail(stdout: &[u8], stderr: &[u8], program: &str) -> Failure {
    let mut messages = Vec::new();
    for event in events(stdout) {
        if let Some(message) = event.get("message").and_then(Value::as_str) {
            messages.push(message.trim().to_owned());
        }
        if let Some(message) = event
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
        {
            messages.push(message.trim().to_owned());
        }
    }
    let stderr = String::from_utf8_lossy(stderr);
    messages.extend(
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && *line != "Reading additional input from stdin...")
            .map(str::to_owned),
    );
    messages.retain(|message| !message.is_empty());
    for message in &messages {
        let lowered = message.to_lowercase();
        if lowered.contains("usage limit") {
            return usage_limit(NAME);
        }
        if lowered.contains("401 unauthorized")
            || lowered.contains("not logged in")
            || lowered.contains("missing bearer")
            || lowered.contains("token_expired")
            || lowered.contains("refresh_token")
        {
            return login_required(NAME, &format!("{program} login"));
        }
        if lowered.contains("tls handshake") || lowered.contains("failed to connect") {
            return Failure::coded(
                "cli_offline",
                "Codex could not reach its server. Check the network and try again.",
            )
            .with("name", NAME);
        }
    }
    cli_failed(
        NAME,
        None,
        &stderr,
        messages.last().map_or("", String::as_str),
    )
}

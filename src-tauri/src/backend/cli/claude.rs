//! Claude Code (`claude -p`): the chat assistant and reference analysis
//! (the old `app/generation/backends/claude_code.py`). Claude cannot make
//! images, so it is never the image backend.
//!
//! Each run is one print-mode call with the prompt on standard input and a
//! JSON result on standard output, in restricted mode (no command-running
//! tools, no web tools, no user/project settings, file tools confined to the
//! working folder), with no MCP servers, no skills or slash commands, and no
//! saved session. The chat gets no tools at all; analysis gets only Read,
//! to look at the reference images staged in its working folder.

use std::ffi::OsString;
use std::path::Path;

use serde_json::Value;

use super::{
    Cli, Invocation, cli_failed, empty_reply, keep, login_required, run_error_failure, shorten,
    usage_limit,
};
use crate::backend::failure::Failure;

const NAME: &str = "Claude Code";

/// What the run may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tools {
    None,
    /// Read, inside the working folder.
    ReadImages,
}

/// When Maskette itself runs inside a Claude Code session (development),
/// that session's variables would point the child at the host's proxy and
/// account; they are dropped only in that nested case. `CLAUDE_CONFIG_DIR`
/// (where the user keeps their login) is left alone.
pub fn nested_session_variables<'a>(
    environment: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<String> {
    let names: Vec<&str> = environment.into_iter().map(|(name, _)| name).collect();
    if !names.contains(&"CLAUDECODE") {
        return Vec::new();
    }
    names
        .into_iter()
        .filter(|name| {
            (name.starts_with("CLAUDE") && *name != "CLAUDE_CONFIG_DIR")
                || matches!(
                    *name,
                    "ANTHROPIC_BASE_URL" | "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_MODEL"
                )
        })
        .map(str::to_owned)
        .collect()
}

fn environment_to_drop() -> Vec<String> {
    let environment: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    nested_session_variables(
        environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    )
}

pub fn arguments(tools: Tools, model: &str) -> Vec<OsString> {
    let mut args: Vec<&str> = vec![
        "-p",
        "--output-format",
        "json",
        "--restricted",
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
        // Anything not allowed below is refused without asking.
        "--permission-mode",
        "dontAsk",
    ];
    match tools {
        Tools::None => args.extend(["--tools", ""]),
        Tools::ReadImages => args.extend(["--tools", "Read", "--allowedTools", "Read"]),
    }
    let model = model.trim();
    if !model.is_empty() {
        args.extend(["--model", model]);
    }
    args.into_iter().map(OsString::from).collect()
}

/// Runs one prompt in `workspace` and returns the answer's text. `label`
/// names the kept prompt and output files.
pub async fn run(
    cli: &Cli,
    workspace: &Path,
    label: &str,
    prompt: &str,
    tools: Tools,
    model: &str,
) -> Result<String, Failure> {
    std::fs::create_dir_all(workspace)
        .map_err(|error| format!("cannot create the Claude Code working folder: {error}"))?;
    keep(workspace, &format!("{label}-prompt.md"), prompt.as_bytes());
    let invocation = Invocation {
        program: cli.claude.clone(),
        args: arguments(tools, model),
        stdin: prompt.as_bytes().to_vec(),
        cwd: workspace.to_owned(),
        remove_env: environment_to_drop(),
        timeout: cli.claude_timeout,
    };
    let output = cli
        .runner
        .run(invocation)
        .await
        .map_err(|error| run_error_failure(NAME, &cli.claude, &error))?;
    keep(workspace, &format!("{label}-output.json"), &output.stdout);
    keep(workspace, &format!("{label}-stderr.log"), &output.stderr);
    parse_output(&output.stdout, &output.stderr, output.code, &cli.claude)
}

/// The text of a `--output-format json` result, or what went wrong.
/// `program` is the command that starts Claude Code (to log in with).
pub fn parse_output(
    stdout: &[u8],
    stderr: &[u8],
    code: Option<i32>,
    program: &str,
) -> Result<String, Failure> {
    let text = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(text.trim()) else {
        let detail = if stderr.trim().is_empty() {
            text.trim().to_owned()
        } else {
            stderr.trim().to_owned()
        };
        return Err(cli_failed(NAME, code, &stderr, &detail));
    };
    let result = payload
        .get("result")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    if let Some(denials) = payload
        .get("permission_denials")
        .and_then(Value::as_array)
        .filter(|denials| !denials.is_empty())
    {
        log::warn!(
            "Claude Code refused tool calls: {}",
            shorten(&Value::Array(denials.clone()).to_string())
        );
    }
    if payload.get("is_error").and_then(Value::as_bool) == Some(true) || code != Some(0) {
        return Err(failure_detail(&payload, &result, program));
    }
    let result = strip_code_fence(&result);
    if result.is_empty() {
        return Err(empty_reply(NAME));
    }
    Ok(result)
}

fn failure_detail(
    payload: &serde_json::Map<String, Value>,
    result: &str,
    program: &str,
) -> Failure {
    let status = payload.get("api_error_status").and_then(Value::as_u64);
    let lowered = result.to_lowercase();
    if status == Some(401) || lowered.contains("not logged in") || lowered.contains("/login") {
        return login_required(NAME, program);
    }
    if status == Some(429) || lowered.contains("usage limit") || lowered.contains("rate limit") {
        return usage_limit(NAME);
    }
    let reason = if result.is_empty() {
        payload
            .get("subtype")
            .or_else(|| payload.get("terminal_reason"))
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_owned()
    } else {
        shorten(result)
    };
    cli_failed(NAME, None, "", &reason)
}

fn strip_code_fence(text: &str) -> String {
    let stripped = text.trim();
    let Some(rest) = stripped.strip_prefix("```") else {
        return stripped.to_owned();
    };
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    body.trim_end()
        .strip_suffix("```")
        .unwrap_or(body)
        .trim()
        .to_owned()
}

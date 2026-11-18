//! The subscription CLIs: Codex (`codex exec`) and Claude Code (`claude -p`).
//!
//! They run on the user's own PC, logged in with the user's own
//! subscription; the settings recommend API keys and present these routes
//! as for the user's own use on their own PC. Maskette
//! only starts the official CLI: it never reads, copies or passes on their
//! login tokens and never calls the services behind them directly. What it
//! puts in the prompt comes partly from outside (reference descriptions,
//! the user's text), so each run gets no tools that run commands or reach
//! the network, and the CLI's own sandbox (see [`codex`] and [`claude`]).
//!
//! Every run goes through a [`Runner`], so the tests can stand in for the
//! CLIs without starting them ([`ProcessRunner`] is the real one).

pub mod claude;
pub mod codex;

#[cfg(test)]
mod tests;

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt as _;

use super::failure::Failure;

/// How much of a CLI's own error text goes into a message.
const DETAIL_CHARS: usize = 300;

/// One run of a CLI.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// A program name to look up on `PATH`, or a path.
    pub program: String,
    pub args: Vec<OsString>,
    /// Written to the CLI's standard input, which is then closed.
    pub stdin: Vec<u8>,
    pub cwd: PathBuf,
    /// Environment variables the CLI must not inherit.
    pub remove_env: Vec<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct Output {
    /// The exit code; `None` when the process was ended by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The program is not installed (not on `PATH`, or the path is missing).
    NotFound,
    /// It could not be started or talked to.
    Start(String),
    /// No answer within the time limit; the process was stopped.
    TimedOut(Duration),
}

pub type RunFuture<'a> = Pin<Box<dyn Future<Output = Result<Output, RunError>> + Send + 'a>>;

/// Runs CLIs. Dropping the future stops the process.
pub trait Runner: Send + Sync {
    fn run(&self, invocation: Invocation) -> RunFuture<'_>;
}

/// Starts the real programs with `tokio::process`.
pub struct ProcessRunner;

impl Runner for ProcessRunner {
    fn run(&self, invocation: Invocation) -> RunFuture<'_> {
        Box::pin(run_process(invocation))
    }
}

async fn run_process(invocation: Invocation) -> Result<Output, RunError> {
    let program = which::which(&invocation.program).map_err(|_| RunError::NotFound)?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(&invocation.args)
        .current_dir(&invocation.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A cancelled or timed-out run must not keep using the subscription.
        .kill_on_drop(true);
    for name in &invocation.remove_env {
        command.env_remove(name);
    }
    #[cfg(windows)]
    {
        // The app has no console; without this every run flashes a window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => RunError::NotFound,
        _ => RunError::Start(error.to_string()),
    })?;
    let stdin = child.stdin.take();
    let input = invocation.stdin;
    let writer = async move {
        if let Some(mut stdin) = stdin {
            // A CLI that exits without reading everything closes the pipe;
            // its exit code and output tell what happened.
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        }
    };
    let run = async {
        let ((), output) = tokio::join!(writer, child.wait_with_output());
        output.map_err(|error| RunError::Start(error.to_string()))
    };
    let output = tokio::time::timeout(invocation.timeout, run)
        .await
        .map_err(|_| RunError::TimedOut(invocation.timeout))??;
    Ok(Output {
        code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

/// Which CLIs to run, and how.
#[derive(Clone)]
pub struct Cli {
    pub runner: Arc<dyn Runner>,
    pub codex: String,
    pub claude: String,
    /// Codex's home folder (`CODEX_HOME`, else `~/.codex`), where its image
    /// tool saves the images; `None` reads the environment at each run.
    pub codex_home: Option<PathBuf>,
    pub codex_image_timeout: Duration,
    pub codex_analysis_timeout: Duration,
    pub codex_chat_timeout: Duration,
    pub claude_timeout: Duration,
}

impl Cli {
    pub const CODEX: &str = "codex";
    pub const CLAUDE: &str = "claude";

    pub fn new() -> Self {
        Self::with_runner(Arc::new(ProcessRunner))
    }

    pub fn with_runner(runner: Arc<dyn Runner>) -> Self {
        Self {
            runner,
            codex: Self::CODEX.to_owned(),
            claude: Self::CLAUDE.to_owned(),
            codex_home: None,
            // The old backend waited without a limit; a Codex image took
            // 1 to 2 minutes.
            codex_image_timeout: Duration::from_secs(900),
            codex_analysis_timeout: Duration::from_secs(240),
            // One chat step, like Claude Code's.
            codex_chat_timeout: Duration::from_secs(240),
            claude_timeout: Duration::from_secs(240),
        }
    }

    pub fn codex_home(&self) -> Option<PathBuf> {
        if let Some(home) = &self.codex_home {
            return Some(home.clone());
        }
        if let Some(home) = std::env::var_os("CODEX_HOME").filter(|home| !home.is_empty()) {
            return Some(PathBuf::from(home));
        }
        let user_home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|home| !home.is_empty())?;
        Some(PathBuf::from(user_home).join(".codex"))
    }
}

impl Default for Cli {
    fn default() -> Self {
        Self::new()
    }
}

/// The data folder's working folders for CLI runs (the old
/// `runtime/codex` and `runtime/claude-code`). The chat assistant keeps its
/// steps in the conversation's own folder instead (`agent/<id>/codex/`,
/// `agent/<id>/claude-code/`).
pub fn codex_root(data_dir: &Path) -> PathBuf {
    data_dir.join("codex")
}

pub fn claude_root(data_dir: &Path) -> PathBuf {
    data_dir.join("claude-code")
}

/// Keeps a CLI's raw output next to its prompt, for looking into failures.
fn keep(workspace: &Path, name: &str, bytes: &[u8]) {
    if let Err(error) = std::fs::write(workspace.join(name), bytes) {
        log::warn!("cannot keep {name}: {error}");
    }
}

pub fn shorten(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= DETAIL_CHARS {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(DETAIL_CHARS).collect();
    short.push('…');
    short
}

/// Why a run could not happen or did not finish.
fn run_error_failure(name: &str, program: &str, error: &RunError) -> Failure {
    match error {
        RunError::NotFound => Failure::coded(
            "cli_not_found",
            format!(
                "The {name} CLI ({program}) was not found. Install it and log in, or choose an API key route in the settings."
            ),
        )
        .with("name", name)
        .with("program", program),
        RunError::Start(error) => Failure::coded(
            "cli_start_failed",
            format!("Could not start the {name} CLI: {error}"),
        )
        .with("name", name)
        .with("detail", shorten(error)),
        RunError::TimedOut(limit) => Failure::coded(
            "cli_timeout",
            format!(
                "The {name} CLI did not answer within {} seconds and was stopped. Try again.",
                limit.as_secs()
            ),
        )
        .with("name", name)
        .with("seconds", limit.as_secs()),
    }
}

/// Unknown options mean an older CLI than Maskette expects.
fn looks_outdated(stderr: &str) -> bool {
    let lowered = stderr.to_lowercase();
    lowered.contains("unknown option")
        || lowered.contains("unexpected argument")
        || lowered.contains("unknown feature")
}

/// A CLI that ran and failed, with its own words (`detail`) when it said
/// anything. `exit_code` is told when it helps.
fn cli_failed(name: &str, exit_code: Option<i32>, stderr: &str, detail: &str) -> Failure {
    let detail = shorten(detail);
    let outdated = looks_outdated(stderr);
    let mut english = format!("The {name} CLI failed");
    if let Some(code) = exit_code {
        english.push_str(&format!(" (exit code {code})"));
    }
    english.push('.');
    if outdated {
        english.push_str(" The CLI may be out of date; update it and try again.");
    }
    let mut failure = if detail.is_empty() {
        english.push_str(" It printed nothing.");
        Failure::coded("cli_failed_silently", english)
    } else {
        english.push(' ');
        english.push_str(&detail);
        Failure::coded("cli_failed", english).with("detail", detail)
    }
    .with("name", name);
    if let Some(code) = exit_code {
        failure = failure.with("exit_code", code);
    }
    if outdated {
        failure = failure.with("outdated", true);
    }
    failure
}

/// The user has to act in the CLI itself (log in) or wait (usage limit).
fn login_required(name: &str, command: &str) -> Failure {
    Failure::coded(
        "cli_login_required",
        format!(
            "{name} is not logged in, or its login has expired. Run `{command}` in a terminal to log in, then try again."
        ),
    )
    .with("name", name)
    .with("command", command)
}

fn usage_limit(name: &str) -> Failure {
    Failure::coded(
        "cli_usage_limit",
        format!(
            "{name} has reached its usage limit. Try again later, or choose an API key route in the settings."
        ),
    )
    .with("name", name)
}

pub(crate) fn empty_reply(name: &str) -> Failure {
    Failure::coded(
        "cli_empty_reply",
        format!("{name} gave an empty answer. Try again."),
    )
    .with("name", name)
}

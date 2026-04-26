use crate::config::Config;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Lazily-loaded user config. Populated on first shell invocation; reused
/// thereafter so we don't re-parse `config.toml` per snippet.
fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| Config::load().unwrap_or_default())
}

fn apply_env(cmd: &mut Command) {
    for (k, v) in &config().env {
        cmd.env(k, v);
    }
}

/// Execute a shell snippet with `set -e` and `set -o pipefail` injected.
/// stdin/stdout/stderr are inherited from the parent so installers can
/// prompt for sudo, show progress, and stream output in real time.
///
/// Spawned as `bash -l` (login shell) so that `~/.bash_profile` /
/// `~/.profile` are sourced first — that's where installers like rustup,
/// bun, uv, etc. inject their `PATH` setup. Without `-l`, those `PATH`
/// additions wouldn't be visible to a snippet running immediately after
/// install, and every spell would need its own env-sourcing workaround.
///
/// Returns the exit status; never `Err` on non-zero exit (callers decide
/// what to do with a failed run).
pub fn run(label: &str, snippet: &str) -> Result<i32> {
    let prelude = "set -e\nset -o pipefail\n";
    let full = format!("{prelude}{snippet}");

    let mut cmd = Command::new("bash");
    cmd.arg("-l")
        .arg("-c")
        .arg(&full)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_env(&mut cmd);
    let status = cmd
        .status()
        .with_context(|| format!("spawning bash for {label}"))?;

    Ok(status.code().unwrap_or(-1))
}

/// Run a snippet silently (e.g. `verify`): capture all output, return
/// `(exit_code, captured)`. Used when the tool needs to *check* a state
/// without dumping the check's output to the user's terminal.
///
/// Also runs as a login shell — see [`run`] for rationale.
pub fn check(label: &str, snippet: &str) -> Result<CheckOutput> {
    let prelude = "set -e\nset -o pipefail\n";
    let full = format!("{prelude}{snippet}");

    let mut cmd = Command::new("bash");
    cmd.arg("-l").arg("-c").arg(&full).stdin(Stdio::null());
    apply_env(&mut cmd);
    let output = cmd
        .output()
        .with_context(|| format!("spawning bash for {label}"))?;

    Ok(CheckOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[derive(Debug, Clone)]
pub struct CheckOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CheckOutput {
    pub fn ok(&self) -> bool {
        self.exit_code == 0
    }

    /// stdout, with surrounding whitespace trimmed (typical for version_check).
    pub fn trimmed_stdout(&self) -> &str {
        self.stdout.trim()
    }
}

/// Print a banner to stderr to mark the start of a shell action.
pub fn banner(label: &str, channel: &str) -> Result<()> {
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "\n› {label} via {channel}").context("writing banner")?;
    Ok(())
}

/// Wrap `run` to fail when the snippet returns non-zero.
pub fn run_or_fail(label: &str, snippet: &str) -> Result<()> {
    let code = run(label, snippet)?;
    if code != 0 {
        bail!("{label} failed with exit code {code}");
    }
    Ok(())
}

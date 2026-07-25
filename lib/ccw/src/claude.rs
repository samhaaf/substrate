//! Claude Code process integration — the only place we shell out to `claude`.
//!
//! All invocations honor an account's `CLAUDE_CONFIG_DIR` (the machine default
//! leaves it unset). Timeout-guarded helpers wrap the zero-cost `/usage`
//! introspection and `auth status`; the passthrough spawn inherits stdio so the
//! wrapper is transparent (a drop-in prefix, `cc-recon.md` "recommended
//! invocation pattern").

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use substrate_types::{Result, SubstrateError};

use crate::usage::{parse_usage_json, UsageSnapshot};

/// `claude auth status` JSON (subset — `cc-recon.md` §1).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthStatus {
    #[serde(default, rename = "loggedIn")]
    pub logged_in: bool,
    #[serde(default, rename = "authMethod")]
    pub auth_method: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default, rename = "subscriptionType")]
    pub subscription_type: Option<String>,
}

/// Apply an account's `CLAUDE_CONFIG_DIR` to a command (None = leave unset =
/// machine default). We explicitly clear it when None so an inherited env from
/// the parent does not leak the wrong account.
fn apply_config_dir(cmd: &mut Command, config_dir: Option<&Path>) {
    match config_dir {
        Some(dir) => {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        None => {
            cmd.env_remove("CLAUDE_CONFIG_DIR");
        }
    }
}

/// Run a command to completion with a wall-clock timeout, capturing output.
/// On timeout the child is killed and a `Transport` error is returned.
fn output_with_timeout(mut cmd: Command, timeout: Duration) -> Result<std::process::Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| SubstrateError::Transport(format!("spawning claude: {e}")))?;

    // Take the pipes so the reader thread owns them.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel();

    let reader = thread::spawn(move || {
        use std::io::Read;
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut out);
        }
        if let Some(mut s) = stderr {
            let _ = s.read_to_end(&mut err);
        }
        let status = child.wait();
        let _ = tx.send((status, out, err));
    });

    match rx.recv_timeout(timeout) {
        Ok((status, out, err)) => {
            let _ = reader.join();
            let status = status
                .map_err(|e| SubstrateError::Transport(format!("waiting on claude: {e}")))?;
            Ok(std::process::Output {
                status,
                stdout: out,
                stderr: err,
            })
        }
        Err(_) => {
            // Timed out — best-effort: the child may still be running; the OS
            // reaps it when the reader thread's `child.wait()` returns. We
            // surface a timeout so the caller can proceed (v0: fail-open on the
            // introspection, never on the run itself).
            Err(SubstrateError::Transport(
                "claude call timed out".to_string(),
            ))
        }
    }
}

/// Run the zero-cost `/usage` introspection under an account.
/// `--no-session-persistence` keeps it from leaving a session file.
pub fn usage_snapshot(config_dir: Option<&Path>, timeout: Duration) -> Result<UsageSnapshot> {
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "/usage",
        "--output-format",
        "json",
        "--no-session-persistence",
    ]);
    apply_config_dir(&mut cmd, config_dir);
    let out = output_with_timeout(cmd, timeout)?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_usage_json(&text).map_err(SubstrateError::Engine)
}

/// Run `claude auth status` under an account.
pub fn auth_status(config_dir: Option<&Path>, timeout: Duration) -> Result<AuthStatus> {
    let mut cmd = Command::new("claude");
    cmd.args(["auth", "status"]);
    apply_config_dir(&mut cmd, config_dir);
    let out = output_with_timeout(cmd, timeout)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text
        .find('{')
        .ok_or_else(|| SubstrateError::Engine("no JSON in auth status output".into()))?;
    serde_json::from_str(text[start..].trim())
        .map_err(|e| SubstrateError::Engine(format!("parsing auth status: {e}")))
}

/// Escape a cwd into Claude Code's project-folder name (`cc-recon.md` §4):
/// every non-alphanumeric byte becomes `-`.
pub fn escaped_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The config-dir root for an account (`~/.claude` when the machine default).
pub fn config_root(config_dir: Option<&Path>) -> Result<PathBuf> {
    match config_dir {
        Some(d) => Ok(d.to_path_buf()),
        None => {
            let home = std::env::var("HOME")
                .map_err(|_| SubstrateError::Config("HOME not set".into()))?;
            Ok(PathBuf::from(home).join(".claude"))
        }
    }
}

/// Locate the transcript `.jsonl` for `cwd` under an account's config dir that
/// was modified at/after `since` (the newest match). Returns `None` if none.
pub fn find_transcript(
    config_dir: Option<&Path>,
    cwd: &Path,
    since: SystemTime,
) -> Result<Option<PathBuf>> {
    let root = config_root(config_dir)?
        .join("projects")
        .join(escaped_cwd(cwd));
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(SubstrateError::Io(e));
        }
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        // Allow a small clock slack before `since`.
        if modified + Duration::from_secs(2) < since {
            continue;
        }
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            best = Some((modified, path));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// Spawn `claude` transparently with all args passed verbatim and stdio
/// inherited. Returns the child's exit code (or 1 if killed by signal).
pub fn spawn_passthrough(config_dir: Option<&Path>, args: &[String]) -> Result<i32> {
    let mut cmd = Command::new("claude");
    cmd.args(args);
    apply_config_dir(&mut cmd, config_dir);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| SubstrateError::Transport(format!("spawning claude: {e}")))?;
    Ok(status.code().unwrap_or(1))
}

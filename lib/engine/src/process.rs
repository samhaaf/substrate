//! `LlamaProcess` — manages the llama-server child process.
//!
//! Responsibilities:
//! - Spawn llama-server with the correct CLI flags
//! - Poll `/health` until ready (via `LlamaClient`)
//! - Kill gracefully on explicit call or on drop (best-effort SIGTERM)
//!
//! ## Restart on crash
//!
//! `LlamaProcess` does not restart automatically. The `LlamaBackend` is responsible
//! for detecting a dead process (via `is_alive`) and re-spawning with exponential
//! backoff. This keeps restart policy out of the low-level process handle.

use std::path::{Path, PathBuf};

use substrate_types::{Result, SubstrateError};
use tokio::process::{Child, Command};

/// Full launch configuration for a llama-server process.
///
/// Used by [`BackendProvisioner::start`](crate::provision::BackendProvisioner::start)
/// to spawn the server with accelerator-aware flags. The simpler
/// [`LlamaProcess::start`] remains for the minimal model+port+slots case.
#[derive(Debug, Clone)]
pub struct LlamaProcessConfig {
    /// Path to the GGUF model weights file.
    pub model_path: PathBuf,
    /// HTTP port the server listens on.
    pub port: u16,
    /// Number of parallel completion slots (`--parallel`).
    pub n_parallel: u32,
    /// Layers to offload to the GPU (`--n-gpu-layers`). `-1` = all, `0` = CPU only.
    pub n_gpu_layers: i32,
    /// Context window size in tokens (`--ctx-size`).
    pub context_size: u32,
}

/// A live llama-server child process.
///
/// On drop, sends a best-effort SIGTERM to the child. Call [`LlamaProcess::stop`]
/// (consuming) or [`LlamaProcess::terminate`] (`&mut`) for a clean wait-for-exit.
pub struct LlamaProcess {
    child: Child,
    port: u16,
}

impl LlamaProcess {
    /// Spawn a new llama-server process.
    ///
    /// # Arguments
    ///
    /// - `bin_path`   — path to the `llama-server` binary
    /// - `model_path` — path to the GGUF model weights file
    /// - `port`       — HTTP port the server will listen on
    /// - `n_parallel` — number of concurrent completion slots (`--parallel`)
    pub async fn start(
        bin_path: &Path,
        model_path: &Path,
        port: u16,
        n_parallel: u32,
    ) -> Result<Self> {
        let child = Command::new(bin_path)
            .arg("--model")
            .arg(model_path)
            .arg("--port")
            .arg(port.to_string())
            .arg("--parallel")
            .arg(n_parallel.to_string())
            // Suppress output to avoid polluting the node daemon's stdout/stderr.
            // A future improvement could capture and forward to tracing.
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            // kill_on_drop ensures the OS reclaims the process even if we exit
            // uncleanly (panic, OOM killer, etc.)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                SubstrateError::Engine(format!(
                    "failed to spawn llama-server at {}: {e}",
                    bin_path.display()
                ))
            })?;

        tracing::info!(
            pid = child.id().unwrap_or(0),
            model = %model_path.display(),
            port,
            n_parallel,
            "spawned llama-server",
        );

        Ok(Self { child, port })
    }

    /// Spawn a llama-server process from a full [`LlamaProcessConfig`].
    ///
    /// Adds accelerator flags (`--n-gpu-layers`, `--ctx-size`) on top of the
    /// model / port / parallel args used by [`LlamaProcess::start`].
    pub async fn start_with_config(
        bin_path: &Path,
        config: &LlamaProcessConfig,
    ) -> Result<Self> {
        let child = Command::new(bin_path)
            .arg("--model")
            .arg(&config.model_path)
            .arg("--port")
            .arg(config.port.to_string())
            .arg("--parallel")
            .arg(config.n_parallel.to_string())
            .arg("--n-gpu-layers")
            .arg(config.n_gpu_layers.to_string())
            .arg("--ctx-size")
            .arg(config.context_size.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                SubstrateError::Engine(format!(
                    "failed to spawn llama-server at {}: {e}",
                    bin_path.display()
                ))
            })?;

        tracing::info!(
            pid = child.id().unwrap_or(0),
            model = %config.model_path.display(),
            port = config.port,
            n_parallel = config.n_parallel,
            n_gpu_layers = config.n_gpu_layers,
            context_size = config.context_size,
            "spawned llama-server (full config)",
        );

        Ok(Self {
            child,
            port: config.port,
        })
    }

    /// Stop the child process via `&mut` (does not consume `self`).
    ///
    /// Sends SIGKILL and waits for exit. Used by the provisioner's `stop`, where
    /// the process handle is owned elsewhere and only borrowed.
    pub async fn terminate(&mut self) -> Result<()> {
        self.child.kill().await.map_err(|e| {
            SubstrateError::Engine(format!("failed to kill llama-server: {e}"))
        })?;
        tracing::info!("llama-server process terminated");
        Ok(())
    }

    /// Stop the child process: send SIGTERM and wait for it to exit.
    ///
    /// After this call, the `LlamaProcess` is consumed. If you only need a
    /// best-effort kill (e.g., on drop), `kill_on_drop(true)` already covers that.
    pub async fn stop(mut self) -> Result<()> {
        self.child.kill().await.map_err(|e| {
            SubstrateError::Engine(format!("failed to kill llama-server: {e}"))
        })?;
        tracing::info!("llama-server process stopped");
        Ok(())
    }

    /// Returns `true` if the process is still running (has not exited).
    ///
    /// Uses a non-blocking `try_wait` so it is safe to call from sync context.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,   // still running
            Ok(Some(status)) => {
                tracing::warn!(exit_status = ?status, "llama-server exited unexpectedly");
                false
            }
            Err(e) => {
                tracing::error!(err = %e, "error polling llama-server process status");
                false
            }
        }
    }

    /// The port this process is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The OS process ID, if known.
    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }
}

impl Drop for LlamaProcess {
    fn drop(&mut self) {
        // Best-effort SIGTERM on drop. Errors are intentionally ignored here;
        // `kill_on_drop(true)` already handles this at the tokio level.
        let _ = self.child.start_kill();
    }
}

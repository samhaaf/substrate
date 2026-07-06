//! # substrate-engine
//!
//! Execution engine: owns the inference backend process, slot lifecycle,
//! and the `InferenceBackend` trait seam for future non-llama backends.
//!
//! ## Architecture
//!
//! The engine wraps an `InferenceBackend` implementation (default: `LlamaBackend`,
//! which wraps a `llama-server` child process). It manages:
//!
//! 1. **Process lifecycle** — spawn, health-check, kill, and restart on model swap
//! 2. **Slot management** — track in-flight completions, respect concurrency limits
//! 3. **Submission** — delegate to the backend's SSE streaming `run_completion`
//! 4. **Drain** — wait for all in-flight completions before a model swap
//!
//! ## Module Structure
//!
//! - [`backend`]  — `InferenceBackend` trait + `LlamaBackend` concrete impl
//!                  Stub seams: `VllmBackend`, `MlxBackend`, `RemoteApiBackend`
//! - [`process`]  — `LlamaProcess`: spawn/stop/is_alive
//! - [`client`]   — `LlamaClient`: HTTP client for llama-server API (health, completion, slot actions)
//! - [`slot`]     — `SlotTracker` + `SlotGuard`: RAII concurrency accounting
//!
//! ## What Is Implemented
//!
//! - Full `LlamaBackend`: process spawn, health-wait, SSE streaming, slot guard, metrics
//! - `SlotTracker` with atomic RAII guard
//! - `LlamaClient` with streaming and slot save/restore
//! - `LlamaProcess` with drop-kill and `is_alive` poll
//! - `ExecutionEngine::swap_model`, `submit`, `drain`, `cancel_all_running`, `abort_slot`
//!
//! ## What Remains Stubbed
//!
//! - `VllmBackend`, `MlxBackend`, `RemoteApiBackend` — future backends (all `todo!()`)
//! - KV cache prefix reuse (save/restore via `LlamaClient::slot_action`) — wired but not called
//!   by the scheduler yet

pub mod backend;
pub mod client;
pub mod process;
pub mod provision;
pub mod slot;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, RwLock};

use tokio::sync::broadcast;

use substrate_gc::GcService;
use substrate_store::Store;
use substrate_types::{
    CompletionId, CompletionMetrics, CompletionResult, CompletionRow, CompletionState,
    ErrorKind, LifecycleEvent, ModelId, Result, SubstrateError, StreamEvent, TerminationReason,
};

pub use backend::{
    CompletionPayload, CompletionResponse, InferenceBackend, LlamaBackend, LlamaConfig,
    MlxBackend, RemoteApiBackend, VllmBackend,
};
pub use client::LlamaClient;
pub use process::{LlamaProcess, LlamaProcessConfig};
pub use provision::{AssetMatcher, BackendProvisioner, Platform};
pub use slot::{SlotGuard, SlotTracker};

// ---------------------------------------------------------------------------
// EngineConfig
// ---------------------------------------------------------------------------

/// Configuration for the execution engine.
///
/// Passed to `ExecutionEngine::new`. The engine builds a `LlamaConfig` from
/// this at construction time.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Optional explicit path to a pre-installed llama-server binary.
    ///
    /// When empty, the engine auto-provisions the binary via
    /// [`BackendProvisioner`] using `llama_version` + `llama_data_dir`. When set
    /// to a non-empty path, that binary is used verbatim (provisioning skipped).
    pub llama_server_bin: String,

    /// Port for the llama-server HTTP API.
    pub llama_server_port: u16,

    /// Maximum number of slots (parallel completions) allowed.
    pub max_concurrent_completions: u32,

    /// Root data directory (for KV cache slot save/restore paths).
    pub data_dir: String,

    /// Root directory where model weight GGUF files are stored.
    pub model_weights_dir: String,

    /// llama.cpp release to provision: `"latest"` or a build tag like `"b4935"`.
    pub llama_version: String,

    /// Where to cache downloaded llama.cpp binaries
    /// (`{llama_data_dir}/backends/llama-{version}/`).
    pub llama_data_dir: PathBuf,

    /// GPU layer offload (`--n-gpu-layers`): `-1` = all on GPU, `0` = CPU only.
    pub n_gpu_layers: i32,

    /// Context window size in tokens (`--ctx-size`). Default 4096.
    pub context_size: u32,
}

// ---------------------------------------------------------------------------
// ExecutionEngine
// ---------------------------------------------------------------------------

/// The substrate execution engine.
///
/// Manages the lifecycle of an inference backend (default: `LlamaBackend`) and
/// dispatches completion work to it. One engine per substrate node.
///
/// **Key invariant:** llama-server loads exactly one model at startup.
/// `max_concurrent_completions` (slot count) is fixed at process launch time.
/// The server is never restarted to resize slots — that would drain the KV cache.
///
/// **Model swap = process restart.** To load a different model:
/// 1. Stop admitting new work (scheduler responsibility).
/// 2. Drain in-flight slots (or cancel them under memory pressure).
/// 3. Call `swap_model` — kills the old process and spawns a new one.
pub struct ExecutionEngine {
    #[allow(dead_code)] // retained for future restart-with-same-config and diagnostic use
    config: EngineConfig,
    store: Store,
    backend: Arc<dyn InferenceBackend>,
    /// Map from completion ID to the channel used to stream its token events.
    running: Arc<Mutex<HashMap<CompletionId, mpsc::Sender<StreamEvent>>>>,
    /// Currently loaded model (mirrors backend state for fast sync reads).
    resident_model: RwLock<Option<ModelId>>,
    /// Optional broadcast channel for lifecycle events.
    event_tx: Option<broadcast::Sender<LifecycleEvent>>,
}

impl ExecutionEngine {
    /// Create a new execution engine with the default `LlamaBackend`.
    ///
    /// **Auto-provisioning:** if `config.llama_server_bin` is empty, the engine
    /// resolves and (if needed) downloads the correct llama.cpp build for this
    /// platform via [`BackendProvisioner::ensure_binary`] before constructing the
    /// backend. If `llama_server_bin` is a non-empty path, that binary is used as-is
    /// and no network I/O occurs.
    ///
    /// Does NOT start the backend process — call `swap_model` to load the first model.
    /// (llama-server loads exactly one model, so the process is spawned lazily on the
    /// first model load, then restarted on each swap.)
    pub async fn new(config: EngineConfig, store: Store, gc: Arc<GcService>) -> Result<Self> {
        let bin_path = Self::resolve_binary(&config, gc).await?;

        let llama_config = LlamaConfig {
            bin_path,
            data_dir: config.data_dir.clone().into(),
            port: config.llama_server_port,
            n_parallel: config.max_concurrent_completions,
            model_weights_dir: config.model_weights_dir.clone().into(),
            n_gpu_layers: config.n_gpu_layers,
            context_size: config.context_size,
        };
        let backend = Arc::new(LlamaBackend::new(llama_config));
        Ok(Self::with_backend(config, store, backend))
    }

    /// Resolve the llama-server binary path: explicit override or auto-provision.
    async fn resolve_binary(config: &EngineConfig, gc: Arc<GcService>) -> Result<PathBuf> {
        if !config.llama_server_bin.trim().is_empty() {
            tracing::info!(
                bin = %config.llama_server_bin,
                "using explicit llama-server binary (provisioning skipped)"
            );
            return Ok(PathBuf::from(&config.llama_server_bin));
        }

        let provisioner = BackendProvisioner::new(gc)?;
        tracing::info!(
            platform = ?provisioner.platform(),
            version = %config.llama_version,
            "auto-provisioning llama-server binary"
        );
        provisioner
            .ensure_binary(&config.llama_data_dir, &config.llama_version)
            .await
    }

    /// Create an engine with a custom backend (for testing or alternative runtimes).
    pub fn with_backend(
        config: EngineConfig,
        store: Store,
        backend: Arc<dyn InferenceBackend>,
    ) -> Self {
        Self {
            config,
            store,
            backend,
            running: Arc::new(Mutex::new(HashMap::new())),
            resident_model: RwLock::new(None),
            event_tx: None,
        }
    }

    /// Attach a lifecycle event broadcast channel to this engine.
    ///
    /// Once set, the engine emits model lifecycle events (`ModelLoading`,
    /// `ModelLoaded`, `ModelUnloading`, `ModelUnloaded`, `ModelSwapping`) on this
    /// channel. If not set, event emission is a no-op.
    pub fn with_events(mut self, tx: broadcast::Sender<LifecycleEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Emit a lifecycle event, silently dropping it when there are no subscribers.
    fn emit(&self, event: LifecycleEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// The model currently loaded in the backend process, if any.
    pub fn resident_model(&self) -> Option<ModelId> {
        self.resident_model.try_read().ok().and_then(|g| g.clone())
    }

    /// Number of completions currently in-flight.
    pub fn running_count(&self) -> usize {
        self.running
            .try_lock()
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Drain and return all in-flight completion IDs (for crash recovery).
    ///
    /// Clears the in-flight map — callers should use the returned IDs to requeue
    /// the completions in the store.
    pub fn take_running_ids(&self) -> Vec<CompletionId> {
        match self.running.try_lock() {
            Ok(mut map) => {
                let ids: Vec<_> = map.keys().copied().collect();
                map.clear();
                ids
            }
            Err(_) => Vec::new(),
        }
    }

    /// Submit a completion for execution.
    ///
    /// The backend's `run_completion` is spawned as a background tokio task.
    /// Token events are forwarded to the provided `token_tx` channel.
    ///
    /// Precondition: the engine must have the correct model loaded.
    /// The scheduler ensures this via `swap_model` before calling `submit`.
    pub async fn submit(
        &self,
        row: CompletionRow,
        token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let id = row.id;
        let model_id = row.model_id.clone();

        // Build the completion payload from the row.
        let params: serde_json::Value = serde_json::from_str(&row.params_json)
            .unwrap_or(serde_json::Value::Object(Default::default()));

        let payload = CompletionPayload {
            prompt: row.prompt.clone(),
            n_predict: params.get("max_tokens").and_then(|v| v.as_u64()).map(|v| v as u32),
            temperature: params.get("temperature").and_then(|v| v.as_f64()).map(|v| v as f32),
            top_p: params.get("top_p").and_then(|v| v.as_f64()).map(|v| v as f32),
            top_k: params.get("top_k").and_then(|v| v.as_u64()).map(|v| v as u32),
            repeat_penalty: params.get("repeat_penalty").and_then(|v| v.as_f64()).map(|v| v as f32),
            stop: params.get("stop").and_then(|v| {
                v.as_array().map(|arr| {
                    arr.iter().filter_map(|s| s.as_str().map(String::from)).collect()
                })
            }),
            stream: true,
        };

        // Register the in-flight completion.
        {
            let mut map = self.running.lock().await;
            map.insert(id, token_tx.clone());
        }

        // Spawn the completion task.
        let backend = Arc::clone(&self.backend);
        let running = Arc::clone(&self.running);
        let store = self.store.clone();
        let tx_clone = token_tx.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let outcome = backend.run_completion(id, &payload, tx_clone).await;

            // Deregister from the in-flight map regardless of outcome.
            {
                let mut map = running.lock().await;
                map.remove(&id);
            }

            match outcome {
                Ok(result) => {
                    // Persist the result blob, then mark the completion terminal.
                    if let Err(e) = store.insert_result(&result) {
                        tracing::error!(completion_id = %id, err = %e, "failed to insert result");
                    }
                    let mark_fn = if result.state == CompletionState::Failed {
                        |s: &Store, i: CompletionId| s.mark_failed(i, "backend reported failure")
                    } else {
                        |s: &Store, i: CompletionId| s.mark_completed(i)
                    };
                    if let Err(e) = mark_fn(&store, id) {
                        tracing::error!(
                            completion_id = %id,
                            err = %e,
                            "failed to mark completion state in store"
                        );
                    }

                    // Publish a throughput sample for live dashboards, if the
                    // backend captured token/timing metrics for this run.
                    if result.state != CompletionState::Failed {
                        if let (Some(tx), Some(output_tokens), Some(tokens_per_second)) = (
                            event_tx.as_ref(),
                            result.metrics.completion_tokens,
                            result.metrics.tokens_per_second,
                        ) {
                            let _ = tx.send(LifecycleEvent::CompletionMetricsRecorded {
                                id,
                                model_id: model_id.clone(),
                                output_tokens,
                                tokens_per_second,
                            });
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(completion_id = %id, err = %e, "completion task failed");
                    // Build a minimal failed result so get_result returns something coherent.
                    let failed_result = CompletionResult {
                        id,
                        state: CompletionState::Failed,
                        text: None,
                        termination: Some(TerminationReason::Failed(
                            ErrorKind::TransportError(e.to_string()),
                        )),
                        metrics: CompletionMetrics::default(),
                        completed_at: chrono::Utc::now(),
                    };
                    if let Err(ie) = store.insert_result(&failed_result) {
                        tracing::error!(completion_id = %id, err = %ie, "failed to insert error result");
                    }
                    if let Err(ie) = store.mark_failed(id, &e.to_string()) {
                        tracing::error!(completion_id = %id, err = %ie, "failed to mark completion as failed");
                    }
                }
            }
        });

        Ok(())
    }

    /// Wait for all in-flight completions to finish.
    ///
    /// Polls every 50ms until `running_count() == 0` or `timeout_secs` elapses.
    pub async fn drain(&self, timeout_secs: u64) -> Result<()> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(timeout_secs);

        loop {
            if self.running_count() == 0 {
                tracing::debug!("drain: all slots empty");
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                let remaining = self.running_count();
                tracing::warn!(remaining, "drain: timed out with in-flight completions");
                return Err(SubstrateError::Engine(format!(
                    "drain timed out after {timeout_secs}s; {remaining} completions still running"
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Cancel all currently running completions by closing their token channels.
    ///
    /// Dropping the sender causes the receiver (WebSocket layer) to observe a
    /// closed stream. The background task will still run to completion but its
    /// token events will be discarded.
    pub async fn cancel_all_running(&self) -> Result<()> {
        let mut map = self.running.lock().await;
        let count = map.len();
        map.clear(); // Drops all senders → receivers see closed channel.
        tracing::info!(cancelled = count, "cancelled all running completions");
        Ok(())
    }

    /// Cancel a single in-flight completion by closing its token channel.
    pub async fn abort_slot(&self, id: CompletionId) -> Result<()> {
        let mut map = self.running.lock().await;
        if map.remove(&id).is_some() {
            tracing::info!(completion_id = %id, "aborted slot");
            Ok(())
        } else {
            Err(SubstrateError::CompletionNotFound(id))
        }
    }

    /// Swap the loaded model.
    ///
    /// Caller must drain or cancel in-flight work first (this method does NOT drain).
    /// Kills the existing backend process, spawns a new one with the given model,
    /// and waits for the health check to pass.
    pub async fn swap_model(&self, model_id: ModelId, model_path: &Path) -> Result<()> {
        let start = std::time::Instant::now();
        let path_str = model_path.display().to_string();

        tracing::info!(model_id, path = %path_str, "beginning model swap");

        // Emit swap-starting event (old → new).
        let from_model = self.resident_model.read().await.clone();
        if let Some(ref from) = from_model {
            self.emit(LifecycleEvent::ModelSwapping {
                from: from.clone(),
                to: model_id.clone(),
            });
            // Unload the current model first.
            self.emit(LifecycleEvent::ModelUnloading { model_id: from.clone() });
        }

        self.emit(LifecycleEvent::ModelLoading {
            model_id: model_id.clone(),
            path: path_str.clone(),
        });

        self.backend.load_model(&model_id, model_path).await?;

        {
            let mut guard = self.resident_model.write().await;
            if let Some(old) = guard.take() {
                self.emit(LifecycleEvent::ModelUnloaded { model_id: old });
            }
            *guard = Some(model_id.clone());
        }

        self.emit(LifecycleEvent::ModelLoaded {
            model_id: model_id.clone(),
            path: path_str,
        });

        let elapsed_ms = start.elapsed().as_millis();
        tracing::info!(model_id, elapsed_ms, "model swap complete");
        Ok(())
    }

    /// Unload the current model and stop the backend process.
    pub async fn unload(&self) -> Result<()> {
        let unloading_id = self.resident_model.read().await.clone();
        if let Some(ref model_id) = unloading_id {
            self.emit(LifecycleEvent::ModelUnloading { model_id: model_id.clone() });
        }
        self.backend.unload_model().await?;
        let mut guard = self.resident_model.write().await;
        *guard = None;
        if let Some(model_id) = unloading_id {
            self.emit(LifecycleEvent::ModelUnloaded { model_id });
        }
        Ok(())
    }

    /// Whether the backend process is currently healthy.
    pub async fn is_healthy(&self) -> bool {
        self.backend.is_healthy().await
    }
}

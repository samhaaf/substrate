//! `InferenceBackend` trait — the seam for non-llama inference runtimes.
//!
//! The default implementation is [`LlamaBackend`], which wraps a
//! [`LlamaProcess`](crate::process::LlamaProcess) child process and a
//! [`LlamaClient`](crate::client::LlamaClient) HTTP client.
//!
//! ## Future implementations
//!
//! - `VllmBackend`         — vLLM on Linux/CUDA (server-side batch scheduling)
//! - `MlxBackend`          — Apple MLX on Apple Silicon (native Metal, no llama-server)
//! - `RemoteApiBackend`    — proxy completions to OpenAI / Anthropic / any OpenAI-compat API
//!
//! Each future backend implements `InferenceBackend` without changing any other
//! substrate code. The `ExecutionEngine` accepts any `Box<dyn InferenceBackend>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono;
use serde::{Deserialize, Serialize};
use substrate_types::{
    CompletionId, CompletionMetrics, CompletionResult, CompletionState,
    ModelId, Result, StopReason, StreamEvent, SubstrateError, TerminationReason,
};
use tokio::sync::mpsc;

use crate::client::{LlamaClient, SseTokenEvent};
use crate::process::LlamaProcess;
use crate::slot::SlotTracker;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for creating a [`LlamaBackend`].
#[derive(Debug, Clone)]
pub struct LlamaConfig {
    /// Path to the `llama-server` binary.
    pub bin_path: PathBuf,

    /// Root data directory (used for KV cache paths and working files).
    pub data_dir: PathBuf,

    /// HTTP port llama-server will listen on.
    pub port: u16,

    /// Number of parallel completion slots (`--parallel N`).
    pub n_parallel: u32,

    /// Root directory where model weight files are stored.
    pub model_weights_dir: PathBuf,

    /// Layers to offload to the GPU (`--n-gpu-layers`). `-1` = all, `0` = CPU only.
    pub n_gpu_layers: i32,

    /// Context window size in tokens (`--ctx-size`).
    pub context_size: u32,
}

// ---------------------------------------------------------------------------
// Shared completion payload / response (backend-internal wire types)
// ---------------------------------------------------------------------------

/// Completion request payload forwarded verbatim to llama-server's `/completion` endpoint.
///
/// All fields map 1:1 to llama-server's API. Substrate does NOT normalize them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionPayload {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    pub stream: bool,
}

/// Non-streaming completion response from llama-server's `/completion` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub content: String,
    pub stop: bool,
    pub tokens_predicted: Option<u32>,
    pub tokens_evaluated: Option<u32>,
    pub generation_settings: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// InferenceBackend trait
// ---------------------------------------------------------------------------

/// Abstraction over an inference runtime.
///
/// All methods are `async` and the trait is object-safe via `async_trait`.
/// The engine stores `Box<dyn InferenceBackend>` so any concrete backend can be
/// swapped in without changing surrounding code.
///
/// # Implementing a new backend
///
/// 1. Create a new struct (e.g., `VllmBackend`, `MlxBackend`, `RemoteApiBackend`).
/// 2. Implement all methods — or use `todo!()` stubs for future functionality.
/// 3. Add a discriminant to `LlamaConfig` or create a parallel config type.
/// 4. Inject via `ExecutionEngine::new(config, store)` — no other changes needed.
///
/// # Future implementors
///
/// - `VllmBackend`      — vLLM subprocess on Linux/CUDA
/// - `MlxBackend`       — Apple MLX native on Apple Silicon
/// - `RemoteApiBackend` — HTTP proxy to OpenAI / Anthropic / Together / etc.
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Human-readable backend name (used in logs).
    fn name(&self) -> &str;

    /// Load the specified model. For process-based backends this spawns (or
    /// restarts) the server process. For API backends this may be a no-op.
    async fn load_model(&self, model_id: &ModelId, model_path: &Path) -> Result<()>;

    /// Unload the current model, stopping the backend process if applicable.
    async fn unload_model(&self) -> Result<()>;

    /// Run a completion, streaming token events into `token_tx`.
    ///
    /// Token events should be sent as `StreamEvent::Token { id, text, token_count }`.
    /// A terminal event (`StreamEvent::Completed` or `StreamEvent::Failed`) is
    /// sent as the last item before returning.
    ///
    /// Returns a [`CompletionResult`] summarising the outcome (text, metrics, etc.).
    async fn run_completion(
        &self,
        id: CompletionId,
        payload: &CompletionPayload,
        token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResult>;

    /// The model currently loaded, or `None` if no model is resident.
    async fn current_model(&self) -> Option<ModelId>;

    /// Maximum number of completions that can run simultaneously on this backend.
    fn max_concurrent_completions(&self) -> u32;

    /// True if the backend process / connection is healthy.
    async fn is_healthy(&self) -> bool;
}

// ---------------------------------------------------------------------------
// LlamaBackend
// ---------------------------------------------------------------------------

/// Inner state shared across async operations on the backend.
struct LlamaState {
    /// Running process handle, if a model is currently loaded.
    process: Option<LlamaProcess>,
    /// The model that is currently loaded (matches what the process was started with).
    resident_model: Option<ModelId>,
}

/// The default `InferenceBackend` implementation: wraps a `llama-server` subprocess.
///
/// A `LlamaBackend` owns a single `LlamaProcess` at a time. Loading a new model
/// kills the existing process and spawns a new one. The [`SlotTracker`] enforces
/// `n_parallel` as the admission ceiling.
pub struct LlamaBackend {
    config: LlamaConfig,
    client: LlamaClient,
    slots: SlotTracker,
    state: Arc<tokio::sync::Mutex<LlamaState>>,
}

impl LlamaBackend {
    /// Create a new `LlamaBackend`. Does not start the process yet;
    /// call `load_model` to spawn llama-server.
    pub fn new(config: LlamaConfig) -> Self {
        let port = config.port;
        let n_parallel = config.n_parallel;
        Self {
            client: LlamaClient::new(port),
            slots: SlotTracker::new(n_parallel),
            state: Arc::new(tokio::sync::Mutex::new(LlamaState {
                process: None,
                resident_model: None,
            })),
            config,
        }
    }

    /// Spawn the llama-server process for the given model path and wait until healthy.
    async fn spawn_and_wait(
        config: &LlamaConfig,
        client: &LlamaClient,
        model_path: &Path,
    ) -> Result<LlamaProcess> {
        let process = LlamaProcess::start_with_config(
            &config.bin_path,
            &crate::process::LlamaProcessConfig {
                model_path: model_path.to_path_buf(),
                port: config.port,
                n_parallel: config.n_parallel,
                n_gpu_layers: config.n_gpu_layers,
                context_size: config.context_size,
            },
        )
        .await?;

        // Wait up to 60 seconds for the server to become ready.
        client.wait_healthy(60).await?;

        Ok(process)
    }

    /// Parse a raw SSE line (`data: {...}`) into a `SseTokenEvent`.
    fn parse_sse_line(line: &[u8]) -> Option<SseTokenEvent> {
        let s = std::str::from_utf8(line).ok()?;
        let data = s.strip_prefix("data: ")?;
        // llama-server sends "[DONE]" as the last SSE message — skip it.
        if data.trim() == "[DONE]" {
            return None;
        }
        serde_json::from_str(data).ok()
    }
}

#[async_trait]
impl InferenceBackend for LlamaBackend {
    fn name(&self) -> &str {
        "llama-server"
    }

    async fn load_model(&self, model_id: &ModelId, model_path: &Path) -> Result<()> {
        let mut state = self.state.lock().await;

        // Kill any existing process before spawning a new one.
        if let Some(old_proc) = state.process.take() {
            tracing::info!(
                old_model = ?state.resident_model,
                "killing existing llama-server before model swap"
            );
            old_proc.stop().await?;
        }
        state.resident_model = None;

        // Spawn the new process.
        let process =
            Self::spawn_and_wait(&self.config, &self.client, model_path).await?;

        state.process = Some(process);
        state.resident_model = Some(model_id.clone());

        tracing::info!(model_id, "model loaded into llama-server");
        Ok(())
    }

    async fn unload_model(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(proc) = state.process.take() {
            proc.stop().await?;
            tracing::info!(model = ?state.resident_model, "model unloaded");
        }
        state.resident_model = None;
        Ok(())
    }

    async fn run_completion(
        &self,
        id: CompletionId,
        payload: &CompletionPayload,
        token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResult> {
        // Acquire a slot — returns None if at capacity.
        let _slot = self.slots.try_acquire(id).ok_or_else(|| {
            SubstrateError::Engine(format!(
                "all {} slots busy — cannot admit completion {id}",
                self.config.n_parallel
            ))
        })?;

        // Send the Started event.
        let _ = token_tx.send(StreamEvent::Started { id }).await;

        // Kick off the SSE stream.
        let mut stream = self.client.complete_stream(payload).await?;

        let mut full_text = String::new();
        let mut token_count: u32 = 0;
        let mut prompt_tokens: Option<u32> = None;
        let mut generation_ms: Option<u64> = None;
        let mut stop_reason = StopReason::Eos;

        let gen_start = std::time::Instant::now();

        use futures::StreamExt;
        while let Some(line_result) = stream.next().await {
            let line = match line_result {
                Ok(l) => l,
                Err(e) => {
                    let metrics = build_metrics(prompt_tokens, token_count, generation_ms);
                    let _ = token_tx
                        .send(StreamEvent::Failed {
                            id,
                            termination: TerminationReason::Failed(
                                substrate_types::ErrorKind::TransportError(e.to_string()),
                            ),
                            metrics: metrics.clone(),
                        })
                        .await;
                    return Ok(CompletionResult {
                        id,
                        state: CompletionState::Failed,
                        text: Some(full_text),
                        termination: Some(TerminationReason::Failed(
                            substrate_types::ErrorKind::TransportError(e.to_string()),
                        )),
                        metrics,
                        completed_at: chrono::Utc::now(),
                    });
                }
            };

            if let Some(event) = Self::parse_sse_line(&line) {
                full_text.push_str(&event.content);
                token_count += 1;

                // Forward token to the WebSocket layer.
                let _ = token_tx
                    .send(StreamEvent::Token {
                        id,
                        text: event.content.clone(),
                        token_count,
                    })
                    .await;

                if event.stop {
                    // Determine stop reason from timings / metadata.
                    if let Some(ref timings) = event.timings {
                        generation_ms = timings
                            .predicted_ms
                            .map(|ms| ms as u64);
                    }
                    prompt_tokens = event.tokens_evaluated;

                    // llama-server sets stop=true on EOS, stop sequences, or length.
                    // We treat the presence of a stop as EOS unless n_predict was hit.
                    stop_reason = if let Some(n_predict) = payload.n_predict {
                        if event.tokens_predicted.unwrap_or(0) >= n_predict {
                            StopReason::Length
                        } else {
                            StopReason::Eos
                        }
                    } else {
                        StopReason::Eos
                    };
                    break;
                }
            }
        }

        // Fallback generation time if timings were not in the final SSE event.
        if generation_ms.is_none() {
            generation_ms = Some(gen_start.elapsed().as_millis() as u64);
        }

        let metrics = build_metrics(prompt_tokens, token_count, generation_ms);
        let termination = TerminationReason::Completed(stop_reason);

        let _ = token_tx
            .send(StreamEvent::Completed {
                id,
                termination: termination.clone(),
                metrics: metrics.clone(),
            })
            .await;

        Ok(CompletionResult {
            id,
            state: CompletionState::Completed,
            text: Some(full_text),
            termination: Some(termination),
            metrics,
            completed_at: chrono::Utc::now(),
        })
        // `_slot` is dropped here, releasing the slot.
    }

    async fn current_model(&self) -> Option<ModelId> {
        self.state.lock().await.resident_model.clone()
    }

    fn max_concurrent_completions(&self) -> u32 {
        self.config.n_parallel
    }

    async fn is_healthy(&self) -> bool {
        self.client.health_check().await.is_ok()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_metrics(
    prompt_tokens: Option<u32>,
    completion_tokens: u32,
    generation_ms: Option<u64>,
) -> CompletionMetrics {
    let tokens_per_second = generation_ms.and_then(|ms| {
        if ms == 0 {
            None
        } else {
            Some(completion_tokens as f32 / (ms as f32 / 1000.0))
        }
    });

    CompletionMetrics {
        queue_latency_ms: None, // set by the scheduler layer, not the engine
        generation_ms,
        prompt_tokens,
        completion_tokens: Some(completion_tokens),
        tokens_per_second,
        preemption_count: 0,
        error_retry_count: 0,
    }
}

// ---------------------------------------------------------------------------
// Stub backends (future)
// ---------------------------------------------------------------------------

/// Stub backend for vLLM on Linux/CUDA.
///
/// # Future
///
/// Implement by spawning a `vllm serve` subprocess and proxying to its
/// OpenAI-compatible API. vLLM provides server-side continuous batching,
/// which may require changes to slot management semantics.
pub struct VllmBackend;

#[async_trait]
impl InferenceBackend for VllmBackend {
    fn name(&self) -> &str { "vllm" }
    async fn load_model(&self, _id: &ModelId, _path: &Path) -> Result<()> {
        todo!("VllmBackend::load_model")
    }
    async fn unload_model(&self) -> Result<()> {
        todo!("VllmBackend::unload_model")
    }
    async fn run_completion(
        &self, _id: CompletionId, _payload: &CompletionPayload,
        _token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResult> {
        todo!("VllmBackend::run_completion")
    }
    async fn current_model(&self) -> Option<ModelId> { todo!("VllmBackend::current_model") }
    fn max_concurrent_completions(&self) -> u32 { todo!("VllmBackend::max_concurrent_completions") }
    async fn is_healthy(&self) -> bool { todo!("VllmBackend::is_healthy") }
}

/// Stub backend for Apple MLX on Apple Silicon.
///
/// # Future
///
/// Implement by calling into `mlx_lm` Python (or a native Rust MLX binding)
/// directly — no subprocess server. This removes the HTTP roundtrip entirely
/// and could significantly reduce per-token latency on Apple Silicon.
pub struct MlxBackend;

#[async_trait]
impl InferenceBackend for MlxBackend {
    fn name(&self) -> &str { "mlx" }
    async fn load_model(&self, _id: &ModelId, _path: &Path) -> Result<()> {
        todo!("MlxBackend::load_model")
    }
    async fn unload_model(&self) -> Result<()> {
        todo!("MlxBackend::unload_model")
    }
    async fn run_completion(
        &self, _id: CompletionId, _payload: &CompletionPayload,
        _token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResult> {
        todo!("MlxBackend::run_completion")
    }
    async fn current_model(&self) -> Option<ModelId> { todo!("MlxBackend::current_model") }
    fn max_concurrent_completions(&self) -> u32 { todo!("MlxBackend::max_concurrent_completions") }
    async fn is_healthy(&self) -> bool { todo!("MlxBackend::is_healthy") }
}

/// Stub backend for proxying to a remote OpenAI-compatible API.
///
/// # Future
///
/// Implement by forwarding `CompletionPayload` to an external HTTP endpoint
/// (OpenAI, Anthropic, Together, Fireworks, etc.) and mapping the response back
/// to `StreamEvent`. Useful for comparison baselines and as a fallback when local
/// hardware is unavailable.
pub struct RemoteApiBackend;

#[async_trait]
impl InferenceBackend for RemoteApiBackend {
    fn name(&self) -> &str { "remote-api" }
    async fn load_model(&self, _id: &ModelId, _path: &Path) -> Result<()> {
        todo!("RemoteApiBackend::load_model — no-op for API backends")
    }
    async fn unload_model(&self) -> Result<()> {
        todo!("RemoteApiBackend::unload_model — no-op for API backends")
    }
    async fn run_completion(
        &self, _id: CompletionId, _payload: &CompletionPayload,
        _token_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResult> {
        todo!("RemoteApiBackend::run_completion — proxy to remote API")
    }
    async fn current_model(&self) -> Option<ModelId> { None }
    fn max_concurrent_completions(&self) -> u32 {
        todo!("RemoteApiBackend::max_concurrent_completions — depends on API rate limits")
    }
    async fn is_healthy(&self) -> bool {
        todo!("RemoteApiBackend::is_healthy — GET remote health endpoint")
    }
}

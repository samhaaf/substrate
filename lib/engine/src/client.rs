//! `LlamaClient` — HTTP client for the llama-server OpenAI-compatible API.
//!
//! Provides typed wrappers for:
//! - `GET  /health`            — liveness probe
//! - `POST /completion`        — non-streaming completion
//! - `POST /completion`        — streaming completion (SSE byte stream)
//! - `POST /slots/:id/action`  — KV cache slot save / restore
//!
//! ## SSE streaming
//!
//! `complete_stream` returns a `BoxStream<'static, Result<Vec<u8>>>` of raw SSE line
//! bytes. The caller (typically `LlamaBackend`) is responsible for parsing lines,
//! extracting the `data:` field, deserializing tokens, and forwarding them as
//! `StreamEvent`s.
//!
//! ## Error handling
//!
//! All non-2xx HTTP responses are converted to `SubstrateError::Engine`. Network
//! errors (connection refused, timeout) are wrapped as `SubstrateError::Transport`.

use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use substrate_types::{Result, SubstrateError};

use crate::backend::{CompletionPayload, CompletionResponse};

/// HTTP client for a single llama-server instance.
///
/// Cheap to clone (wraps `reqwest::Client` which is already `Arc`-backed).
#[derive(Clone)]
pub struct LlamaClient {
    client: Client,
    base_url: String,
}

/// Raw token event as returned by llama-server SSE stream.
///
/// Each SSE `data:` line deserializes into this struct. When `stop == true`
/// this is the final event — content may be empty.
#[derive(Debug, Deserialize)]
pub struct SseTokenEvent {
    pub content: String,
    pub stop: bool,
    pub tokens_predicted: Option<u32>,
    pub tokens_evaluated: Option<u32>,
    pub timings: Option<SseTimings>,
}

/// Timing breakdown included in the final SSE token event.
#[derive(Debug, Deserialize)]
pub struct SseTimings {
    pub prompt_n: Option<u32>,
    pub prompt_ms: Option<f64>,
    pub predicted_n: Option<u32>,
    pub predicted_ms: Option<f64>,
    pub predicted_per_second: Option<f64>,
}

/// Health check response from `GET /health`.
#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

/// Slot action request body.
#[derive(Debug, Serialize)]
struct SlotActionBody<'a> {
    action: &'a str,
    filename: &'a str,
}

impl LlamaClient {
    /// Create a new client pointed at `http://127.0.0.1:<port>`.
    pub fn new(port: u16) -> Self {
        Self {
            client: Client::builder()
                // No connection timeout — model loading takes many seconds.
                .build()
                .expect("reqwest client build should not fail"),
            base_url: format!("http://127.0.0.1:{port}"),
        }
    }

    /// `GET /health` — returns `true` if the server reports `status == "ok"`.
    ///
    /// Returns `Err(SubstrateError::Transport)` if the server is not reachable.
    /// Returns `Ok(false)` if the server responds but is not yet ready.
    pub async fn health_check(&self) -> Result<()> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("GET /health failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SubstrateError::Engine(format!(
                "GET /health returned {}",
                resp.status()
            )));
        }

        let body: HealthResponse = resp.json().await.map_err(|e| {
            SubstrateError::Engine(format!("GET /health: failed to parse body: {e}"))
        })?;

        if body.status != "ok" {
            return Err(SubstrateError::Engine(format!(
                "GET /health: server status = {:?} (expected \"ok\")",
                body.status
            )));
        }

        Ok(())
    }

    /// Poll `GET /health` until the server reports ready, with 1-second intervals.
    ///
    /// Returns `Err` if `max_attempts` is exhausted without a healthy response.
    pub async fn wait_healthy(&self, max_attempts: u32) -> Result<()> {
        for attempt in 1..=max_attempts {
            match self.health_check().await {
                Ok(()) => {
                    tracing::debug!(attempt, "llama-server health check passed");
                    return Ok(());
                }
                Err(e) => {
                    tracing::debug!(attempt, max_attempts, err = %e, "llama-server not ready yet");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Err(SubstrateError::Engine(format!(
            "llama-server did not become healthy after {max_attempts} attempts"
        )))
    }

    /// `POST /completion` — non-streaming completion.
    ///
    /// Blocks until the full response is received. Use `complete_stream` for
    /// interactive / low-latency use cases.
    pub async fn complete(&self, payload: &CompletionPayload) -> Result<CompletionResponse> {
        // Ensure stream flag is off for the non-streaming path.
        let mut p = payload.clone();
        p.stream = false;

        let resp = self
            .client
            .post(format!("{}/completion", self.base_url))
            .json(&p)
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("POST /completion failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SubstrateError::Engine(format!(
                "POST /completion returned {status}: {body}"
            )));
        }

        resp.json::<CompletionResponse>().await.map_err(|e| {
            SubstrateError::Engine(format!("POST /completion: failed to parse response: {e}"))
        })
    }

    /// `POST /completion` — streaming completion.
    ///
    /// Returns a stream of raw SSE line bytes. Each item is one complete line from the
    /// HTTP response body. The caller must parse `data: {...}` JSON envelopes and
    /// deserialize them as [`SseTokenEvent`].
    ///
    /// The stream ends when the underlying HTTP connection closes (which happens when
    /// llama-server sends the final event with `stop: true`).
    pub async fn complete_stream(
        &self,
        payload: &CompletionPayload,
    ) -> Result<impl futures::Stream<Item = Result<Vec<u8>>>> {
        let mut p = payload.clone();
        p.stream = true;

        let resp = self
            .client
            .post(format!("{}/completion", self.base_url))
            .json(&p)
            .send()
            .await
            .map_err(|e| SubstrateError::Transport(format!("POST /completion (stream) failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SubstrateError::Engine(format!(
                "POST /completion (stream) returned {status}: {body}"
            )));
        }

        // Convert the reqwest byte stream into a stream of lines.
        // Each chunk may contain partial lines; we buffer and split on '\n'.
        let byte_stream = resp.bytes_stream();
        let line_stream = byte_stream.scan(Vec::<u8>::new(), |buf, chunk| {
            let result = match chunk {
                Ok(bytes) => {
                    let mut lines: Vec<Result<Vec<u8>>> = Vec::new();
                    buf.extend_from_slice(&bytes);
                    // Drain all complete lines from buf.
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line = buf.drain(..=pos).collect::<Vec<u8>>();
                        // Trim trailing \r\n.
                        let trimmed = line
                            .strip_suffix(b"\r\n")
                            .or_else(|| line.strip_suffix(b"\n"))
                            .unwrap_or(&line)
                            .to_vec();
                        if !trimmed.is_empty() {
                            lines.push(Ok(trimmed));
                        }
                    }
                    Some(lines)
                }
                Err(e) => {
                    Some(vec![Err(SubstrateError::Transport(format!("stream error: {e}")))])
                }
            };
            futures::future::ready(result)
        });

        // Flatten the Vec<Result<Vec<u8>>> items into individual stream items.
        let flat = line_stream.flat_map(|lines| futures::stream::iter(lines));
        Ok(flat)
    }

    /// `POST /slots/:slot_id/action` — save or restore a KV cache slot.
    ///
    /// - `action`: `"save"` or `"restore"`
    /// - `path`:   file path for the slot state blob
    pub async fn slot_action(&self, slot_id: u32, action: &str, path: &str) -> Result<()> {
        let body = SlotActionBody { action, filename: path };

        let resp = self
            .client
            .post(format!("{}/slots/{slot_id}/action", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                SubstrateError::Transport(format!(
                    "POST /slots/{slot_id}/action ({action}) failed: {e}"
                ))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SubstrateError::Engine(format!(
                "POST /slots/{slot_id}/action ({action}) returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

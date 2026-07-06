//! `InferenceConfig` — parsed from `substrate.toml`.
//!
//! This is the v2 rename of v1's `SubstrateConfig`. Field names are identical
//! except where noted in the V1→V2 migration map.

use std::path::Path;

use serde::{Deserialize, Serialize};

use substrate_types::{ModelConfig, Result, SubstrateError};

/// Top-level inference configuration. Parsed from `substrate.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Root directory for all substrate data (DB, model weights, KV cache).
    /// Defaults to `~/.substrate`.
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    /// Explicit path to a pre-installed `llama-server` binary.
    ///
    /// Leave empty (the default) to auto-provision the correct llama.cpp build
    /// for this platform via the engine's `BackendProvisioner`.
    #[serde(default = "default_llama_server_bin")]
    pub llama_server_bin: String,

    /// llama.cpp release to provision when `llama_server_bin` is empty:
    /// `"latest"` or a specific build tag like `"b4935"`.
    #[serde(default = "default_llama_version")]
    pub llama_version: String,

    /// GPU layer offload for llama-server (`--n-gpu-layers`):
    /// `-1` = all layers on GPU, `0` = CPU only.
    #[serde(default = "default_n_gpu_layers")]
    pub n_gpu_layers: i32,

    /// Context window size in tokens (`--ctx-size`).
    #[serde(default = "default_context_size")]
    pub context_size: u32,

    /// Address to bind the node API server.
    /// Format: `"127.0.0.1:8420"` (localhost only) or `"0.0.0.0:8420"` (all interfaces).
    #[serde(default = "default_api_bind")]
    pub api_bind: String,

    /// Port for the internal llama-server process.
    /// Must not conflict with `api_bind`.
    #[serde(default = "default_llama_server_port")]
    pub llama_server_port: u16,

    /// Maximum number of completions to run in parallel.
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_completions: u32,

    /// Resource limits for disk usage.
    #[serde(default)]
    pub resource_limits: ResourceLimits,

    /// Default completion parameters (used when the request doesn't specify them).
    #[serde(default)]
    pub defaults: CompletionDefaults,

    /// Registered models. Each becomes a row in the models table.
    #[serde(default)]
    pub models: Vec<ModelConfig>,

    /// Telemetry polling interval in milliseconds.
    #[serde(default = "default_telemetry_poll_ms")]
    pub telemetry_poll_ms: u64,

    /// Hugging Face API token for downloading gated models.
    pub hf_token: Option<String>,
}

/// Resource budget limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum total bytes of model weight files on disk (default: 40 GB).
    #[serde(default = "default_weights_budget")]
    pub model_weights_budget_bytes: u64,

    /// Maximum total bytes of KV cache on disk (default: 10 GB).
    #[serde(default = "default_kv_budget")]
    pub kv_cache_budget_bytes: u64,

    /// Memory soft pressure threshold. Above this, admission is throttled.
    #[serde(default = "default_soft_pct")]
    pub memory_soft_pct: f32,

    /// Memory hard pressure threshold. Above this, no new work is admitted.
    #[serde(default = "default_hard_pct")]
    pub memory_hard_pct: f32,
}

/// Default values for completion parameters when not specified in the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionDefaults {
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    #[serde(default = "default_max_preemption_count")]
    pub max_preemption_count: u32,

    #[serde(default = "default_error_retry_limit")]
    pub error_retry_limit: u32,
}

impl InferenceConfig {
    /// Parse a `InferenceConfig` from a TOML file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SubstrateError::Config(format!("cannot read config: {e}")))?;
        toml::from_str(&content)
            .map_err(|e| SubstrateError::Config(format!("invalid config TOML: {e}")))
    }
}

// Default implementations.
impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            model_weights_budget_bytes: default_weights_budget(),
            kv_cache_budget_bytes: default_kv_budget(),
            memory_soft_pct: default_soft_pct(),
            memory_hard_pct: default_hard_pct(),
        }
    }
}

impl Default for CompletionDefaults {
    fn default() -> Self {
        Self {
            max_tokens: default_max_tokens(),
            max_preemption_count: default_max_preemption_count(),
            error_retry_limit: default_error_retry_limit(),
        }
    }
}

fn default_data_dir() -> String       { "~/.substrate".into() }
fn default_llama_server_bin() -> String { String::new() }
fn default_llama_version() -> String  { "latest".into() }
fn default_n_gpu_layers() -> i32      { -1 }
fn default_context_size() -> u32      { 4096 }
fn default_api_bind() -> String       { "127.0.0.1:8420".into() }
fn default_llama_server_port() -> u16 { 8421 }
fn default_max_concurrent() -> u32    { 4 }
fn default_weights_budget() -> u64    { 40 * 1024 * 1024 * 1024 }   // 40 GB
fn default_kv_budget() -> u64         { 10 * 1024 * 1024 * 1024 }   // 10 GB
fn default_soft_pct() -> f32          { 0.90 }
fn default_hard_pct() -> f32          { 0.95 }
fn default_max_tokens() -> u32        { 4096 }
fn default_max_preemption_count() -> u32 { 100 }
fn default_error_retry_limit() -> u32 { 3 }
fn default_telemetry_poll_ms() -> u64 { 1000 }

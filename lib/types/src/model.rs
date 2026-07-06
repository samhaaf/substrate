//! Model identity and configuration types.
//!
//! [`ModelId`] is the primary key used throughout substrate to refer to a model.
//! It is a human-readable string (e.g., `"qwen3.6-30b-a3b-q4"`) that maps to an
//! entry in the `[[models]]` section of `substrate.toml`.
//!
//! [`ModelConfig`] is the deserialized form of a `[[models]]` config entry.
//! It is the v2 equivalent of v1's `ModelConfig` in `config.rs`, but lives in
//! `substrate-types` so that downstream crates (`substrate-store`, `substrate-engine`)
//! can reference it without depending on `substrate-inference`.
//!
//! [`ModelRow`] is the persistence-facing view (includes download state, file path, etc.).
//! The store's `models` table mirrors this struct exactly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Identity ────────────────────────────────────────────────────────────

/// Human-readable model identifier. Corresponds to `[[models]].id` in `substrate.toml`.
///
/// Examples: `"qwen3.6-30b-a3b-q4"`, `"llama-3.1-8b-instruct-q8"`
///
/// This is a `String` rather than a `Uuid` because model names must be
/// human-readable in config files, log output, and API responses.
pub type ModelId = String;

// ── ModelConfig ─────────────────────────────────────────────────────────

/// A single model entry from the `[[models]]` section of `substrate.toml`.
///
/// This is the authoritative in-memory form of a model's static configuration.
/// It is stored in the `models` table via [`ModelRow`] at node startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Unique identifier for this model within this node.
    pub id: ModelId,

    /// Download source URI. Supported schemes:
    /// - `hf:owner/repo/filename.gguf`   — HuggingFace Hub
    /// - `https://...`                    — Direct URL
    /// - `file:///absolute/path.gguf`    — Local file (no download)
    pub source: String,

    /// Prompt template. Use `{prompt}` as the placeholder.
    ///
    /// Example: `"<|user|>\n{prompt}\n<|assistant|>\n"`
    ///
    /// Substrate expands this before sending the prompt to llama-server.
    /// Default: `"{prompt}"` (passthrough).
    #[serde(default = "default_prompt_template")]
    pub prompt_template: String,

    /// Context window length in tokens. Passed to llama-server as `--ctx-size`.
    #[serde(default = "default_context_length")]
    pub context_length: u32,

    /// Override the number of GPU layers to offload. `None` = llama-server auto.
    pub n_gpu_layers: Option<i32>,

    /// Max number of parallel completion slots for this model.
    /// `None` = use the node's `max_concurrent_completions` default.
    pub max_slots: Option<u32>,

    /// Expected GGUF file size in bytes (for download verification). Optional.
    pub expected_size_bytes: Option<u64>,

    /// SHA-256 hash of the GGUF file (hex string) for integrity verification. Optional.
    pub sha256: Option<String>,

    /// Additional raw arguments forwarded verbatim to llama-server (escape hatch).
    #[serde(default)]
    pub extra_args: Vec<String>,
}

fn default_prompt_template() -> String {
    "{prompt}".to_string()
}

fn default_context_length() -> u32 {
    8192
}

// ── ModelStatus ─────────────────────────────────────────────────────────

/// Runtime lifecycle state of a model on this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    /// The model weights file is not present on disk.
    Absent,
    /// The model is currently being downloaded.
    Downloading,
    /// The model weights are on disk and ready to load.
    Ready,
    /// The model is currently loaded in llama-server.
    Loaded,
    /// The model is in the process of being unloaded (llama-server restarting).
    Unloading,
}

// ── ModelRow ────────────────────────────────────────────────────────────

/// Persistence-facing model record (includes runtime state).
///
/// Stored in the `models` table. `substrate-store` reads and writes this struct.
/// Not serialized directly to external API clients.
#[derive(Debug, Clone)]
pub struct ModelRow {
    pub id: ModelId,
    pub source: String,
    pub prompt_template: String,
    pub context_length: u32,
    pub n_gpu_layers: Option<i32>,
    pub max_slots: Option<u32>,

    /// Whether the model weights file has been fully downloaded and verified.
    pub is_downloaded: bool,

    /// Whether the model is currently loaded in the engine (llama-server running it).
    pub is_loaded: bool,

    /// Absolute path to the GGUF file on disk. `None` if not yet downloaded.
    pub file_path: Option<String>,

    /// Size of the GGUF file in bytes. `None` if not yet known.
    pub file_bytes: Option<u64>,

    /// When the model was first registered in the store.
    pub registered_at: DateTime<Utc>,

    /// When the model was last successfully used for a completion (LRU eviction key).
    pub last_used_at: Option<DateTime<Utc>>,
}

impl ModelRow {
    /// Derive the current [`ModelStatus`] from the stored boolean flags.
    pub fn status(&self) -> ModelStatus {
        if self.is_loaded {
            ModelStatus::Loaded
        } else if self.is_downloaded {
            ModelStatus::Ready
        } else {
            ModelStatus::Absent
        }
    }
}

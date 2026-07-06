//! # substrate-models
//!
//! Model lifecycle management: registry sync from config, resumable download pipeline,
//! and disk-budget LRU eviction.
//!
//! ## Responsibilities
//!
//! 1. **Registry sync** — on startup, read `[[models]]` from config and upsert each
//!    into the store. Models not in config are retained (they may still have results).
//!
//! 2. **Download pipeline** — `ensure_available` checks if the model is on disk. If not,
//!    it kicks off a resumable download (supports `hf:`, `https://`, `file://` sources).
//!    Downloads stream to a `.partial` file and rename on completion.
//!
//! ## Module Structure
//!
//! - [`registry`]  — `ModelRegistry`: read-only view of registered models
//! - [`download`]  — download pipeline for `hf:`, `https://`, `file://` sources

pub mod registry;
pub mod download;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use substrate_gc::{EntryKind, GcService};
use substrate_store::Store;
use substrate_types::{ModelConfig, ModelId, ModelRow, Result};

use chrono::Utc;

// ---------------------------------------------------------------------------
// ModelManagerConfig
// ---------------------------------------------------------------------------

/// Configuration for the model manager.
#[derive(Debug, Clone)]
pub struct ModelManagerConfig {
    /// Root directory for model weight files.
    pub models_dir: String,

    /// Maximum total bytes of model weight files allowed on disk.
    pub model_weights_budget_bytes: u64,

    /// HuggingFace API token. Required for gated/private models.
    pub hf_token: Option<String>,
}

// ---------------------------------------------------------------------------
// ModelManager
// ---------------------------------------------------------------------------

/// Manages the full lifecycle of model weight files on this node.
///
/// Coordinates between the store (registry), download pipeline, and
/// disk-budget eviction. This is the single entry point for anything that
/// needs to touch model weight files.
pub struct ModelManager {
    config: ModelManagerConfig,
    store: Store,
    gc: Arc<GcService>,
}

impl ModelManager {
    /// Create a new `ModelManager`.
    pub fn new(config: ModelManagerConfig, store: Store, gc: Arc<GcService>) -> Self {
        Self { config, store, gc }
    }

    // ── Registry sync ────────────────────────────────────────────────────

    /// Sync the configured model list into the store.
    ///
    /// Called at startup. Upserts each [`ModelConfig`] entry into the store,
    /// preserving any existing runtime state (download status, file path, etc.).
    /// Does not remove models that were previously registered but are no longer
    /// in config — they may still have completion results attached.
    pub async fn sync_registry(&self, models: &[ModelConfig]) -> Result<()> {
        for cfg in models {
            let row = model_config_to_row(cfg);
            self.store.upsert_model(&row)?;
            tracing::debug!(model_id = %cfg.id, "synced model into registry");
        }
        Ok(())
    }

    // ── Availability ─────────────────────────────────────────────────────

    /// Ensure a model is downloaded and return its absolute file path.
    ///
    /// Workflow:
    /// 1. Look up the model in the store. Return `ModelNotFound` if absent.
    /// 2. If `file_path` is set and the file exists, touch `last_used_at` and return.
    /// 3. If `file_path` is set but the file is missing, clear the stale record.
    /// 4. Evict LRU unloaded models if needed to satisfy the disk budget.
    /// 5. Download the model weights (resumable).
    /// 6. Record the downloaded file path and size in the store.
    /// 7. Return the file path.
    pub async fn ensure_available(&self, model_id: &ModelId) -> Result<String> {
        let row = self.store.get_model(model_id)?;

        // Fast path: file is already on disk.
        if let Some(path) = &row.file_path {
            if Path::new(path).exists() {
                self.store.touch_model(model_id)?;
                return Ok(path.clone());
            }
            // File was recorded but is missing — clear the stale record.
            tracing::warn!(
                model_id = %model_id,
                "weight file missing from disk, clearing stale record: {path}"
            );
            self.store.clear_model_file(model_id)?;
        }

        // Make room via GC before downloading.
        let expected_bytes = row.file_bytes.unwrap_or(10 * 1024 * 1024 * 1024); // 10 GB estimate
        let weights_dir_str = self.config.models_dir.clone();
        let _ = self.gc.make_room(&weights_dir_str, expected_bytes);

        // Ensure the weights directory exists.
        let weights_dir = PathBuf::from(&self.config.models_dir);
        tokio::fs::create_dir_all(&weights_dir).await?;

        let dest = self.weights_path(model_id);

        tracing::info!(model_id = %model_id, dest = %dest.display(), "downloading model");

        let bytes = download::download_model(
            &row.source,
            &dest,
            self.config.hf_token.as_deref(),
        )
        .await?;

        let dest_str = dest.to_string_lossy().into_owned();
        self.store
            .set_model_downloaded(model_id, &dest_str, bytes)?;

        // Register with GC and lock for 24h while the model is available.
        let recovery_hint = row.source.clone();
        if let Err(e) = self.gc.register_entry(&dest, EntryKind::File, Some(recovery_hint)) {
            tracing::warn!("gc register_entry failed for {}: {e}", dest.display());
        }
        if let Err(e) = self.gc.lock(&dest_str, 86400) {
            tracing::warn!("gc lock failed for {dest_str}: {e}");
        }

        Ok(dest_str)
    }

    // ── Path helpers ─────────────────────────────────────────────────────

    /// Return the deterministic file path for a model's weight file.
    ///
    /// Path: `<models_dir>/<model_id>.gguf`
    ///
    /// The caller does not need to check whether this path exists — use
    /// [`ensure_available`] for that.
    pub fn weights_path(&self, model_id: &ModelId) -> PathBuf {
        PathBuf::from(&self.config.models_dir).join(format!("{model_id}.gguf"))
    }

    // ── Registry view ────────────────────────────────────────────────────

    /// Return a read-only view of the model registry.
    pub fn registry(&self) -> registry::ModelRegistry {
        registry::ModelRegistry::new(self.store.clone())
    }
}

// ---------------------------------------------------------------------------
// Helper: ModelConfig → ModelRow
// ---------------------------------------------------------------------------

/// Convert a config-file model entry into a store row suitable for upsert.
///
/// Runtime fields (is_downloaded, is_loaded, file_path, file_bytes, last_used_at)
/// are set to their "fresh" defaults. The upsert SQL preserves existing runtime
/// state on conflict, so these defaults are only used on first insert.
fn model_config_to_row(cfg: &ModelConfig) -> ModelRow {
    ModelRow {
        id: cfg.id.clone(),
        source: cfg.source.clone(),
        prompt_template: cfg.prompt_template.clone(),
        context_length: cfg.context_length,
        n_gpu_layers: cfg.n_gpu_layers,
        max_slots: cfg.max_slots,
        is_downloaded: false,
        is_loaded: false,
        file_path: None,
        file_bytes: cfg.expected_size_bytes,
        registered_at: Utc::now(),
        last_used_at: None,
    }
}

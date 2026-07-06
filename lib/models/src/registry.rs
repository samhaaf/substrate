//! `ModelRegistry` — read-only view of the registered model table.
//!
//! The registry is a thin wrapper around [`Store`] that exposes only the
//! model-read surface. All writes go through [`ModelManager`].

use substrate_store::Store;
use substrate_types::{ModelId, ModelRow, Result};

// ---------------------------------------------------------------------------
// ModelRegistry
// ---------------------------------------------------------------------------

/// A read-only view of the model registry backed by the store.
///
/// Cheap to clone — the underlying [`Store`] is already `Arc`-backed.
#[derive(Clone)]
pub struct ModelRegistry {
    store: Store,
}

impl ModelRegistry {
    /// Create a registry view over an existing store.
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Return all registered models, ordered by ID.
    pub fn list(&self) -> Result<Vec<ModelRow>> {
        self.store.list_models()
    }

    /// Fetch a single model by ID.
    ///
    /// Returns [`SubstrateError::ModelNotFound`] if the ID is not registered.
    pub fn get(&self, id: &ModelId) -> Result<ModelRow> {
        self.store.get_model(id)
    }

    /// Return only models that have been fully downloaded (is_downloaded = true).
    pub fn downloaded(&self) -> Result<Vec<ModelRow>> {
        Ok(self
            .store
            .list_models()?
            .into_iter()
            .filter(|m| m.is_downloaded)
            .collect())
    }

    /// Return the sum of all weight file sizes currently on disk, in bytes.
    pub fn total_weights_bytes(&self) -> Result<u64> {
        self.store.total_weights_bytes()
    }
}

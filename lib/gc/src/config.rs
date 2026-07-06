//! Config types and parsing for `.gc/` folder config files.
//!
//! Each managed directory has a `.gc/` subfolder containing:
//! - `config.toml` — the directory-level [`DirPolicy`]
//! - `{item_name}.toml` — per-item [`ItemConfig`] overrides (optional)

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// How items are chosen for eviction when a budget is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvictionPolicy {
    /// Least-recently-touched first.
    Lru,
    /// First-registered first.
    Fifo,
}

/// Whether the policy applies to the directory itself or to its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnitMode {
    /// The directory itself is the managed unit.
    Self_,
    /// Each child of the directory is a managed unit.
    Children,
}

/// What to do when a budget is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnFullAction {
    /// Evict (reclaim) LRU/FIFO items to make room.
    Evict,
    /// Migrate items elsewhere. Currently a stub that falls back to evict.
    Migrate,
}

const DEFAULT_MAX_SIZE_BYTES: u64 = 40 * 1024 * 1024 * 1024; // 40 GB
const DEFAULT_TTL_SECS: u64 = 7 * 24 * 60 * 60; // 7 days

/// Directory-level garbage-collection policy. Persisted as `.gc/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirPolicy {
    /// Maximum total size of managed entries in this directory. Default 40 GB.
    pub max_size_bytes: u64,
    /// Default TTL applied to entries without a per-item override. Default 7 days.
    pub default_ttl_secs: u64,
    /// Eviction ordering. Default [`EvictionPolicy::Lru`].
    pub eviction: EvictionPolicy,
    /// Managed unit mode. Default [`UnitMode::Children`].
    pub unit: UnitMode,
    /// Whether the policy applies recursively. Default false.
    pub recursive: bool,
    /// Action taken when the budget is exceeded. Default [`OnFullAction::Evict`].
    pub on_full: OnFullAction,
}

impl Default for DirPolicy {
    fn default() -> Self {
        Self {
            max_size_bytes: DEFAULT_MAX_SIZE_BYTES,
            default_ttl_secs: DEFAULT_TTL_SECS,
            eviction: EvictionPolicy::Lru,
            unit: UnitMode::Children,
            recursive: false,
            on_full: OnFullAction::Evict,
        }
    }
}

impl DirPolicy {
    /// Read `gc_dir/config.toml`. Returns [`DirPolicy::default`] if it does not exist.
    pub fn load(gc_dir: &Path) -> Result<Self> {
        let path = gc_dir.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading dir policy {}", path.display()))?;
        let policy: DirPolicy = toml::from_str(&text)
            .with_context(|| format!("parsing dir policy {}", path.display()))?;
        Ok(policy)
    }

    /// Write this policy to `gc_dir/config.toml`, creating `gc_dir` if needed.
    pub fn save(&self, gc_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(gc_dir)
            .with_context(|| format!("creating gc dir {}", gc_dir.display()))?;
        let path = gc_dir.join("config.toml");
        let text = toml::to_string_pretty(self).context("serializing dir policy")?;
        std::fs::write(&path, text)
            .with_context(|| format!("writing dir policy {}", path.display()))?;
        Ok(())
    }
}

/// Per-item config override. Persisted as `.gc/{item_name}.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemConfig {
    /// Per-item TTL override in seconds. Falls back to the dir default when `None`.
    pub ttl_secs: Option<u64>,
    /// Human-readable hint logged when this item is evicted.
    pub recovery_hint: Option<String>,
}

impl ItemConfig {
    /// Read `gc_dir/{item_name}.toml`. Returns `None` if it does not exist.
    pub fn load(gc_dir: &Path, item_name: &str) -> Result<Option<Self>> {
        let path = gc_dir.join(format!("{item_name}.toml"));
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading item config {}", path.display()))?;
        let cfg: ItemConfig = toml::from_str(&text)
            .with_context(|| format!("parsing item config {}", path.display()))?;
        Ok(Some(cfg))
    }

    /// Write this config to `gc_dir/{item_name}.toml`, creating `gc_dir` if needed.
    pub fn save(&self, gc_dir: &Path, item_name: &str) -> Result<()> {
        std::fs::create_dir_all(gc_dir)
            .with_context(|| format!("creating gc dir {}", gc_dir.display()))?;
        let path = gc_dir.join(format!("{item_name}.toml"));
        let text = toml::to_string_pretty(self).context("serializing item config")?;
        std::fs::write(&path, text)
            .with_context(|| format!("writing item config {}", path.display()))?;
        Ok(())
    }
}

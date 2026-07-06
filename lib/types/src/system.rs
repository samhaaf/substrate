//! System state snapshot and mesh node identity types.
//!
//! [`SystemState`] is the telemetry view of a single node, returned by
//! `/v1/system/state`. It captures resource utilization at a point in time
//! alongside the current scheduling state.
//!
//! [`NodeId`] and [`NodeInfo`] are used by the mesh layer to identify and
//! describe nodes in the substrate mesh network. These types live in
//! `substrate-types` so both `substrate-inference` and `substrate-mesh` can use
//! them without a circular dependency.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::ModelId;

// ── SystemState ─────────────────────────────────────────────────────────

/// A point-in-time snapshot of node resource utilization and queue state.
///
/// Returned by `/v1/system/state`. The mesh proxy may aggregate these across
/// nodes in a future capability (`/mesh/system/state`).
///
/// GPU fields return 0/0.0 until GPU telemetry is implemented (same as v1).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemState {
    /// When this snapshot was taken.
    pub sampled_at: DateTime<Utc>,

    /// Total system RAM in bytes.
    pub total_memory_bytes: u64,

    /// Currently used RAM in bytes.
    pub used_memory_bytes: u64,

    /// RAM utilisation as a fraction of total (0.0–1.0).
    ///
    /// Derived from `used_memory_bytes / total_memory_bytes`. Provided here for
    /// convenience so callers do not need to handle the division-by-zero edge case.
    pub memory_pressure: f32,

    /// CPU utilisation as a fraction (0.0–1.0), averaged across all cores.
    pub cpu_utilization: f32,

    /// GPU memory used in bytes. Returns 0 until GPU sampling is implemented.
    pub gpu_memory_used_bytes: u64,

    /// GPU memory total in bytes. Returns 0 until GPU sampling is implemented.
    pub gpu_memory_total_bytes: u64,

    /// GPU utilisation as a fraction (0.0–1.0). Returns 0.0 until GPU sampling
    /// is implemented.
    pub gpu_utilization: f32,

    /// Whether `gpu_utilization` is a real driver reading (`true`) or a stub
    /// zero (`false`). `true` on macOS when `ioreg` sampling succeeds.
    pub is_gpu_estimate: bool,

    /// The model currently loaded in the engine, if any.
    pub resident_model: Option<ModelId>,

    /// Number of completions currently executing on the engine.
    pub running_count: usize,

    /// Number of completions currently in the pending queue.
    pub pending_count: usize,

    /// Total bytes of model weights on disk (across all downloaded models).
    pub weights_on_disk_bytes: u64,

    /// Total bytes of KV-cache prefix files on disk.
    pub kv_cache_bytes: u64,
}

impl SystemState {
    /// Memory utilisation helper — same as `memory_pressure` but computed fresh
    /// from the raw counters, guarding against division by zero.
    pub fn memory_pct(&self) -> f32 {
        if self.total_memory_bytes == 0 {
            return 0.0;
        }
        self.used_memory_bytes as f32 / self.total_memory_bytes as f32
    }
}

// ── NodeId / NodeInfo ────────────────────────────────────────────────────

/// Stable identity for a substrate mesh node.
///
/// Conventionally the Tailscale device name (e.g., `"gpu-node-01"`), but can
/// be any unique string. Used as the routing key in `substrate-mesh`.
pub type NodeId = String;

/// Description of a node as seen by the mesh layer.
///
/// The mesh proxy tracks one `NodeInfo` per configured or discovered node.
/// Fields are intentionally minimal — the mesh layer communicates with nodes
/// only over HTTP and fills these fields from the node's health/state endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// The node's stable identity (Tailscale name or configured name).
    pub id: NodeId,

    /// Base URL of the node's substrate API (e.g., `"http://gpu-node-01:8420"`).
    pub api_url: String,

    /// True if this `NodeInfo` describes the process itself (i.e., the mesh proxy
    /// has a local node). Unused in the current single-node stub; reserved for
    /// future multi-node topologies.
    pub is_self: bool,

    /// Most recent system state snapshot for this node.
    ///
    /// `None` until the mesh proxy has successfully polled the node's
    /// `/v1/system/state` endpoint.
    pub last_state: Option<SystemState>,
}

impl NodeInfo {
    /// Create a new `NodeInfo` for a remote node (not self).
    pub fn remote(id: impl Into<NodeId>, api_url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            api_url: api_url.into(),
            is_self: false,
            last_state: None,
        }
    }
}

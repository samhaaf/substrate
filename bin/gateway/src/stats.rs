//! `GET /api/nodes/local/stats` — gateway-native stats endpoint.
//!
//! Returns disk usage (from the OS) and GPU utilization (proxied from the
//! inference service system state). The GC-managed bytes figure is obtained
//! by calling the GC service's `GET /dirs` endpoint and summing each
//! directory's actual `used_bytes` (bytes currently occupied by managed
//! entries) — not the `policy.max_size_bytes` budget ceiling, which can
//! exceed real usage and would make disk composition figures sum to over
//! 100%.
//!
//! Response JSON:
//! ```json
//! {
//!   "disk": {
//!     "total_bytes":      <u64>,
//!     "used_bytes":       <u64>,
//!     "free_bytes":       <u64>,
//!     "gc_managed_bytes": <u64>
//!   },
//!   "gpu": {
//!     "utilization_fraction": <f32>,
//!     "is_estimate":          <bool>
//!   }
//! }
//! ```

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use sysinfo::Disks;

use crate::state::GatewayState;

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /api/nodes/local/stats
pub async fn local_stats(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    // ── Disk stats (OS) ──────────────────────────────────────────────────────
    let (total_bytes, used_bytes, free_bytes) = disk_stats();

    // ── GC-managed bytes (via GC service /dirs) ───────────────────────────────
    let gc_managed_bytes = fetch_gc_managed_bytes(&state).await;

    // ── GPU info (via inference service /v1/system/state) ─────────────────────
    let (gpu_utilization_fraction, is_gpu_estimate) = fetch_gpu_info(&state).await;

    let body = serde_json::json!({
        "disk": {
            "total_bytes":      total_bytes,
            "used_bytes":       used_bytes,
            "free_bytes":       free_bytes,
            "gc_managed_bytes": gc_managed_bytes,
        },
        "gpu": {
            "utilization_fraction": gpu_utilization_fraction,
            "is_estimate":          is_gpu_estimate,
        }
    });

    (StatusCode::OK, Json(body))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read disk statistics for the filesystem that contains "/" (the root mount).
///
/// Returns `(total_bytes, used_bytes, free_bytes)`. Falls back to zeros on
/// any sysinfo error.
fn disk_stats() -> (u64, u64, u64) {
    let disks = Disks::new_with_refreshed_list();

    // Pick the disk whose mount point is "/" if present, otherwise use the
    // first disk found. This is a reasonable heuristic for single-disk machines.
    let best = disks
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"))
        .or_else(|| disks.iter().next());

    match best {
        Some(disk) => {
            let total = disk.total_space();
            let free = disk.available_space();
            let used = total.saturating_sub(free);
            (total, used, free)
        }
        None => (0, 0, 0),
    }
}

/// Call `GET {gc_url}/dirs` and sum each directory's `used_bytes` (actual
/// bytes occupied by managed entries) across all registered directories.
///
/// Returns 0 on any network or parsing error (degraded-mode fallback).
async fn fetch_gc_managed_bytes(state: &GatewayState) -> u64 {
    let gc_base = state.config.gc_url.trim_end_matches('/');
    let url = format!("{gc_base}/dirs");

    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("stats: failed to reach GC /dirs: {e}");
            return 0;
        }
    };

    if !resp.status().is_success() {
        tracing::debug!("stats: GC /dirs returned {}", resp.status());
        return 0;
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("stats: failed to parse GC /dirs JSON: {e}");
            return 0;
        }
    };

    // Each element has: { root, policy: { max_size_bytes, ... }, used_bytes, ... }
    // Sum used_bytes — actual occupied space, not the policy's budget ceiling.
    let dirs = match json.as_array() {
        Some(a) => a,
        None => return 0,
    };

    dirs.iter()
        .filter_map(|d| d.get("used_bytes").and_then(|v| v.as_u64()))
        .sum()
}

/// Fetch GPU utilization from the inference service's `/v1/system/state`.
///
/// Returns `(gpu_utilization, is_gpu_estimate)`. Degrades to `(0.0, false)`.
async fn fetch_gpu_info(state: &GatewayState) -> (f32, bool) {
    let inf_base = state.config.inference_url.trim_end_matches('/');
    let url = format!("{inf_base}/v1/system/state");

    let resp = match state.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("stats: failed to reach inference /v1/system/state: {e}");
            return (0.0, false);
        }
    };

    if !resp.status().is_success() {
        tracing::debug!(
            "stats: inference /v1/system/state returned {}",
            resp.status()
        );
        return (0.0, false);
    }

    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("stats: failed to parse system/state JSON: {e}");
            return (0.0, false);
        }
    };

    let utilization = json
        .get("gpu_utilization")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(0.0);

    let is_estimate = json
        .get("is_gpu_estimate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    (utilization, is_estimate)
}

//! `SystemSampler` — polls CPU, memory, and GPU utilization.
//!
//! CPU and memory are sampled via the `sysinfo` crate (cross-platform).
//! GPU utilization on macOS is sampled via `ioreg` (Apple Silicon "Core
//! Utilization" reporting); see `sample_gpu_macos` below. All other platforms
//! still stub GPU sampling at `(0.0, false)`.
//!
//! # Usage
//!
//! ```ignore
//! let (sampler, handle) = SystemSampler::start(Duration::from_secs(1));
//! let snap = sampler.snapshot();
//! ```
//!
//! # Future: GPU sampling
//! - Apple Silicon: done via `ioreg` (see `sample_gpu_macos`); a Metal/IOKit
//!   integration would give exact rather than estimated utilization.
//! - NVIDIA Linux: `nvml` crate (NVIDIA Management Library)
//! - AMD Linux: `rocm-smi` subprocess

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::System;
use tokio::task::JoinHandle;
use tracing::warn;

use substrate_types::SystemState;

// ---------------------------------------------------------------------------
// macOS GPU sampling via ioreg
// ---------------------------------------------------------------------------

/// Sample GPU Core Utilization on Apple Silicon via `ioreg`.
///
/// Returns `(utilization_0_to_1, is_estimate)`.
/// `is_estimate` is `true` when we successfully parsed a value from ioreg output.
/// Falls back to `(0.0, false)` on any error or missing data.
#[cfg(target_os = "macos")]
fn sample_gpu_macos() -> (f32, bool) {
    use std::process::Command;

    let output = match Command::new("ioreg")
        .args(["-r", "-c", "IOAccelerator"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return (0.0, false),
    };

    let stdout = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(_) => return (0.0, false),
    };

    // Look for lines containing "GPU Core Utilization" and parse the integer.
    // Typical line: `"GPU Core Utilization" = 42`
    // Multiple IOAccelerator entries are possible (e.g. integrated + eGPU) —
    // average across all readings rather than trusting whichever sorts first.
    let mut readings: Vec<u32> = Vec::new();
    for line in stdout.lines() {
        if line.contains("GPU Core Utilization") {
            if let Some(val_str) = line.split('=').last() {
                let trimmed = val_str.trim();
                if let Ok(pct) = trimmed.parse::<u32>() {
                    readings.push(pct);
                }
            }
        }
    }

    if readings.is_empty() {
        return (0.0, false);
    }

    let avg_pct = readings.iter().sum::<u32>() as f32 / readings.len() as f32;
    ((avg_pct / 100.0).clamp(0.0, 1.0), true)
}

/// Non-macOS stub — GPU sampling not implemented.
#[cfg(not(target_os = "macos"))]
fn sample_gpu_macos() -> (f32, bool) {
    (0.0, false)
}

/// Polls system resource utilization in the background.
///
/// Call [`SystemSampler::start`] to launch the background polling task.
/// Call [`SystemSampler::snapshot`] at any time to get the latest reading.
/// Gracefully degrades: if GPU info is unavailable, GPU fields are zero-filled.
pub struct SystemSampler {
    state: Mutex<SystemState>,
    sys: Mutex<System>,
}

impl SystemSampler {
    /// Spawn a background task that polls system resources every `interval`.
    ///
    /// Returns the shared sampler handle (for `snapshot()` calls) and the
    /// background task's join handle.
    pub fn start(interval: Duration) -> (Arc<SystemSampler>, JoinHandle<()>) {
        let mut sys = System::new_all();
        sys.refresh_all();

        let initial_state = Self::sample_sys(&mut sys);

        let sampler = Arc::new(SystemSampler {
            state: Mutex::new(initial_state),
            sys: Mutex::new(sys),
        });

        let sampler_clone = Arc::clone(&sampler);
        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                sampler_clone.poll();
            }
        });

        (sampler, handle)
    }

    /// Return the most recently sampled system state. Never blocks for more than
    /// the mutex acquisition — the lock is always held briefly.
    pub fn snapshot(&self) -> SystemState {
        self.state
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| {
                warn!("SystemSampler: state mutex poisoned — returning zeroed snapshot");
                zeroed_snapshot()
            })
    }

    /// Perform one sampling cycle and store the result internally.
    fn poll(&self) {
        let new_state = self
            .sys
            .lock()
            .map(|mut sys| Self::sample_sys(&mut sys))
            .unwrap_or_else(|_| {
                warn!("SystemSampler: sys mutex poisoned — skipping poll");
                zeroed_snapshot()
            });

        if let Ok(mut guard) = self.state.lock() {
            // Preserve fields that are populated externally (by the node layer).
            let prev_resident = guard.resident_model.clone();
            let prev_running = guard.running_count;
            let prev_pending = guard.pending_count;
            let prev_weights = guard.weights_on_disk_bytes;
            let prev_kv = guard.kv_cache_bytes;

            *guard = new_state;
            guard.resident_model = prev_resident;
            guard.running_count = prev_running;
            guard.pending_count = prev_pending;
            guard.weights_on_disk_bytes = prev_weights;
            guard.kv_cache_bytes = prev_kv;
        } else {
            warn!("SystemSampler: state mutex poisoned while writing — skipping update");
        }
    }

    /// Take a single reading from `sysinfo`. Pure function over the `System` handle.
    fn sample_sys(sys: &mut System) -> SystemState {
        sys.refresh_memory();
        sys.refresh_cpu_usage();

        let total = sys.total_memory();
        let used = sys.used_memory();
        let pressure = if total > 0 {
            used as f32 / total as f32
        } else {
            0.0
        };

        let cpus = sys.cpus();
        let cpu_util = if cpus.is_empty() {
            0.0f32
        } else {
            cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32 / 100.0
        };

        // GPU: try macOS ioreg first; fall back to stub zeros on other platforms.
        let (gpu_utilization, is_gpu_estimate) = sample_gpu_macos();

        SystemState {
            sampled_at: chrono::Utc::now(),
            total_memory_bytes: total,
            used_memory_bytes: used,
            memory_pressure: pressure,
            cpu_utilization: cpu_util,
            gpu_memory_used_bytes: 0,
            gpu_memory_total_bytes: 0,
            gpu_utilization,
            is_gpu_estimate,
            // These scheduling fields are populated by the node layer, not the sampler.
            resident_model: None,
            running_count: 0,
            pending_count: 0,
            weights_on_disk_bytes: 0,
            kv_cache_bytes: 0,
        }
    }
}

/// A zeroed `SystemState` used as a safe fallback when the sampler is in a
/// degraded state (e.g., mutex poisoned after a prior panic).
fn zeroed_snapshot() -> SystemState {
    SystemState {
        sampled_at: chrono::Utc::now(),
        total_memory_bytes: 0,
        used_memory_bytes: 0,
        memory_pressure: 0.0,
        cpu_utilization: 0.0,
        gpu_memory_used_bytes: 0,
        gpu_memory_total_bytes: 0,
        gpu_utilization: 0.0,
        is_gpu_estimate: false,
        resident_model: None,
        running_count: 0,
        pending_count: 0,
        weights_on_disk_bytes: 0,
        kv_cache_bytes: 0,
    }
}

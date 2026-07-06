//! Admission control: concurrency targeting and resource-pressure backpressure.
//!
//! The admission controller decides how many completions to admit on each scheduler tick.
//! It consults:
//! - The concurrency target (may be dynamic based on telemetry)
//! - Current memory pressure (hard/soft limits)
//! - The number of slots currently in use
//!
//! When memory pressure exceeds the soft limit, the concurrency target is reduced.
//! When pressure exceeds the hard limit, no new completions are admitted.

use substrate_types::SystemState;

/// Severity of current memory pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    /// Memory usage is below the soft threshold — normal operation.
    None,
    /// Memory usage is between the soft and hard thresholds.
    /// No new work should be admitted; in-flight work is allowed to finish.
    Soft,
    /// Memory usage is above the hard threshold.
    /// Running work should be preempted; no new admissions under any circumstances.
    Hard,
}

/// Controls admission of new work to the engine.
///
/// The controller applies two levers:
///
/// 1. **Concurrency target** — the configured upper bound on parallel completions.
///    May be reduced proportionally when memory is in the soft-pressure band.
///
/// 2. **Memory gates** — soft (no admission) and hard (preempt and cancel).
///
/// Call [`AdmissionController::slots_to_admit`] on each scheduler tick to learn
/// how many new completions can be dispatched.
pub struct AdmissionController {
    /// Configured maximum concurrency.
    max_concurrent: u32,

    /// Soft memory pressure threshold (0.0–1.0). Above this, target is reduced.
    soft_pct: f32,

    /// Hard memory pressure threshold (0.0–1.0). Above this, no admission.
    hard_pct: f32,
}

impl AdmissionController {
    /// Create an `AdmissionController` with default memory limits (soft: 0.90, hard: 0.95).
    pub fn new(max_concurrent: u32) -> Self {
        Self {
            max_concurrent,
            soft_pct: 0.90,
            hard_pct: 0.95,
        }
    }

    /// Override the memory soft/hard thresholds.
    pub fn with_memory_limits(mut self, soft_pct: f32, hard_pct: f32) -> Self {
        self.soft_pct = soft_pct;
        self.hard_pct = hard_pct;
        self
    }

    /// Classify current memory pressure from a system state snapshot.
    pub fn memory_pressure(&self, snapshot: &SystemState) -> MemoryPressure {
        let pct = snapshot.memory_pressure;
        if pct >= self.hard_pct {
            MemoryPressure::Hard
        } else if pct >= self.soft_pct {
            MemoryPressure::Soft
        } else {
            MemoryPressure::None
        }
    }

    /// Return true if a new completion can be admitted given current state.
    ///
    /// Convenience wrapper around [`slots_to_admit`] for single-admission decisions.
    pub fn can_admit(&self, snapshot: &SystemState, in_flight: u32) -> bool {
        self.slots_to_admit(snapshot, in_flight as usize) > 0
    }

    /// Compute how many additional completions to admit given current state.
    ///
    /// Returns 0 if under memory pressure or at the concurrency ceiling.
    /// Returns the gap between current running count and the effective concurrency
    /// target otherwise.
    pub fn slots_to_admit(&self, sys: &SystemState, running: usize) -> u32 {
        if sys.memory_pressure >= self.hard_pct {
            return 0;
        }
        let target = if sys.memory_pressure >= self.soft_pct {
            // Reduce target proportionally above soft limit.
            // Reduce target proportionally above soft limit.
            // Use round() rather than floor() to handle f32 precision loss:
            // values like 0.925_f32 are not exactly representable, so
            // the product can land just below an integer (e.g. 1.9999976
            // instead of 2.0) and floor() would return the wrong value.
            let range = self.hard_pct - self.soft_pct;
            let excess = sys.memory_pressure - self.soft_pct;
            let fraction_remaining = 1.0 - (excess / range);
            (self.max_concurrent as f32 * fraction_remaining).round() as u32
        } else {
            self.max_concurrent
        };

        target.saturating_sub(running as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use substrate_types::SystemState;

    fn state_with_pressure(pct: f32) -> SystemState {
        SystemState {
            memory_pressure: pct,
            ..Default::default()
        }
    }

    #[test]
    fn no_pressure_admits_up_to_max() {
        let ac = AdmissionController::new(4);
        let sys = state_with_pressure(0.5);
        assert_eq!(ac.slots_to_admit(&sys, 0), 4);
        assert_eq!(ac.slots_to_admit(&sys, 2), 2);
        assert_eq!(ac.slots_to_admit(&sys, 4), 0);
    }

    #[test]
    fn hard_pressure_admits_nothing() {
        let ac = AdmissionController::new(4);
        let sys = state_with_pressure(0.96);
        assert_eq!(ac.slots_to_admit(&sys, 0), 0);
    }

    #[test]
    fn soft_pressure_reduces_target() {
        let ac = AdmissionController::new(4);
        // At exactly the soft limit (0.90), fraction_remaining = 1.0, target = 4
        let sys = state_with_pressure(0.90);
        assert_eq!(ac.slots_to_admit(&sys, 0), 4);
        // Midway through the band: fraction_remaining = 0.5, target = floor(2.0) = 2
        let sys = state_with_pressure(0.925);
        assert_eq!(ac.slots_to_admit(&sys, 0), 2);
    }

    #[test]
    fn pressure_classification() {
        let ac = AdmissionController::new(4);
        assert_eq!(ac.memory_pressure(&state_with_pressure(0.5)), MemoryPressure::None);
        assert_eq!(ac.memory_pressure(&state_with_pressure(0.91)), MemoryPressure::Soft);
        assert_eq!(ac.memory_pressure(&state_with_pressure(0.96)), MemoryPressure::Hard);
    }
}

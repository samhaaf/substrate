//! Throughput estimation types.
//!
//! The estimator in `substrate-telemetry` fits a polynomial model to benchmark
//! results and can predict tokens/second at a given `(prompt_tokens, max_tokens,
//! concurrency)` operating point. Clients use these types to make scheduling
//! decisions and to set realistic timeout expectations.
//!
//! The estimator is exposed via the `/v1/estimate` REST endpoint and is used
//! internally by the scheduler's `suggested_concurrency` logic.

use serde::{Deserialize, Serialize};

use crate::model::ModelId;

// ── CompletionShape ──────────────────────────────────────────────────────

/// The "shape" of a completion: the parameters that primarily affect throughput.
///
/// This is the input to the estimator. All fields that influence tokens/second
/// prediction are captured here so that the estimator can look up the right
/// fitted polynomial.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionShape {
    /// The model to estimate throughput for.
    pub model_id: ModelId,

    /// Number of tokens in the prompt.
    pub prompt_tokens: u32,

    /// Maximum tokens to generate (used to estimate total wall time).
    pub max_tokens: u32,

    /// Concurrency level (how many completions are running in parallel).
    /// The estimator accounts for throughput degradation under concurrency.
    pub concurrency: u32,
}

// ── Estimate ─────────────────────────────────────────────────────────────

/// A throughput estimate for a given [`CompletionShape`].
///
/// Returned by `Estimator::estimate` and by the `/v1/estimate` REST endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estimate {
    /// The shape this estimate was computed for.
    pub shape: CompletionShape,

    /// Predicted tokens per second during generation.
    pub tokens_per_second: f32,

    /// Predicted total wall time in milliseconds
    /// (`max_tokens / tokens_per_second * 1000`).
    pub estimated_ms: u64,

    /// Confidence interval (low/high tps bounds). `None` if insufficient data.
    pub region: Option<EstimateRegion>,

    /// Warnings about the quality of this estimate.
    #[serde(default)]
    pub warnings: Vec<EstimateWarning>,

    /// True when no benchmark data exists for this model and conservative
    /// defaults were used instead of a fitted polynomial.
    pub cold_start: bool,
}

// ── EstimateRegion ───────────────────────────────────────────────────────

/// Confidence interval around a throughput estimate.
///
/// The bounds are expressed in tokens/second. Both `low_tps` and `high_tps`
/// represent 1-sigma bounds from the polynomial regression residuals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimateRegion {
    /// Lower bound on predicted tokens/second.
    pub low_tps: f32,
    /// Upper bound on predicted tokens/second.
    pub high_tps: f32,
}

// ── EstimateWarning ──────────────────────────────────────────────────────

/// A warning attached to an [`Estimate`] that indicates reduced confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateWarning {
    /// Fewer benchmark runs than desired — estimate may be extrapolating.
    InsufficientData {
        runs_available: u32,
        runs_recommended: u32,
    },

    /// The requested shape is outside the range of observed benchmark data.
    /// Polynomial extrapolation can be unreliable far from the fitted region.
    OutOfRange,

    /// The polynomial fit quality was poor (high residuals / low R²).
    PoorFit { r_squared: f32 },

    /// The estimator has no data for this model at all (cold start).
    ColdStart,
}

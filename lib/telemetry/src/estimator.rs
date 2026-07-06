//! Throughput estimator: polynomial regression over observed completions.
//!
//! [`ThroughputEstimator`] accumulates data points recorded from real completion
//! runs and fits a degree-2 polynomial surface over the feature space
//! `(input_tokens, output_tokens, parallelism) -> duration_ms`.
//!
//! ## Algorithm
//!
//! Feature vector (10 terms for a full degree-2 expansion over 3 variables):
//! ```text
//! [1, x1, x2, x3, x1², x2², x3², x1·x2, x1·x3, x2·x3]
//! ```
//! where `x1 = input_tokens`, `x2 = output_tokens`, `x3 = parallelism`.
//!
//! Coefficients are solved via ordinary least squares (SVD for numerical stability).
//!
//! ## Confidence
//!
//! - `Confidence::InEnvelope` — the query point lies within the convex hull of the
//!   training data's input/output/parallelism ranges. Prediction is interpolation.
//! - `Confidence::Extrapolated` — the query point is outside at least one observed
//!   dimension. Prediction is extrapolation; treat with caution.
//!
//! ## Cold-start behaviour
//!
//! Fewer than [`MIN_DATA_POINTS`] data points → returns a conservative default
//! estimate of [`COLD_START_MS_PER_OUTPUT_TOKEN`] ms per output token.

use std::sync::Mutex;

use nalgebra::{DMatrix, DVector};
use tracing::warn;

/// Minimum observations before fitting the polynomial.
///
/// A degree-2 model over 3 variables has 10 coefficients; we require at least
/// this many data points so the system is (just) over-determined.
const MIN_DATA_POINTS: usize = 10;

/// Number of features in the degree-2 polynomial over 3 variables.
/// [1, x1, x2, x3, x1², x2², x3², x1·x2, x1·x3, x2·x3]
const NUM_FEATURES: usize = 10;

/// Conservatively pessimistic default: 5 ms per output token (200 tok/s).
const COLD_START_MS_PER_OUTPUT_TOKEN: f64 = 5.0;

/// Conservative parallelism penalty multiplier applied on cold start.
const COLD_START_PARALLELISM_FACTOR: f64 = 0.8;

/// Minimum plausible ms estimate returned (clamp floor).
const MIN_ESTIMATE_MS: u64 = 1;

/// Confidence level attached to an [`EstimatedDuration`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    /// The query point is within the observed training envelope. Prediction is
    /// interpolation and should be reasonably accurate.
    InEnvelope,
    /// The query point is outside at least one dimension of observed data.
    /// Prediction is extrapolation; use with caution.
    Extrapolated,
}

/// A predicted completion duration with a confidence annotation.
#[derive(Debug, Clone)]
pub struct EstimatedDuration {
    /// Predicted wall-clock duration in milliseconds.
    pub millis: u64,
    /// Whether the prediction is an interpolation or extrapolation.
    pub confidence: Confidence,
}

/// One recorded observation fed to the estimator.
#[derive(Debug, Clone)]
struct DataPoint {
    input_tokens: u32,
    output_tokens: u32,
    parallelism: u32,
    duration_ms: u64,
}

/// Throughput estimator using polynomial regression over observed completions.
///
/// Not async — pure computation. Thread-safe via an internal `Mutex`.
pub struct ThroughputEstimator {
    data: Mutex<Vec<DataPoint>>,
}

impl ThroughputEstimator {
    /// Create a new, empty estimator.
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Vec::new()),
        }
    }

    /// Record a completed observation.
    ///
    /// All four parameters are required. `parallelism` is the number of
    /// concurrent completions running alongside this one (including itself),
    /// i.e. `1` means it ran alone.
    pub fn record(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        parallelism: u32,
        duration_ms: u64,
    ) {
        match self.data.lock() {
            Ok(mut guard) => {
                guard.push(DataPoint {
                    input_tokens,
                    output_tokens,
                    parallelism,
                    duration_ms,
                });
            }
            Err(_) => {
                warn!("ThroughputEstimator: data mutex poisoned — dropping observation");
            }
        }
    }

    /// Estimate the duration for a completion with the given parameters.
    ///
    /// Returns a conservative default with [`Confidence::Extrapolated`] when
    /// fewer than [`MIN_DATA_POINTS`] data points have been recorded.
    pub fn estimate(
        &self,
        input_tokens: u32,
        output_tokens: u32,
        parallelism: u32,
    ) -> EstimatedDuration {
        let data = match self.data.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                warn!("ThroughputEstimator: data mutex poisoned — returning cold-start estimate");
                return cold_start_estimate(output_tokens, parallelism);
            }
        };

        if data.len() < MIN_DATA_POINTS {
            return cold_start_estimate(output_tokens, parallelism);
        }

        // Compute observed ranges for envelope detection.
        let input_min = data.iter().map(|d| d.input_tokens).min().unwrap_or(0);
        let input_max = data.iter().map(|d| d.input_tokens).max().unwrap_or(0);
        let output_min = data.iter().map(|d| d.output_tokens).min().unwrap_or(0);
        let output_max = data.iter().map(|d| d.output_tokens).max().unwrap_or(0);
        let par_min = data.iter().map(|d| d.parallelism).min().unwrap_or(1);
        let par_max = data.iter().map(|d| d.parallelism).max().unwrap_or(1);

        let in_envelope = input_tokens >= input_min
            && input_tokens <= input_max
            && output_tokens >= output_min
            && output_tokens <= output_max
            && parallelism >= par_min
            && parallelism <= par_max;

        let confidence = if in_envelope {
            Confidence::InEnvelope
        } else {
            Confidence::Extrapolated
        };

        let n = data.len();
        let mut x_data = vec![0.0f64; n * NUM_FEATURES];
        let mut y_data = vec![0.0f64; n];

        for (i, dp) in data.iter().enumerate() {
            let x1 = dp.input_tokens as f64;
            let x2 = dp.output_tokens as f64;
            let x3 = dp.parallelism as f64;
            fill_features(&mut x_data[i * NUM_FEATURES..(i + 1) * NUM_FEATURES], x1, x2, x3);
            y_data[i] = dp.duration_ms as f64;
        }

        let coefficients = fit_polynomial(&x_data, &y_data, n);

        let x1 = input_tokens as f64;
        let x2 = output_tokens as f64;
        let x3 = parallelism as f64;
        let predicted = evaluate_polynomial(&coefficients, x1, x2, x3);

        let millis = (predicted.max(0.0) as u64).max(MIN_ESTIMATE_MS);

        EstimatedDuration { millis, confidence }
    }
}

impl Default for ThroughputEstimator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Cold-start estimate: pessimistic linear model, no training data.
fn cold_start_estimate(output_tokens: u32, parallelism: u32) -> EstimatedDuration {
    let par_penalty = COLD_START_PARALLELISM_FACTOR.powi(parallelism.saturating_sub(1) as i32);
    let millis = ((output_tokens as f64 * COLD_START_MS_PER_OUTPUT_TOKEN) / par_penalty) as u64;
    EstimatedDuration {
        millis: millis.max(MIN_ESTIMATE_MS),
        confidence: Confidence::Extrapolated,
    }
}

/// Fill a 10-element feature slice for degree-2 polynomial over (x1, x2, x3).
/// Order: [1, x1, x2, x3, x1², x2², x3², x1·x2, x1·x3, x2·x3]
fn fill_features(feat: &mut [f64], x1: f64, x2: f64, x3: f64) {
    debug_assert_eq!(feat.len(), NUM_FEATURES);
    feat[0] = 1.0;
    feat[1] = x1;
    feat[2] = x2;
    feat[3] = x3;
    feat[4] = x1 * x1;
    feat[5] = x2 * x2;
    feat[6] = x3 * x3;
    feat[7] = x1 * x2;
    feat[8] = x1 * x3;
    feat[9] = x2 * x3;
}

/// Solve least squares via SVD: coefficients = (XᵀX)⁻¹ Xᵀy
///
/// Returns a vector of NUM_FEATURES coefficients, or zeroes on failure.
fn fit_polynomial(x_data: &[f64], y_data: &[f64], n: usize) -> Vec<f64> {
    let x = DMatrix::from_row_slice(n, NUM_FEATURES, x_data);
    let y = DVector::from_column_slice(y_data);

    let xt = x.transpose();
    let xtx = &xt * &x;
    let xty = &xt * &y;

    // SVD for numerical stability; threshold weak singular values.
    let svd = xtx.svd(true, true);
    match svd.solve(&xty, 1e-10) {
        Ok(coeff_vec) => coeff_vec.as_slice().to_vec(),
        Err(e) => {
            warn!("ThroughputEstimator: SVD solve failed ({e}) — falling back to zeroes");
            vec![0.0; NUM_FEATURES]
        }
    }
}

/// Evaluate the degree-2 polynomial at point (x1, x2, x3).
fn evaluate_polynomial(coefficients: &[f64], x1: f64, x2: f64, x3: f64) -> f64 {
    if coefficients.len() < NUM_FEATURES {
        return 0.0;
    }
    coefficients[0]
        + coefficients[1] * x1
        + coefficients[2] * x2
        + coefficients[3] * x3
        + coefficients[4] * x1 * x1
        + coefficients[5] * x2 * x2
        + coefficients[6] * x3 * x3
        + coefficients[7] * x1 * x2
        + coefficients[8] * x1 * x3
        + coefficients[9] * x2 * x3
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_start_below_min_data_points() {
        let est = ThroughputEstimator::new();
        // Record only 5 observations — below MIN_DATA_POINTS.
        for i in 0..5u32 {
            est.record(100, 200, 1, 1000 + i as u64 * 10);
        }
        let result = est.estimate(100, 200, 1);
        assert_eq!(result.confidence, Confidence::Extrapolated);
        assert!(result.millis > 0);
    }

    #[test]
    fn in_envelope_after_sufficient_data() {
        let est = ThroughputEstimator::new();
        // Supply enough data points with a roughly linear relationship.
        for i in 0..15u32 {
            let output = 100 + i * 10;
            let dur = 500 + (i as u64 * 50);
            est.record(200, output, 1, dur);
        }
        // Query within the observed range.
        let result = est.estimate(200, 150, 1);
        assert_eq!(result.confidence, Confidence::InEnvelope);
        assert!(result.millis > 0);
    }

    #[test]
    fn extrapolated_outside_envelope() {
        let est = ThroughputEstimator::new();
        for i in 0..15u32 {
            let output = 100 + i * 10;
            let dur = 500 + (i as u64 * 50);
            est.record(200, output, 1, dur);
        }
        // Query with parallelism=4 — never seen.
        let result = est.estimate(200, 150, 4);
        assert_eq!(result.confidence, Confidence::Extrapolated);
    }

    #[test]
    fn cold_start_returns_nonzero() {
        let result = cold_start_estimate(512, 1);
        assert!(result.millis >= 1);
        assert_eq!(result.confidence, Confidence::Extrapolated);
    }
}

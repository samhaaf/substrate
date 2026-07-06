//! Benchmark suite definitions.
//!
//! A [`BenchmarkSuite`] describes the full grid of (output_length, parallelism,
//! input_length) cells to be measured. Each cell needs
//! [`BenchmarkSuite::target_samples_per_cell`] data points before it is
//! considered confident.

use chrono::Utc;
use substrate_store::{BenchmarkRun, Store};
use substrate_types::{
    CollectionId, CompletionRequest, MetricsFlags, ModelId, Result,
};

// ---------------------------------------------------------------------------
// Axis definitions
// ---------------------------------------------------------------------------

/// Output-length axis: max_tokens values used in the sweep grid.
pub const OUTPUT_LENGTHS: &[u32] = &[100, 1000, 10_000];

/// Parallelism axis: number of concurrent completions in the same measurement
/// window. (The orchestrator submits this many completions for a single cell.)
pub const PARALLELISM_LEVELS: &[u32] = &[1, 2, 4, 8];

/// Input-length axis: approximate prompt token counts.
pub const INPUT_LENGTHS: &[u32] = &[100, 1000, 10_000];

// ---------------------------------------------------------------------------
// SweepPoint
// ---------------------------------------------------------------------------

/// A single cell in the benchmark grid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SweepPoint {
    /// Max tokens to generate (output length axis).
    pub output_length: u32,
    /// Number of parallel completions to run (parallelism axis).
    pub parallelism: u32,
    /// Approximate number of prompt tokens (input length axis).
    pub input_length: u32,
}

// ---------------------------------------------------------------------------
// BenchmarkSuite
// ---------------------------------------------------------------------------

/// Orchestrator for a benchmark sweep on a single model.
///
/// Generates [`CompletionRequest`]s that cover the full
/// (output_length × parallelism × input_length) grid. Each request is
/// submitted at priority 0 inside a single collection with
/// `request_full_system: true`, so it only runs while the node is idle and
/// any real work immediately preempts it.
pub struct BenchmarkSuite {
    model_id: ModelId,
    store: Store,
}

impl BenchmarkSuite {
    /// Create a new suite for `model_id`, backed by `store` for existing-run
    /// lookups.
    pub fn new(model_id: ModelId, store: Store) -> Self {
        Self { model_id, store }
    }

    /// The default number of samples required per grid cell before that cell
    /// is considered confident.
    pub fn target_samples_per_cell() -> u32 {
        3
    }

    /// Generate all benchmark [`CompletionRequest`]s for a full sweep.
    ///
    /// Produces one request per (output_length × parallelism × input_length)
    /// combination. All requests:
    /// - are at priority 0
    /// - carry `MetricsFlags::ALL` so timing and token counts are captured
    /// - share the supplied `collection_id`
    pub fn plan_completions(&self, collection_id: CollectionId) -> Vec<CompletionRequest> {
        let mut requests = Vec::new();

        for &output_length in OUTPUT_LENGTHS {
            for &parallelism in PARALLELISM_LEVELS {
                for &input_length in INPUT_LENGTHS {
                    // Repeat `parallelism` times so the engine runs that many
                    // concurrent completions for this cell.
                    for _ in 0..parallelism {
                        requests.push(CompletionRequest {
                            id: CompletionRequest::new_id(),
                            model_id: self.model_id.clone(),
                            prompt: generate_benchmark_prompt(input_length),
                            max_tokens: Some(output_length),
                            temperature: Some(0.0),
                            top_p: None,
                            top_k: None,
                            repeat_penalty: None,
                            stop: None,
                            json_schema: None,
                            priority: 0,
                            preemption_threshold: Some(1),
                            collection_id: Some(collection_id),
                            metrics: MetricsFlags::ALL,
                            metadata: Some(serde_json::json!({
                                "benchmark": true,
                                "cell": {
                                    "output_length": output_length,
                                    "parallelism": parallelism,
                                    "input_length": input_length,
                                }
                            })),
                            created_at: Utc::now(),
                        });
                    }
                }
            }
        }

        requests
    }

    /// Return completion requests only for cells that still need more samples.
    ///
    /// Queries `store` for existing benchmark runs for this model and returns
    /// requests for each (output_length, parallelism, input_length) cell that
    /// has fewer than [`Self::target_samples_per_cell`] completed runs.
    pub fn missing_samples(
        &self,
        collection_id: CollectionId,
        existing: &[BenchmarkRun],
    ) -> Result<Vec<CompletionRequest>> {
        let target = Self::target_samples_per_cell();
        let mut requests = Vec::new();

        for &output_length in OUTPUT_LENGTHS {
            for &parallelism in PARALLELISM_LEVELS {
                for &input_length in INPUT_LENGTHS {
                    let count = existing
                        .iter()
                        .filter(|r| {
                            r.max_tokens == output_length
                                && r.concurrency == parallelism
                                && r.prompt_tokens == input_length
                                && r.tokens_per_second.is_some()
                        })
                        .count() as u32;

                    if count < target {
                        // This cell still needs at least one more sample.
                        // Submit `parallelism` copies so the measurement window
                        // sees the correct concurrency level for this cell.
                        for _ in 0..parallelism {
                            requests.push(CompletionRequest {
                                id: CompletionRequest::new_id(),
                                model_id: self.model_id.clone(),
                                prompt: generate_benchmark_prompt(input_length),
                                max_tokens: Some(output_length),
                                temperature: Some(0.0),
                                top_p: None,
                                top_k: None,
                                repeat_penalty: None,
                                stop: None,
                                json_schema: None,
                                priority: 0,
                                preemption_threshold: Some(1),
                                collection_id: Some(collection_id),
                                metrics: MetricsFlags::ALL,
                                metadata: Some(serde_json::json!({
                                    "benchmark": true,
                                    "cell": {
                                        "output_length": output_length,
                                        "parallelism": parallelism,
                                        "input_length": input_length,
                                    }
                                })),
                                created_at: Utc::now(),
                            });
                        }
                    }
                }
            }
        }

        Ok(requests)
    }

    /// Query the store for all benchmark runs for this model.
    pub fn existing_runs(&self) -> Result<Vec<BenchmarkRun>> {
        self.store.benchmark_runs_for_model(&self.model_id)
    }
}

// ---------------------------------------------------------------------------
// Prompt generation
// ---------------------------------------------------------------------------

/// Generate a synthetic prompt of approximately `target_tokens` tokens.
///
/// Uses repeated English filler text. The actual token count depends on the
/// model's tokenizer, but at roughly 1 token ≈ 4 chars for English prose this
/// is close enough for benchmark purposes.
pub fn generate_benchmark_prompt(target_tokens: u32) -> String {
    let target_chars = (target_tokens as usize).saturating_mul(4);
    let filler = "The quick brown fox jumps over the lazy dog. ";
    if target_chars == 0 {
        return String::new();
    }
    let mut prompt = String::with_capacity(target_chars + filler.len());
    while prompt.len() < target_chars {
        prompt.push_str(filler);
    }
    prompt.truncate(target_chars);
    prompt
}

// ---------------------------------------------------------------------------
// Public re-export of the default sweep helper (used by orchestrator)
// ---------------------------------------------------------------------------

/// Return the full set of unique grid cells for a standard benchmark sweep.
pub fn all_cells() -> Vec<SweepPoint> {
    let mut cells = Vec::new();
    for &output_length in OUTPUT_LENGTHS {
        for &parallelism in PARALLELISM_LEVELS {
            for &input_length in INPUT_LENGTHS {
                cells.push(SweepPoint {
                    output_length,
                    parallelism,
                    input_length,
                });
            }
        }
    }
    cells
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_prompt_length() {
        let p = generate_benchmark_prompt(100);
        // Should be approximately 400 chars, at least 380 and no more than 420.
        assert!(p.len() >= 380, "prompt too short: {}", p.len());
        assert!(p.len() <= 420, "prompt too long: {}", p.len());
    }

    #[test]
    fn generate_prompt_zero() {
        let p = generate_benchmark_prompt(0);
        assert!(p.is_empty());
    }

    #[test]
    fn all_cells_count() {
        let cells = all_cells();
        assert_eq!(
            cells.len(),
            OUTPUT_LENGTHS.len() * PARALLELISM_LEVELS.len() * INPUT_LENGTHS.len()
        );
    }

    #[test]
    fn plan_completions_count() {
        let store = substrate_store::Store::open_in_memory().unwrap();
        let suite = BenchmarkSuite::new("test-model".to_string(), store);
        let col_id = uuid::Uuid::new_v4();
        let reqs = suite.plan_completions(col_id);

        // Each cell gets `parallelism` requests; sum = Σ parallelism across all cells
        let expected: u32 = OUTPUT_LENGTHS
            .iter()
            .flat_map(|_| PARALLELISM_LEVELS.iter())
            .flat_map(|&p| INPUT_LENGTHS.iter().map(move |_| p))
            .sum();
        assert_eq!(reqs.len() as u32, expected);
    }

    #[test]
    fn missing_samples_all_missing() {
        let store = substrate_store::Store::open_in_memory().unwrap();
        let suite = BenchmarkSuite::new("test-model".to_string(), store);
        let col_id = uuid::Uuid::new_v4();
        let missing = suite.missing_samples(col_id, &[]).unwrap();

        // With no existing runs, missing_samples should match plan_completions
        let plan = suite.plan_completions(col_id);
        assert_eq!(missing.len(), plan.len());
    }
}

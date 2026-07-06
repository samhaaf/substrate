//! `LoadBalancer` trait — selects which node handles a given completion request.
//!
//! ## Trait Contract
//!
//! Given the full `CompletionRequest` and a slice of available `NodeEndpoint`s,
//! `select` returns the endpoint that should handle this request. Returns an error
//! if no nodes are available.
//!
//! ## Implementations
//!
//! - [`PassthroughBalancer`] — always returns the first (only) node. The v2 stub.
//!   Functional for single-node deployments.
//!
//! ## Future Implementations
//!
//! - `ModelAffinityBalancer` — route to the node that already has the requested
//!   model loaded, to avoid swap cost. Queries node `/v1/system/state`.
//! - `LeastLoadedBalancer` — route to the node with the fewest running completions.
//! - `EstimatorAwareBalancer` — use per-node estimator data to route to the node
//!   that will complete the request fastest.
//! - `RoundRobinBalancer` — simple round-robin across all nodes.

use substrate_types::{Result, SubstrateError};

use crate::discovery::NodeEndpoint;

/// The request context passed to the balancer for routing decisions.
/// This is intentionally minimal — the balancer should not need full request details.
#[derive(Debug, Clone)]
pub struct RoutingContext {
    pub model_id: String,
    pub prompt_token_estimate: Option<u32>,
}

/// Selects which node should handle a given request.
pub trait LoadBalancer: Send + Sync {
    /// Choose a node from `available` for the given routing context.
    /// Returns an error if `available` is empty.
    fn select(
        &self,
        context: &RoutingContext,
        available: &[NodeEndpoint],
    ) -> Result<NodeEndpoint>;
}

/// Always returns the first configured node. The v2 functional stub.
///
/// This is the production implementation for single-node deployments.
/// No routing logic needed when there is only one node.
pub struct PassthroughBalancer;

impl LoadBalancer for PassthroughBalancer {
    fn select(
        &self,
        _context: &RoutingContext,
        available: &[NodeEndpoint],
    ) -> Result<NodeEndpoint> {
        available.first().cloned().ok_or_else(|| {
            SubstrateError::Internal("no nodes available in PassthroughBalancer".into())
        })
    }
}

/// Stub: round-robin across all available nodes.
pub struct RoundRobinBalancer {
    counter: std::sync::atomic::AtomicUsize,
}

impl RoundRobinBalancer {
    pub fn new() -> Self {
        Self { counter: std::sync::atomic::AtomicUsize::new(0) }
    }
}

impl LoadBalancer for RoundRobinBalancer {
    fn select(
        &self,
        _context: &RoutingContext,
        available: &[NodeEndpoint],
    ) -> Result<NodeEndpoint> {
        if available.is_empty() {
            return Err(SubstrateError::Internal("no nodes available".into()));
        }
        let idx = self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % available.len();
        Ok(available[idx].clone())
    }
}

/// Stub: routes to the node that already has the target model loaded.
///
/// # Future
/// Query each node's `/v1/system/state` to find which has the model as `resident_model`.
/// Fall back to `PassthroughBalancer` if no node has it loaded.
pub struct ModelAffinityBalancer;

impl LoadBalancer for ModelAffinityBalancer {
    fn select(
        &self,
        _context: &RoutingContext,
        available: &[NodeEndpoint],
    ) -> Result<NodeEndpoint> {
        todo!(
            "Query each node's /v1/system/state, find node where resident_model == context.model_id, \
             fall back to first available"
        )
    }
}

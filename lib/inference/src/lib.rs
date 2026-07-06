//! # substrate-inference
//!
//! Inference service core: wires all subsystems into a single-machine inference runtime.
//!
//! This is the v2 successor to the v1 `Substrate` struct. The rename reflects the
//! two-tier architecture: "InferenceService" is the per-machine layer; "Mesh" is the routing layer.
//!
//! ## Initialization Sequence (`InferenceService::start`)
//!
//! 1. Parse `InferenceConfig` from `substrate.toml`
//! 2. Open SQLite store
//! 3. Crash recovery: requeue any completions stuck in Running state
//! 4. Start telemetry polling (background)
//! 5. Sync model registry from config
//! 6. Initialize model manager (download pipeline + eviction)
//! 7. Initialize KV cache manager
//! 8. Initialize execution engine (llama-server not started until first completion)
//! 9. Initialize scheduler (with default swap evaluator + selection policy)
//! 10. Start benchmark orchestrator (background)
//! 11. Start scheduler loop (background)
//! 12. Return `InferenceService` — caller calls `InferenceService::serve()` to bind the API server
//!
//! ## In-Process API
//!
//! In addition to the HTTP API, `InferenceService` exposes a direct Rust API for embedding:
//! - `submit(req)` — returns a `Promise` that resolves on completion
//! - `cancel(id)` — cancel a pending or running completion
//! - `system_state()` — current telemetry snapshot
//! - `resident_model()` — which model is loaded in the engine
//! - `ensure_model(id)` — trigger model download
//!
//! ## Prompt Hook Seam
//!
//! `InferenceService` holds an `Option<Arc<dyn PromptHook>>`. When set, the hook transforms
//! `CompletionRequest.prompt` before submission (e.g., for roll-up expansion).
//! Default: None (no-op).

pub mod config;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use tokio::sync::{broadcast, Mutex};

use substrate_api::ApiState;
use substrate_benchmark::{BenchmarkConfig, BenchmarkOrchestrator};
use substrate_cache::{CacheConfig, CacheManager};
use substrate_engine::{EngineConfig, ExecutionEngine};
use substrate_gc::{GcConfig, GcService, DeleteReclaimer};
use substrate_models::{ModelManager, ModelManagerConfig};
use substrate_scheduler::Scheduler;
use substrate_store::{Store, StoreObserver};
use substrate_telemetry::Telemetry;
use substrate_types::{
    CompletionId, CompletionRequest, CompletionState, LifecycleEvent, ModelId,
    Promise, PromiseSender, Result, SystemState, promise_pair,
};

/// Broadcast channel capacity for lifecycle events.
///
/// Old events are dropped when the channel is full (slow consumers miss events
/// rather than blocking the emit path).
const EVENT_CHANNEL_CAPACITY: usize = 256;

use crate::config::InferenceConfig;

// ---------------------------------------------------------------------------
// PromiseRegistry — fulfills in-process Promises via StoreObserver
// ---------------------------------------------------------------------------

/// Holds pending `PromiseSender`s keyed by `CompletionId`.
///
/// Registered as a `StoreObserver` so that when the store writes a terminal
/// state transition (`Completed`, `Failed`, or `Cancelled`), the matching
/// sender is pulled from the map and the result is delivered to the caller.
///
/// The sender is fulfilled asynchronously (via `tokio::spawn`) so that the
/// synchronous store mutation is not blocked by the oneshot send.
struct PromiseRegistry {
    senders: Mutex<HashMap<CompletionId, PromiseSender>>,
    /// A store clone used only to call `get_result` at fulfillment time.
    store: Store,
}

impl PromiseRegistry {
    fn new(store: Store) -> Arc<Self> {
        Arc::new(Self {
            senders: Mutex::new(HashMap::new()),
            store,
        })
    }

    /// Register a sender that should be fulfilled when `id` reaches a terminal state.
    async fn register(&self, id: CompletionId, sender: PromiseSender) {
        self.senders.lock().await.insert(id, sender);
    }
}

impl StoreObserver for PromiseRegistry {
    fn on_completion_state_changed(
        &self,
        id: CompletionId,
        _old: CompletionState,
        new: CompletionState,
    ) {
        // Only act on terminal transitions.
        let is_terminal = matches!(
            new,
            CompletionState::Completed | CompletionState::Failed | CompletionState::Cancelled
        );
        if !is_terminal {
            return;
        }

        // Synchronously remove the sender from the map using `try_lock`.
        // We use `try_lock` because the observer is called from the store's
        // synchronous mutex context. If the lock is contended (unlikely), we
        // log a warning and skip — the caller either dropped their Promise or
        // will time out waiting. This is a best-effort delivery path.
        let sender = match self.senders.try_lock() {
            Ok(mut map) => map.remove(&id),
            Err(_) => {
                tracing::warn!(
                    completion_id = %id,
                    "PromiseRegistry: could not lock senders on state change — promise may not be fulfilled"
                );
                return;
            }
        };

        let Some(sender) = sender else {
            // No in-process waiter registered for this completion; nothing to do.
            return;
        };

        // Spawn an async task to fetch the result and fulfill the promise.
        let store = self.store.clone();
        tokio::spawn(async move {
            match store.get_result(id) {
                Ok(Some(result)) => {
                    if sender.fulfill(result).is_err() {
                        tracing::debug!(
                            completion_id = %id,
                            "PromiseRegistry: receiver dropped before fulfillment"
                        );
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        completion_id = %id,
                        "PromiseRegistry: terminal state reached but no result row found"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        completion_id = %id,
                        err = %e,
                        "PromiseRegistry: get_result failed; promise will not be fulfilled"
                    );
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Prompt Hook seam
// ---------------------------------------------------------------------------

/// Called before a completion is submitted, allowing prompt transformation.
///
/// # Future
/// The prompt roll-up system implements this trait. When the node is embedded
/// in a harness application, a `RollupHook` is injected that expands roll-up
/// references in the prompt before scheduling.
pub trait PromptHook: Send + Sync {
    /// Transform the request in place. Called on every submission.
    fn transform(&self, req: &mut CompletionRequest) -> Result<()>;
}

/// No-op PromptHook (the default when no hook is configured).
struct NoOpPromptHook;

impl PromptHook for NoOpPromptHook {
    fn transform(&self, _req: &mut CompletionRequest) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// InferenceService
// ---------------------------------------------------------------------------

/// A fully initialized substrate inference service. One per machine.
pub struct InferenceService {
    config: InferenceConfig,
    store: Store,
    engine: Arc<ExecutionEngine>,
    scheduler: Arc<Scheduler>,
    telemetry: Arc<Telemetry>,
    model_manager: ModelManager,
    cache_manager: CacheManager,
    prompt_hook: Arc<dyn PromptHook>,
    promise_registry: Arc<PromiseRegistry>,
    /// Broadcast channel for node-level lifecycle events (model, backend, queue).
    event_tx: broadcast::Sender<LifecycleEvent>,
    /// When `true`, the scheduler will not admit new completions.
    paused: Arc<AtomicBool>,
}

impl InferenceService {
    /// Initialize all subsystems and return a ready InferenceService.
    ///
    /// This is the main entry point. Call `InferenceService::serve()` after this to
    /// bind the API server and block until shutdown.
    pub async fn start(config: InferenceConfig) -> Result<Self> {
        tracing::info!("substrate-inference starting");

        // Step 1: Open store, then wire in the PromiseRegistry observer before
        //         distributing the store to any subsystem.
        let db_path = format!("{}/substrate.db", config.data_dir);
        let raw_store = Store::open(&db_path)?;

        // Build the registry with a raw-store clone for its internal get_result calls.
        let promise_registry = PromiseRegistry::new(raw_store.clone());

        // Equip the main store with the observer. All subsystems share this clone.
        let store = raw_store.with_observer(Arc::clone(&promise_registry) as Arc<dyn StoreObserver>);

        // Step 2: Crash recovery.
        let recovered = store.recover_running()?;
        if recovered > 0 {
            tracing::warn!("recovered {} completions from Running state", recovered);
        }

        // Step 3: Telemetry.
        let telemetry = Arc::new(Telemetry::new(config.telemetry_poll_ms));
        Arc::clone(&telemetry).start_polling();

        // Step 4: GC service — tracks model weights, KV cache, and backend binaries.
        let gc = std::sync::Arc::new(
            GcService::new(
                GcConfig {
                    db_path: std::path::PathBuf::from(&config.data_dir).join("gc.db"),
                    sweep_interval_secs: 3600,
                },
                Box::new(DeleteReclaimer),
            )
            .map_err(|e| substrate_types::SubstrateError::Internal(e.to_string()))?,
        );
        let _ = gc.register_dir(&std::path::PathBuf::from(&config.data_dir).join("models"));
        let _ = gc.register_dir(&std::path::PathBuf::from(&config.data_dir).join("backends"));
        let _ = gc.register_dir(&std::path::PathBuf::from(&config.data_dir).join("kv_cache"));
        gc.clone().start_sweep_loop(3600);

        // Step 5: Model manager + sync.
        let model_manager = ModelManager::new(
            ModelManagerConfig {
                models_dir: format!("{}/models", config.data_dir),
                model_weights_budget_bytes: config.resource_limits.model_weights_budget_bytes,
                hf_token: config.hf_token.clone(),
            },
            store.clone(),
            gc.clone(),
        );
        model_manager.sync_registry(&config.models).await?;

        // Step 6: KV cache manager.
        let cache_manager = CacheManager::new(
            CacheConfig {
                cache_dir: format!("{}/kvcache", config.data_dir),
                kv_cache_budget_bytes: config.resource_limits.kv_cache_budget_bytes,
            },
            store.clone(),
            gc.clone(),
        );

        // Step 7: Execution engine.
        // The event channel is created before the engine so the tx can be cloned
        // into the engine. The channel itself is stored on the InferenceService for subscribe().
        let (event_tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);

        let engine = Arc::new(
            ExecutionEngine::new(
                EngineConfig {
                    llama_server_bin: config.llama_server_bin.clone(),
                    llama_server_port: config.llama_server_port,
                    max_concurrent_completions: config.max_concurrent_completions,
                    data_dir: config.data_dir.clone(),
                    model_weights_dir: format!("{}/models", config.data_dir),
                    llama_version: config.llama_version.clone(),
                    llama_data_dir: std::path::PathBuf::from(&config.data_dir),
                    n_gpu_layers: config.n_gpu_layers,
                    context_size: config.context_size,
                },
                store.clone(),
                gc.clone(),
            )
            .await?
            .with_events(event_tx.clone()),
        );

        // Step 8: Scheduler.
        let scheduler = Arc::new(Scheduler::new(
            store.clone(),
            Arc::clone(&engine),
            Arc::clone(&telemetry),
            config.max_concurrent_completions,
        ).with_events(event_tx.clone()));

        // Step 9: Benchmark orchestrator (background).
        let bench_scheduler = Arc::clone(&scheduler);
        let bench_store = store.clone();
        tokio::spawn(async move {
            let orch = BenchmarkOrchestrator::new(
                bench_store,
                BenchmarkConfig::default(),
                bench_scheduler,
            );
            if let Err(e) = orch.run().await {
                tracing::error!("benchmark orchestrator error: {}", e);
            }
        });

        // Step 10: Scheduler loop (background).
        let sched_ref = Arc::clone(&scheduler);
        tokio::spawn(async move {
            if let Err(e) = sched_ref.run().await {
                tracing::error!("scheduler error: {}", e);
            }
        });

        tracing::info!("substrate-inference ready");

        Ok(Self {
            config,
            store,
            engine,
            scheduler,
            telemetry,
            model_manager,
            cache_manager,
            prompt_hook: Arc::new(NoOpPromptHook),
            promise_registry,
            event_tx,
            paused: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Inject a custom prompt hook (must be called before `serve`).
    pub fn with_prompt_hook(mut self, hook: Arc<dyn PromptHook>) -> Self {
        self.prompt_hook = hook;
        self
    }

    /// Bind the API server and block until shutdown.
    pub async fn serve(&self) -> Result<()> {
        let addr: SocketAddr = self.config.api_bind.parse()
            .map_err(|e| substrate_types::SubstrateError::Config(format!("invalid api_bind: {e}")))?;

        let state = ApiState {
            store: self.store.clone(),
            scheduler: Arc::clone(&self.scheduler),
            telemetry: Arc::clone(&self.telemetry),
            event_tx: self.event_tx.clone(),
            paused: Arc::clone(&self.paused),
        };

        let app = substrate_api::router(state);

        tracing::info!("API server listening on {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await
            .map_err(|e| substrate_types::SubstrateError::Internal(format!("bind failed: {e}")))?;

        axum::serve(listener, app).await
            .map_err(|e| substrate_types::SubstrateError::Internal(format!("server error: {e}")))
    }

    // -----------------------------------------------------------------------
    // In-process API
    // -----------------------------------------------------------------------

    /// Submit a completion and return a Promise that resolves when it finishes.
    ///
    /// The `PromiseSender` is registered with the `PromiseRegistry` before the
    /// completion is enqueued. This ensures the sender is in place before the
    /// scheduler can run the completion and the store fires `on_completion_state_changed`.
    pub async fn submit(&self, mut req: CompletionRequest) -> Result<Promise> {
        self.prompt_hook.transform(&mut req)?;

        // Capture the ID before moving the request into the scheduler.
        let id = req.id;

        // Create the promise pair before submitting so the sender is registered
        // before any concurrent scheduler tick can complete the work.
        let (promise, sender) = promise_pair(id);
        self.promise_registry.register(id, sender).await;

        // Enqueue the completion. If this fails, we have an orphaned sender in
        // the registry. Clean it up to avoid a memory leak.
        if let Err(e) = self.scheduler.submit(req) {
            self.promise_registry.senders.lock().await.remove(&id);
            return Err(e);
        }

        Ok(promise)
    }

    /// Cancel a completion.
    pub async fn cancel(&self, id: CompletionId) -> Result<()> {
        self.scheduler.cancel(id).await
    }

    /// Return the current system state.
    pub async fn system_state(&self) -> SystemState {
        self.telemetry.current_state().await
    }

    /// Return the currently resident model ID, if any.
    pub fn resident_model(&self) -> Option<ModelId> {
        self.engine.resident_model()
    }

    /// Trigger model download (returns immediately; download runs in background).
    pub async fn ensure_model(&self, model_id: &ModelId) -> Result<()> {
        let _path = self.model_manager.ensure_available(model_id).await?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Event bus
    // -----------------------------------------------------------------------

    /// Subscribe to node-level lifecycle events.
    ///
    /// Returns a `broadcast::Receiver<LifecycleEvent>` that receives all
    /// subsequent events emitted by the node (model swaps, backend state changes,
    /// queue depth changes, pause/resume). Old events already in-flight before
    /// this call are not replayed.
    ///
    /// Receivers that fall behind by more than `EVENT_CHANNEL_CAPACITY` events
    /// will receive a `RecvError::Lagged` error and must resubscribe.
    pub fn subscribe_events(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.event_tx.subscribe()
    }

    /// Emit a lifecycle event to all active subscribers.
    ///
    /// Silently drops the event when there are no subscribers (the broadcast
    /// channel returns `Err(SendError)` only when the receiver count is zero,
    /// which is normal at startup and during tests).
    pub fn emit_event(&self, event: LifecycleEvent) {
        let _ = self.event_tx.send(event);
    }

    // -----------------------------------------------------------------------
    // Pause / Resume
    // -----------------------------------------------------------------------

    /// Pause execution: the scheduler will stop admitting new completions.
    ///
    /// In-flight completions continue to run. Returns immediately; callers that
    /// need all work drained before proceeding should poll `running_count()` or
    /// call `engine.drain()` explicitly.
    ///
    /// Emits `LifecycleEvent::ExecutionPaused` on the event bus.
    pub fn pause_execution(&self) {
        self.paused.store(true, Ordering::SeqCst);
        self.emit_event(LifecycleEvent::ExecutionPaused);
        tracing::info!("execution paused");
    }

    /// Resume execution: the scheduler will begin admitting new completions.
    ///
    /// Emits `LifecycleEvent::ExecutionResumed` on the event bus.
    pub fn resume_execution(&self) {
        self.paused.store(false, Ordering::SeqCst);
        self.emit_event(LifecycleEvent::ExecutionResumed);
        tracing::info!("execution resumed");
    }

    /// Whether execution is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

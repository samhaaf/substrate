import { writable } from 'svelte/store'

// System state - derived from 'system' topic events / polled from /api/inference/metrics
export const systemState = writable({
  cpu_utilization: 0,       // 0.0–1.0 fraction
  gpu_utilization: 0,       // 0.0–1.0 fraction
  gpu_memory_used_bytes: 0,
  gpu_memory_total_bytes: 0,
  memory_pressure: 0,       // 0.0–1.0 fraction
  total_memory_bytes: 0,
  used_memory_bytes: 0,
})

// Model state
export const modelState = writable({
  resident_model: null, // ModelId string or null
  status: 'idle',       // 'idle'|'loading'|'unloading'|'swapping'|'ready'|'error'
  path: null,
})

// Backend state
export const backendState = writable({
  active: null,          // { binary_path, port } or null
  status: 'stopped',     // 'stopped'|'starting'|'ready'|'stopping'|'installing'
  installed: [],         // list of installed versions
  installing_version: null,
})

// Queue state
export const queueState = writable({
  pending: 0,
  running: 0,
  paused: false,
  items: [], // recent CompletionRow items from /api/inference/completions
})

// GC state
export const gcState = writable({
  dirs: [],           // GcDirRow list
  entries: [],        // GcEntryRow list (most recent 50)
  recent_events: [],  // last 20 GcEvents (as GatewayEvents)
})

// Network state
export const networkState = writable({
  nodes: [
    { id: 'local', url: 'http://127.0.0.1:8420', status: 'unknown' },
    // stub: future nodes from Tailscale would appear here
  ],
})

// Benchmark / kernel state
export const kernelData = writable(null) // { models: [...] } from /api/inference/benchmark/kernel
export const liveDots = writable([])     // [{ output_tokens, tokens_per_sec, ts }] — live inference results

export function addLiveDot(output_tokens, tokens_per_sec) {
  liveDots.update((arr) => {
    const next = [...arr, { output_tokens, tokens_per_sec, ts: Date.now() }]
    // Keep only the last 200 dots
    return next.length > 200 ? next.slice(next.length - 200) : next
  })
}

// Core event-handler. Updates derived state stores from a GatewayEvent.
export function handleEvent(gatewayEvent) {
  if (!gatewayEvent || !gatewayEvent.event) return
  const { service, event } = gatewayEvent

  if (service === 'inference') {
    switch (event.type) {
      case 'model_loading':
        modelState.update((s) => ({ ...s, status: 'loading', resident_model: event.model_id, path: event.path }))
        break
      case 'model_loaded':
        modelState.update((s) => ({ ...s, status: 'ready', resident_model: event.model_id, path: event.path }))
        break
      case 'model_unloading':
        modelState.update((s) => ({ ...s, status: 'unloading' }))
        break
      case 'model_unloaded':
        modelState.update((s) => ({ ...s, status: 'idle', resident_model: null, path: null }))
        break
      case 'model_swapping':
        // LifecycleEvent::ModelSwapping { from, to }
        modelState.update((s) => ({ ...s, status: 'swapping', resident_model: event.to }))
        break
      case 'backend_starting':
        backendState.update((s) => ({ ...s, status: 'starting', active: { binary_path: event.binary_path, port: event.port } }))
        break
      case 'backend_ready':
        backendState.update((s) => ({ ...s, status: 'ready', active: { binary_path: event.binary_path, port: event.port } }))
        break
      case 'backend_stopping':
        backendState.update((s) => ({ ...s, status: 'stopping' }))
        break
      case 'backend_stopped':
        backendState.update((s) => ({ ...s, status: 'stopped', active: null }))
        break
      case 'backend_installing':
        backendState.update((s) => ({ ...s, status: 'installing', installing_version: event.version }))
        break
      case 'backend_installed':
        backendState.update((s) => ({
          ...s,
          status: 'stopped',
          installed: s.installed.includes(event.version) ? s.installed : [...s.installed, event.version],
          installing_version: null,
        }))
        break
      case 'queue_depth_changed':
        queueState.update((s) => ({ ...s, pending: event.pending, running: event.running }))
        break
      case 'execution_paused':
        queueState.update((s) => ({ ...s, paused: true }))
        break
      case 'execution_resumed':
        queueState.update((s) => ({ ...s, paused: false }))
        break
      case 'completion_metrics_recorded':
        // LifecycleEvent::CompletionMetricsRecorded { id, model_id, output_tokens, tokens_per_second }
        // This is the one completion-related event the gateway forwards externally
        // (see substrate-api's is_external_event) — specifically for this live chart.
        if (event.output_tokens != null && event.tokens_per_second != null) {
          addLiveDot(event.output_tokens, event.tokens_per_second)
        }
        break
    }
  }

  if (service === 'gc') {
    gcState.update((s) => ({
      ...s,
      recent_events: [gatewayEvent, ...s.recent_events].slice(0, 20),
    }))
    if (event.type === 'entry_evicted') {
      gcState.update((s) => ({
        ...s,
        entries: s.entries.filter((e) => e.path !== event.path),
      }))
    }
  }

  if (service === 'gateway' || gatewayEvent.topic === 'system') {
    if (event.type === 'system' || event.cpu_utilization !== undefined) {
      systemState.update((s) => ({ ...s, ...event }))
    }
  }
}

<script>
  import { queueState } from '../stores/state.js'
  import { onMount, onDestroy } from 'svelte'

  async function togglePause() {
    const endpoint = $queueState.paused ? '/api/inference/execution/resume' : '/api/inference/execution/pause'
    try { await fetch(endpoint, { method: 'POST' }) } catch (e) { /* ignore */ }
  }

  async function dropCompletion(id) {
    try { await fetch(`/api/inference/completions/${id}`, { method: 'DELETE' }) } catch (e) { /* ignore */ }
  }

  let interval
  onMount(() => {
    const poll = async () => {
      try {
        const r = await fetch('/api/inference/completions?state=pending,running&limit=20')
        if (r.ok) {
          const items = await r.json()
          queueState.update((s) => ({ ...s, items }))
        }
      } catch (e) { /* ignore */ }
    }
    poll()
    interval = setInterval(poll, 1000)
  })
  onDestroy(() => clearInterval(interval))
</script>

<section class="panel queue-panel">
  <h2>QUEUE
    <span class="count pending">{$queueState.pending} pending</span>
    <span class="count running">{$queueState.running} running</span>
    <button class="pause-btn {$queueState.paused ? 'paused' : ''}" on:click={togglePause}>
      {$queueState.paused ? '▶ RESUME' : '⏸ PAUSE'}
    </button>
  </h2>
  <div class="queue-list">
    {#each $queueState.items as item (item.id)}
      <div class="queue-item" class:running={item.state === 'running'}>
        <span class="item-id">{item.id.slice(0, 8)}</span>
        <span class="item-model dim">{item.model_id}</span>
        <span class="item-priority">p{item.priority}</span>
        <span class="item-state {item.state}">{item.state}</span>
        <button class="drop-btn" on:click={() => dropCompletion(item.id)}>✕</button>
      </div>
    {/each}
    {#if $queueState.items.length === 0}
      <div class="dim empty">queue empty</div>
    {/if}
  </div>
</section>

<style>
  .count { font-size: 0.65rem; font-weight: normal; letter-spacing: 0; }
  .count.pending { color: #ffaa00; }
  .count.running { color: #00ff88; }
  .pause-btn { margin-left: auto; }
  .pause-btn.paused { border-color: #ffaa00; color: #ffaa00; }
  .queue-list { display: flex; flex-direction: column; gap: 0.15rem; }
  .queue-item { display: flex; align-items: center; gap: 0.5rem; font-size: 0.7rem; padding: 0.2rem 0.3rem; border-left: 2px solid transparent; }
  .queue-item.running { border-left-color: #00ff88; background: #0d1a12; }
  .item-id { color: #00aaff; width: 4.5rem; }
  .item-model { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .item-priority { color: #888; }
  .item-state { width: 4rem; }
  .item-state.running { color: #00ff88; }
  .item-state.pending { color: #ffaa00; }
  .drop-btn { padding: 0 0.3rem; }
  .empty { font-size: 0.7rem; padding: 0.5rem 0; }
</style>

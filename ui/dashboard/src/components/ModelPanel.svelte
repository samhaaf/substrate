<script>
  import { modelState } from '../stores/state.js'

  const statusColors = {
    idle: '#444', loading: '#ffaa00', ready: '#00ff88',
    unloading: '#ffaa00', swapping: '#00aaff', error: '#ff4444',
  }
</script>

<section id="model-panel" class="panel">
  <h2>MODEL</h2>
  <div id="model-status-badge" class="status-badge" style="color:{statusColors[$modelState.status] || '#ccc'}">
    ● {$modelState.status.toUpperCase()}
  </div>
  {#if $modelState.resident_model}
    <div class="model-id">{$modelState.resident_model}</div>
    {#if $modelState.path}
      <div class="model-path dim">{$modelState.path}</div>
    {/if}
  {:else}
    <div class="dim">No model loaded</div>
  {/if}
</section>

<style>
  .status-badge { font-size: 0.8rem; margin-bottom: 0.5rem; }
  .model-id { font-size: 0.8rem; color: #ccc; word-break: break-all; }
  .model-path { font-size: 0.7rem; margin-top: 0.25rem; word-break: break-all; }
</style>

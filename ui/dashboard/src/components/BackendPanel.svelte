<script>
  import { backendState } from '../stores/state.js'

  const statusColors = {
    stopped: '#444', starting: '#ffaa00', ready: '#00ff88',
    stopping: '#ffaa00', installing: '#00aaff',
  }
</script>

<section id="backend-panel" class="panel">
  <h2>BACKEND</h2>
  <div id="backend-status" class="status" style="color:{statusColors[$backendState.status] || '#ccc'}">
    ● {$backendState.status.toUpperCase()}
  </div>
  {#if $backendState.active}
    <div class="port">:{$backendState.active.port}</div>
    <div class="dim path">{$backendState.active.binary_path}</div>
  {/if}
  {#if $backendState.installing_version}
    <div class="installing">Installing {$backendState.installing_version}...</div>
  {/if}
  {#if $backendState.installed.length}
    <div id="backend-installed-list" class="installed-list">
      {#each $backendState.installed as v}
        <span class="version-tag">{v}</span>
      {/each}
    </div>
  {/if}
</section>

<style>
  .status { font-size: 0.8rem; margin-bottom: 0.5rem; }
  .port { font-size: 0.8rem; color: #ccc; }
  .path { font-size: 0.7rem; word-break: break-all; margin-top: 0.25rem; }
  .installing { color: #00aaff; font-size: 0.75rem; margin-top: 0.5rem; }
  .installed-list { display: flex; flex-wrap: wrap; gap: 0.3rem; margin-top: 0.5rem; }
  .version-tag { background: #1a1a2e; border: 1px solid #333; padding: 0.1rem 0.4rem; font-size: 0.65rem; color: #888; }
</style>

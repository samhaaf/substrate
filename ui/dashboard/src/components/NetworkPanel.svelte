<script>
  import { networkState } from '../stores/state.js'
  import { wsStatus } from '../stores/connection.js'
  // Future: poll /api/nodes for registered mesh nodes
</script>

<section id="network-panel" class="panel">
  <h2>NETWORK</h2>
  <div id="network-node-list" class="node-list">
    {#each $networkState.nodes as node (node.id)}
      <div id={`network-node-${node.id}`} class="node-card">
        <div class="node-id">{node.id}</div>
        <div class="node-url dim">{node.url}</div>
        <div class="node-status" class:connected={node.id === 'local' && $wsStatus === 'connected'}>
          {node.id === 'local' ? $wsStatus : node.status}
        </div>
      </div>
    {/each}
    <div class="stub-note dim">// future: Tailscale nodes will appear here</div>
  </div>
</section>

<style>
  .node-list { display: flex; flex-direction: column; gap: 0.4rem; }
  .node-card { border: 1px solid #1a1a2e; padding: 0.4rem; }
  .node-id { font-size: 0.75rem; color: #ccc; }
  .node-url { font-size: 0.65rem; margin: 0.15rem 0; }
  .node-status { font-size: 0.65rem; color: #555; }
  .node-status.connected { color: #00ff88; }
  .stub-note { font-size: 0.65rem; margin-top: 0.3rem; }
</style>

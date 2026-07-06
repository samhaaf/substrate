<script>
  import { onMount, onDestroy } from 'svelte'
  import { connect, disconnect, wsStatus } from './stores/connection.js'
  import SystemPanel from './components/SystemPanel.svelte'
  import ModelPanel from './components/ModelPanel.svelte'
  import BackendPanel from './components/BackendPanel.svelte'
  import QueuePanel from './components/QueuePanel.svelte'
  import GcPanel from './components/GcPanel.svelte'
  import NetworkPanel from './components/NetworkPanel.svelte'
  import BenchmarkPanel from './components/BenchmarkPanel.svelte'
  import EventLog from './components/EventLog.svelte'

  onMount(() => {
    connect()
  })

  onDestroy(() => {
    disconnect()
  })
</script>

<main>
  <header>
    <h1>SUBSTRATE</h1>
    <span class="ws-status" class:connected={$wsStatus === 'connected'}>{$wsStatus}</span>
  </header>

  <div class="grid">
    <div class="col-left">
      <QueuePanel />
      <BenchmarkPanel />
    </div>
    <div class="col-right">
      <SystemPanel />
      <ModelPanel />
      <BackendPanel />
      <GcPanel />
      <NetworkPanel />
    </div>
  </div>

  <EventLog />
</main>

<style>
  :global(*) { box-sizing: border-box; margin: 0; padding: 0; }
  :global(body) { background: #0a0a0f; color: #ccc; font-family: 'JetBrains Mono', 'Fira Code', monospace; }
  :global(.panel) { border: 1px solid #1a1a2e; padding: 1rem; margin-bottom: 0.5rem; background: #0d0d18; }
  :global(.panel h2) { color: #00ff88; font-size: 0.75rem; letter-spacing: 0.15em; margin-bottom: 0.75rem; display: flex; align-items: center; gap: 0.5rem; }
  :global(.dim) { color: #555; }
  :global(button) { background: #1a1a2e; border: 1px solid #333; color: #ccc; padding: 0.2rem 0.5rem; cursor: pointer; font-family: monospace; font-size: 0.7rem; }
  :global(button:hover) { border-color: #00ff88; color: #00ff88; }

  main { padding: 1rem; max-width: 1400px; margin: 0 auto; }
  header { display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem; border-bottom: 1px solid #1a1a2e; padding-bottom: 0.5rem; }
  h1 { color: #00ff88; font-size: 0.9rem; letter-spacing: 0.3em; }
  .ws-status { font-size: 0.7rem; color: #555; }
  .ws-status.connected { color: #00ff88; }

  .grid { display: grid; grid-template-columns: 2fr 1fr; gap: 0.5rem; }

  @media (max-width: 900px) {
    .grid { grid-template-columns: 1fr; }
  }
</style>

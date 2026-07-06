<script>
  import { eventLog, clearLog } from '../stores/connection.js'

  let collapsed = false

  function formatTs(ts) {
    if (!ts) return ''
    return new Date(ts).toLocaleTimeString()
  }

  function topicColor(topic) {
    const colors = { lifecycle: '#00aaff', queue: '#ffaa00', gc: '#ff88aa', system: '#888', completions: '#00ff88', all: '#ccc' }
    return colors[topic] || '#ccc'
  }
</script>

<section class="panel event-log" class:collapsed>
  <h2>
    EVENT LOG ({$eventLog.length})
    <button on:click={() => (collapsed = !collapsed)}>{collapsed ? '▼' : '▲'}</button>
    <button on:click={clearLog}>CLEAR</button>
  </h2>
  {#if !collapsed}
    <div class="log-list">
      {#each [...$eventLog].reverse().slice(0, 200) as evt, i (i)}
        <div class="log-entry">
          <span class="log-ts">{formatTs(evt.ts)}</span>
          <span class="log-topic" style="color:{topicColor(evt.topic)}">{evt.topic}</span>
          <span class="log-service dim">{evt.service}</span>
          <span class="log-type">{evt.event?.type || '?'}</span>
        </div>
      {/each}
      {#if $eventLog.length === 0}
        <div class="dim empty">no events yet</div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .event-log { margin-top: 0.5rem; }
  .event-log h2 { justify-content: flex-start; }
  .event-log h2 button { margin-left: 0.5rem; }
  .log-list { max-height: 260px; overflow-y: auto; display: flex; flex-direction: column; gap: 0.05rem; }
  .log-entry { display: flex; gap: 0.75rem; font-size: 0.68rem; padding: 0.1rem 0; }
  .log-ts { color: #555; width: 6rem; }
  .log-topic { width: 6rem; }
  .log-service { width: 5rem; }
  .log-type { color: #ccc; }
  .empty { font-size: 0.7rem; padding: 0.5rem 0; }
</style>

<script>
  import { systemState } from '../stores/state.js'
  import Bar from './Bar.svelte'
  import { onMount, onDestroy } from 'svelte'

  function formatBytes(b) {
    if (!b) return '0 B'
    if (b > 1e9) return (b / 1e9).toFixed(1) + ' GB'
    if (b > 1e6) return (b / 1e6).toFixed(1) + ' MB'
    if (b > 1e3) return (b / 1e3).toFixed(1) + ' KB'
    return b + ' B'
  }

  // Disk stats polled from /api/nodes/local/stats (every 10s — heavier endpoint)
  let diskStats = null
  let metricsInterval
  let diskInterval

  onMount(() => {
    const pollMetrics = async () => {
      try {
        const r = await fetch('/api/inference/metrics')
        if (r.ok) {
          const d = await r.json()
          systemState.set(d)
        }
      } catch (e) { /* ignore */ }
    }
    pollMetrics()
    metricsInterval = setInterval(pollMetrics, 2000)

    const pollDisk = async () => {
      try {
        const r = await fetch('/api/nodes/local/stats')
        if (r.ok) {
          const d = await r.json()
          diskStats = d
        }
      } catch (e) { /* ignore */ }
    }
    pollDisk()
    diskInterval = setInterval(pollDisk, 10000)
  })
  onDestroy(() => {
    clearInterval(metricsInterval)
    clearInterval(diskInterval)
  })

  $: gpuEstimate = $systemState.is_gpu_estimate
  $: gpuUtilPct = $systemState.gpu_utilization * 100
  $: showGpu = gpuEstimate !== false  // show when true or undefined

  $: ramUsed = $systemState.used_memory_bytes
  $: ramTotal = $systemState.total_memory_bytes
  $: ramPressure = $systemState.memory_pressure || 0
  $: ramPct = ramTotal > 0 ? (ramUsed / ramTotal * 100) : (ramPressure * 100)
  $: ramPressureColor = ramPct > 90 ? '#ff4444' : ramPct > 70 ? '#ffaa00' : '#00ff88'

  $: diskTotal = diskStats?.disk?.total_bytes || 0
  $: diskUsed = diskStats?.disk?.used_bytes || 0
  $: diskFree = diskStats?.disk?.free_bytes || 0
  $: diskGc = diskStats?.disk?.gc_managed_bytes || 0

  // Segment widths as percentages of total. gc_managed_bytes can come from a
  // directory outside the root mount, so it isn't guaranteed to be a subset
  // of used_bytes — clamp it and derive the other two segments from what's
  // left so the three always sum to 100%.
  $: diskGcClamped = diskTotal > 0 ? Math.min(diskGc, diskUsed, diskTotal) : 0
  $: diskOtherUsed = diskTotal > 0 ? Math.max(0, diskUsed - diskGcClamped) : 0
  $: diskGcPct = diskTotal > 0 ? (diskGcClamped / diskTotal * 100) : 0
  $: diskOtherPct = diskTotal > 0 ? (diskOtherUsed / diskTotal * 100) : 0
  $: diskFreePct = diskTotal > 0 ? Math.max(0, 100 - diskGcPct - diskOtherPct) : 0
</script>

<section id="system-panel" class="panel">
  <h2>SYSTEM</h2>
  <Bar id="system-cpu-bar" label="CPU" value={$systemState.cpu_utilization * 100} />

  {#if showGpu}
    <div id="system-gpu-bar" class="bar-row" title="GPU utilization estimate via ioreg — may not reflect true compute load">
      <span class="bar-label">GPU</span>
      <div class="bar-track">
        <div class="bar-fill" style="width:{Math.min(gpuUtilPct, 100)}%;background:{gpuUtilPct > 90 ? '#ff4444' : gpuUtilPct > 70 ? '#ffaa00' : '#00ff88'}"></div>
      </div>
      <span class="bar-value">~{gpuUtilPct.toFixed(1)}%</span>
    </div>
  {:else}
    <div id="system-gpu-bar" class="bar-row">
      <span class="bar-label">GPU</span>
      <span class="na-text">N/A</span>
    </div>
  {/if}

  <Bar id="system-vram-bar" label="VRAM" value={$systemState.gpu_memory_total_bytes > 0 ? ($systemState.gpu_memory_used_bytes / $systemState.gpu_memory_total_bytes * 100) : 0} />

  <!-- RAM as a progress bar -->
  <div id="system-ram-bar" class="bar-row">
    <span class="bar-label">RAM</span>
    <div class="bar-track">
      <div class="bar-fill" style="width:{Math.min(ramPct, 100)}%;background:{ramPressureColor}"></div>
    </div>
    <span class="bar-value ram-value">
      {#if ramTotal > 0}
        {formatBytes(ramUsed)} / {formatBytes(ramTotal)} ({ramPct.toFixed(0)}%)
      {:else}
        {(ramPressure * 100).toFixed(0)}%
      {/if}
    </span>
  </div>

  <!-- Disk stacked bar -->
  {#if diskTotal > 0}
    <div id="system-disk-bar" class="disk-section">
      <div class="bar-row">
        <span class="bar-label">Disk</span>
        <div class="bar-track stacked">
          <div class="seg seg-gc" style="width:{Math.min(diskGcPct, 100)}%"></div>
          <div class="seg seg-used" style="width:{Math.min(diskOtherPct, 100)}%"></div>
          <div class="seg seg-free" style="width:{Math.min(diskFreePct, 100)}%"></div>
        </div>
      </div>
      <div class="disk-labels dim">
        <span>{formatBytes(diskUsed)} used / {formatBytes(diskTotal)} total</span>
        <span class="gc-managed">GC managing {formatBytes(diskGc)}</span>
      </div>
    </div>
  {:else}
    <div id="system-disk-bar" class="bar-row">
      <span class="bar-label">Disk</span>
      <span class="na-text dim">loading…</span>
    </div>
  {/if}
</section>

<style>
  .bar-row { display:flex; align-items:center; gap:0.5rem; font-family:monospace; font-size:0.8rem; margin:0.25rem 0; }
  .bar-label { width:3rem; color:#888; flex-shrink:0; }
  .bar-track { flex:1; height:8px; background:#1a1a2e; border:1px solid #333; position:relative; overflow:hidden; }
  .bar-track.stacked { display:flex; }
  .bar-fill { height:100%; transition:width 0.3s ease; }
  .bar-value { width:3.5rem; text-align:right; color:#ccc; }
  .ram-value { width:auto; min-width:3.5rem; font-size:0.7rem; white-space:nowrap; }
  .na-text { color:#555; font-size:0.75rem; }

  /* Disk stacked segments */
  .seg { height:100%; transition:width 0.3s ease; }
  .seg-gc { background:#00ff88; }
  .seg-used { background:#3a3a4e; }
  .seg-free { background:#1a1a2e; border-right:none; }

  .disk-section { margin:0.25rem 0; }
  .disk-labels { display:flex; justify-content:space-between; font-size:0.65rem; margin-top:0.15rem; padding-left:3.5rem; }
  .gc-managed { color:#00ff88; opacity:0.7; }
</style>

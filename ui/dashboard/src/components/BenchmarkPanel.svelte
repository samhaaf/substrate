<script>
  import { kernelData, liveDots } from '../stores/state.js'
  import { onMount, onDestroy } from 'svelte'

  // Chart dimensions
  const CHART_W = 400
  const CHART_H = 160
  const PAD = { top: 10, right: 10, bottom: 28, left: 40 }
  const innerW = CHART_W - PAD.left - PAD.right
  const innerH = CHART_H - PAD.top - PAD.bottom

  // Palette for multi-model curves
  const COLORS = ['#00ff88', '#00aaff', '#ffaa00', '#ff6688', '#aa88ff']

  // Model selector state
  let selectorOpen = false
  // Which model_ids are toggled on (all on by default when data arrives)
  let enabledModels = new Set()

  // Running benchmark state
  let benchRunning = false
  let benchError = null

  // Reactive: which models to render
  $: models = $kernelData?.models || []
  $: currentModel = models.find((m) => m.is_current) || models[0]

  // When models list changes, enable all by default
  $: if (models.length > 0) {
    const next = new Set(enabledModels)
    for (const m of models) {
      if (!next.has(m.model_id)) next.add(m.model_id)
    }
    enabledModels = next
  }

  $: enabledList = models.filter((m) => enabledModels.has(m.model_id))

  // Compute chart domain across all enabled models + live dots
  $: xMax = computeXMax(enabledList, $liveDots)
  $: yMax = computeYMax(enabledList, $liveDots)

  function computeXMax(mods, dots) {
    let max = 100
    for (const m of mods) {
      const mx = (m.x_max_measured || 0) * 1.2
      if (mx > max) max = mx
    }
    for (const d of dots) {
      if (d.output_tokens > max) max = d.output_tokens * 1.1
    }
    return max
  }

  function computeYMax(mods, dots) {
    let max = 10
    for (const m of mods) {
      const yAtXmax = evalQuad(m.coefficients, m.x_max_measured || 0)
      const yAtXmin = evalQuad(m.coefficients, m.x_min_measured || 0)
      const peak = Math.max(yAtXmax, yAtXmin, 1)
      if (peak > max) max = peak
    }
    for (const d of dots) {
      if (d.tokens_per_sec > max) max = d.tokens_per_sec * 1.1
    }
    return max * 1.1
  }

  function evalQuad(coefs, x) {
    if (!coefs || coefs.length < 3) return 0
    const [a, b, c] = coefs
    return a + b * x + c * x * x
  }

  function toSvgX(x) {
    return PAD.left + (x / xMax) * innerW
  }
  function toSvgY(y) {
    return PAD.top + innerH - (y / yMax) * innerH
  }

  // Build SVG path for a quadratic model curve; split at measured range boundary
  function buildPaths(m) {
    if (!m.coefficients || m.coefficients.length < 3) return { solid: '', dashed: '' }
    const xMinM = m.x_min_measured || 0
    const xMaxM = m.x_max_measured || xMax
    const steps = 80
    const dx = xMax / steps

    const points = []
    for (let i = 0; i <= steps; i++) {
      const x = i * dx
      const y = evalQuad(m.coefficients, x)
      points.push({ x, y, inRange: x >= xMinM && x <= xMaxM })
    }

    // Solid path — inside measured range
    let solid = ''
    let inSeg = false
    for (const p of points) {
      if (p.inRange) {
        const sx = toSvgX(p.x)
        const sy = toSvgY(Math.max(0, p.y))
        solid += inSeg ? `L${sx},${sy}` : `M${sx},${sy}`
        inSeg = true
      } else {
        inSeg = false
      }
    }

    // Dashed path — outside measured range (extrapolated)
    let dashed = ''
    inSeg = false
    for (const p of points) {
      if (!p.inRange) {
        const sx = toSvgX(p.x)
        const sy = toSvgY(Math.max(0, p.y))
        dashed += inSeg ? `L${sx},${sy}` : `M${sx},${sy}`
        inSeg = true
      } else {
        // bridge across the measured range gap so the two dashed segments
        // on either side are separate
        inSeg = false
      }
    }

    return { solid, dashed }
  }

  // Data point circles (measured benchmarks)
  function dataPoints(m) {
    if (!m.data_points) return []
    return m.data_points.map((p) => ({
      cx: toSvgX(p.x),
      cy: toSvgY(Math.max(0, p.y)),
    }))
  }

  // Axis tick helpers
  function xTicks() {
    return [0, xMax * 0.25, xMax * 0.5, xMax * 0.75, xMax].map((v) => ({
      v: Math.round(v),
      x: toSvgX(v),
    }))
  }
  function yTicks() {
    const vals = [0, yMax * 0.5, yMax]
    return vals.map((v) => ({ v: v.toFixed(0), y: toSvgY(v) }))
  }

  function toggleModel(id) {
    const next = new Set(enabledModels)
    if (next.has(id)) {
      if (next.size > 1) next.delete(id) // keep at least one
    } else {
      next.add(id)
    }
    enabledModels = next
  }

  async function fetchKernel() {
    try {
      const r = await fetch('/api/inference/benchmark/kernel')
      if (r.ok) {
        const d = await r.json()
        kernelData.set(d)
      }
    } catch (e) { /* ignore */ }
  }

  // Poll interval and overall timeout while waiting for a sweep to finish.
  const POLL_MS = 1500
  const MAX_WAIT_MS = 5 * 60 * 1000

  // Poll GET /v1/collections/:id (the run_id returned by POST .../benchmark/run)
  // until the collection reaches a terminal state, then refresh the kernel data.
  // POST /v1/benchmark/run only submits the sweep and returns immediately, so
  // there is no other signal available to know when it actually finishes.
  async function pollCollectionUntilDone(runId) {
    const deadline = Date.now() + MAX_WAIT_MS
    while (Date.now() < deadline) {
      await new Promise((res) => setTimeout(res, POLL_MS))
      try {
        const r = await fetch(`/api/inference/collections/${runId}`)
        if (r.ok) {
          const d = await r.json()
          if (d.state === 'completed' || d.state === 'failed' || d.state === 'cancelled') {
            return
          }
        }
      } catch (e) { /* ignore transient errors; keep polling */ }
    }
  }

  async function runBenchmark() {
    benchRunning = true
    benchError = null
    try {
      const r = await fetch('/api/inference/benchmark/run', { method: 'POST' })
      if (r.ok) {
        const d = await r.json()
        if (d.run_id) {
          await pollCollectionUntilDone(d.run_id)
        }
        await fetchKernel()
      } else {
        benchError = `Error ${r.status}`
      }
    } catch (e) {
      benchError = 'Request failed'
    } finally {
      benchRunning = false
    }
  }

  onMount(() => {
    fetchKernel()
  })
</script>

<section class="panel">
  <h2>INFERENCE BENCHMARK</h2>

  <!-- Model selector header -->
  <div class="model-selector">
    <button class="selector-toggle" on:click={() => selectorOpen = !selectorOpen}>
      <span class="selector-label">Kernel: {currentModel?.model_name || currentModel?.model_id || '—'}</span>
      <span class="arrow">{selectorOpen ? '▾' : '▸'}</span>
    </button>
    {#if selectorOpen}
      <div class="model-list">
        {#each models as m, i}
          <label class="model-item">
            <input
              type="checkbox"
              checked={enabledModels.has(m.model_id)}
              on:change={() => toggleModel(m.model_id)}
            />
            <span class="model-dot" style="background:{COLORS[i % COLORS.length]}"></span>
            <span class="model-name">{m.model_name || m.model_id}</span>
            {#if m.is_current}<span class="current-tag">current</span>{/if}
          </label>
        {/each}
        {#if models.length === 0}
          <div class="dim">no benchmark data</div>
        {/if}
      </div>
    {/if}
  </div>

  <!-- SVG Kernel Curve Chart -->
  {#if enabledList.length > 0}
    <div class="chart-wrap">
      <svg width="100%" viewBox="0 0 {CHART_W} {CHART_H}" class="chart-svg" preserveAspectRatio="none">
        <!-- X axis -->
        <line x1={PAD.left} y1={PAD.top + innerH} x2={PAD.left + innerW} y2={PAD.top + innerH} stroke="#333" stroke-width="1"/>
        <!-- Y axis -->
        <line x1={PAD.left} y1={PAD.top} x2={PAD.left} y2={PAD.top + innerH} stroke="#333" stroke-width="1"/>

        <!-- X ticks -->
        {#each xTicks() as t}
          <line x1={t.x} y1={PAD.top + innerH} x2={t.x} y2={PAD.top + innerH + 3} stroke="#555" stroke-width="1"/>
          <text x={t.x} y={CHART_H - 4} fill="#555" font-size="8" text-anchor="middle">{t.v}</text>
        {/each}

        <!-- Y ticks -->
        {#each yTicks() as t}
          <line x1={PAD.left - 3} y1={t.y} x2={PAD.left} y2={t.y} stroke="#555" stroke-width="1"/>
          <text x={PAD.left - 5} y={t.y + 3} fill="#555" font-size="8" text-anchor="end">{t.v}</text>
        {/each}

        <!-- Axis labels -->
        <text x={PAD.left + innerW / 2} y={CHART_H - 1} fill="#555" font-size="8" text-anchor="middle">output tokens</text>
        <text x="8" y={PAD.top + innerH / 2} fill="#555" font-size="8" text-anchor="middle" transform="rotate(-90,8,{PAD.top + innerH / 2})">tok/s</text>

        <!-- Curves per enabled model -->
        {#each enabledList as m, i}
          {@const color = COLORS[i % COLORS.length]}
          {@const paths = buildPaths(m)}
          {@const pts = dataPoints(m)}

          <!-- Solid line (measured range) -->
          {#if paths.solid}
            <path d={paths.solid} fill="none" stroke={color} stroke-width="1.5"/>
          {/if}
          <!-- Dashed line (extrapolated) -->
          {#if paths.dashed}
            <path d={paths.dashed} fill="none" stroke={color} stroke-width="1" stroke-dasharray="3,3" opacity="0.5"/>
          {/if}

          <!-- Measured data points -->
          {#each pts as p}
            <circle cx={p.cx} cy={p.cy} r="2.5" fill={color} opacity="0.9"/>
          {/each}
        {/each}

        <!-- Live inference dots -->
        {#each $liveDots as d}
          <circle
            cx={toSvgX(Math.min(d.output_tokens, xMax))}
            cy={toSvgY(Math.min(d.tokens_per_sec, yMax))}
            r="3"
            fill="#ffffff"
            opacity="0.7"
          />
        {/each}
      </svg>
    </div>
  {:else}
    <div class="dim no-data">no benchmark data — run benchmark to populate</div>
  {/if}

  <!-- Run Benchmark button -->
  <div class="bench-controls">
    <button class="bench-btn" on:click={runBenchmark} disabled={benchRunning}>
      {#if benchRunning}
        <span class="spinner">⟳</span> Benchmarking…
      {:else}
        Run Benchmark
      {/if}
    </button>
    {#if benchError}
      <span class="bench-error dim">{benchError}</span>
    {/if}
  </div>
</section>

<style>
  .model-selector { margin-bottom: 0.5rem; }

  .selector-toggle {
    background: none;
    border: none;
    color: #ccc;
    cursor: pointer;
    padding: 0.2rem 0;
    font-family: monospace;
    font-size: 0.72rem;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    text-align: left;
  }
  .selector-toggle:hover { color: #00ff88; }
  .selector-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .arrow { color: #555; font-size: 0.65rem; flex-shrink: 0; }

  .model-list {
    margin-top: 0.3rem;
    border: 1px solid #1a1a2e;
    padding: 0.4rem;
    background: #0a0a0f;
  }
  .model-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.7rem;
    cursor: pointer;
    padding: 0.1rem 0;
    color: #ccc;
  }
  .model-item input[type="checkbox"] {
    accent-color: #00ff88;
    cursor: pointer;
    margin: 0;
  }
  .model-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .model-name { flex: 1; }
  .current-tag {
    font-size: 0.6rem;
    color: #00ff88;
    border: 1px solid #00ff88;
    padding: 0 0.2rem;
    opacity: 0.7;
  }

  .chart-wrap {
    border: 1px solid #1a1a2e;
    background: #0a0a0f;
    margin-bottom: 0.5rem;
    overflow: hidden;
  }
  .chart-svg {
    display: block;
    width: 100%;
    height: 160px;
  }

  .no-data { font-size: 0.72rem; margin: 0.5rem 0; }

  .bench-controls {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 0.25rem;
  }
  .bench-btn {
    font-size: 0.7rem;
    padding: 0.2rem 0.75rem;
  }
  .bench-btn:disabled { opacity: 0.5; cursor: default; border-color: #333; color: #888; }
  .bench-error { font-size: 0.68rem; }

  .spinner {
    display: inline-block;
    animation: spin 1s linear infinite;
  }
  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }
</style>

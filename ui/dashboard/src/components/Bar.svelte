<script>
  export let label = ''
  export let value = 0 // 0-1 or 0-100, normalize it
  $: pct = value > 1 ? value : value * 100
  $: color = pct > 90 ? '#ff4444' : pct > 70 ? '#ffaa00' : '#00ff88'
</script>

<div class="bar-row">
  <span class="bar-label">{label}</span>
  <div class="bar-track">
    <div class="bar-fill" style="width:{Math.min(pct, 100)}%;background:{color}"></div>
  </div>
  <span class="bar-value">{pct.toFixed(1)}%</span>
</div>

<style>
  .bar-row { display:flex; align-items:center; gap:0.5rem; font-family:monospace; font-size:0.8rem; margin:0.25rem 0; }
  .bar-label { width:3rem; color:#888; }
  .bar-track { flex:1; height:8px; background:#1a1a2e; border:1px solid #333; }
  .bar-fill { height:100%; transition:width 0.3s ease; }
  .bar-value { width:3.5rem; text-align:right; color:#ccc; }
</style>

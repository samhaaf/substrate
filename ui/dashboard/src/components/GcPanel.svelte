<script>
  import { gcState } from '../stores/state.js'
  import { onMount, onDestroy } from 'svelte'

  function formatBytes(b) {
    if (!b) return '0 B'
    if (b > 1e9) return (b / 1e9).toFixed(1) + ' GB'
    if (b > 1e6) return (b / 1e6).toFixed(1) + ' MB'
    if (b > 1e3) return (b / 1e3).toFixed(1) + ' KB'
    return b + ' B'
  }

  function formatAge(ts) {
    if (!ts) return ''
    const secs = Math.floor(Date.now() / 1000) - ts
    if (secs < 60) return `${secs}s ago`
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`
    return `${Math.floor(secs / 86400)}d ago`
  }

  function isLocked(entry) {
    return entry.lock_expires_at && entry.lock_expires_at * 1000 > Date.now()
  }

  function basename(p) {
    return p ? p.split('/').pop() : ''
  }

  // Turn an arbitrary filesystem path into a safe id fragment for dynamic
  // element ids (e.g. gc-dir-toggle-${slug(dir.root)}).
  function slug(p) {
    return (p || '').replace(/[^a-zA-Z0-9]+/g, '-').replace(/^-+|-+$/g, '')
  }

  // Track which dirs are expanded; default collapsed
  let expanded = {}

  function toggle(root) {
    expanded = { ...expanded, [root]: !expanded[root] }
  }

  // Group entries by their parent managed dir root
  $: dirEntryMap = buildDirMap($gcState.dirs, $gcState.entries)

  function buildDirMap(dirs, entries) {
    const map = {}
    for (const dir of dirs) {
      map[dir.root] = { dir, children: [] }
    }
    for (const entry of entries) {
      // Entries carry their exact managed-dir root (EntryResponse.dir_root) —
      // group by that instead of a fragile path-prefix match.
      if (map[entry.dir_root]) {
        map[entry.dir_root].children.push(entry)
      }
    }
    return map
  }

  let interval
  onMount(() => {
    const poll = async () => {
      try {
        const [dirsR, entriesR] = await Promise.all([
          fetch('/api/gc/dirs'),
          fetch('/api/gc/entries'),
        ])
        if (dirsR.ok) {
          const dirs = await dirsR.json()
          gcState.update((s) => ({ ...s, dirs }))
        }
        if (entriesR.ok) {
          const entries = (await entriesR.json()).slice(0, 50)
          gcState.update((s) => ({ ...s, entries }))
        }
      } catch (e) { /* ignore */ }
    }
    poll()
    interval = setInterval(poll, 5000)
  })
  onDestroy(() => clearInterval(interval))
</script>

<section id="gc-panel" class="panel">
  <h2>GARBAGE COLLECTOR</h2>

  {#if $gcState.dirs.length === 0}
    <div class="dim empty">no managed directories</div>
  {:else}
    <div id="gc-dir-tree" class="dir-tree">
      {#each $gcState.dirs as dir (dir.root)}
        {@const group = dirEntryMap[dir.root]}
        {@const usedBytes = dir.used_bytes || 0}
        {@const maxBytes = dir.policy?.max_size_bytes || 0}
        {@const usedPct = maxBytes > 0 ? Math.min(usedBytes / maxBytes * 100, 100) : 0}
        {@const budgetColor = usedPct > 90 ? '#ff4444' : usedPct > 70 ? '#ffaa00' : '#00ff88'}
        {@const isOpen = !!expanded[dir.root]}

        <div class="dir-row">
          <button id={`gc-dir-toggle-${slug(dir.root)}`} class="toggle" on:click={() => toggle(dir.root)} aria-label={isOpen ? 'collapse' : 'expand'}>
            {isOpen ? '▾' : '▸'}
          </button>
          <div class="dir-info">
            <span class="dir-name">{basename(dir.root)}</span>
            <div class="budget-bar-wrap">
              <div class="budget-track">
                <div class="budget-fill" style="width:{usedPct}%;background:{budgetColor}"></div>
              </div>
              <span class="budget-numbers dim">
                {formatBytes(usedBytes)} / {formatBytes(maxBytes)}
              </span>
            </div>
          </div>
        </div>

        {#if isOpen}
          <div id={`gc-entries-list-${slug(dir.root)}`} class="entries-list">
            {#if group && group.children.length > 0}
              {#each group.children as entry (entry.path)}
                <div id={`gc-entry-${slug(entry.path)}`} class="gc-entry" class:locked={isLocked(entry)}>
                  <span class="indent">  </span>
                  <span class="entry-path">{basename(entry.path)}</span>
                  <span class="entry-size dim">{formatBytes(entry.size_bytes)}</span>
                  {#if entry.last_touched_at}<span class="entry-age dim">{formatAge(entry.last_touched_at)}</span>{/if}
                  {#if isLocked(entry)}<span class="lock-icon">🔒</span>{/if}
                  <span class="entry-state {entry.state}">{entry.state}</span>
                </div>
              {/each}
            {:else}
              <div class="dim empty-entries">  no entries</div>
            {/if}
          </div>
        {/if}
      {/each}
    </div>
  {/if}

  <div id="gc-events-list" class="gc-events">
    <h3>Recent Events</h3>
    {#each $gcState.recent_events as evt, i (i)}
      <div class="gc-event dim">{evt.event.type} {evt.event.path ? basename(evt.event.path) : ''}</div>
    {/each}
    {#if $gcState.recent_events.length === 0}
      <div class="dim empty">no events</div>
    {/if}
  </div>
</section>

<style>
  .dir-tree { margin-bottom: 0.5rem; }

  .dir-row {
    display: flex;
    align-items: flex-start;
    gap: 0.3rem;
    padding: 0.25rem 0;
    border-top: 1px solid #1a1a2e;
  }
  .dir-row:first-child { border-top: none; }

  .toggle {
    background: none;
    border: none;
    color: #555;
    cursor: pointer;
    padding: 0;
    font-size: 0.7rem;
    line-height: 1;
    flex-shrink: 0;
    margin-top: 0.1rem;
  }
  .toggle:hover { color: #00ff88; border: none; }

  .dir-info { flex: 1; min-width: 0; }
  .dir-name { font-size: 0.72rem; color: #ccc; display: block; margin-bottom: 0.2rem; }

  .budget-bar-wrap { display: flex; align-items: center; gap: 0.4rem; }
  .budget-track { flex: 1; height: 5px; background: #1a1a2e; border: 1px solid #333; overflow: hidden; }
  .budget-fill { height: 100%; transition: width 0.3s ease; }
  .budget-numbers { font-size: 0.62rem; white-space: nowrap; }

  .entries-list { margin-bottom: 0.25rem; }
  .gc-entry {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.65rem;
    padding: 0.08rem 0;
  }
  .gc-entry.locked { color: #ffaa00; }
  .indent { color: transparent; user-select: none; }
  .entry-path { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .entry-size { flex-shrink: 0; }
  .entry-age { flex-shrink: 0; }
  .entry-state { color: #00ff88; width: 3.5rem; text-align: right; flex-shrink: 0; }
  .lock-icon { flex-shrink: 0; }
  .empty-entries { font-size: 0.65rem; padding: 0.1rem 0; }

  .gc-events { margin-top: 0.75rem; }
  .gc-events h3 { font-size: 0.65rem; color: #888; letter-spacing: 0.1em; margin-bottom: 0.3rem; }
  .gc-event { font-size: 0.65rem; }
  .empty { font-size: 0.68rem; }
</style>

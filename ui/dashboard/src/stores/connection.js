import { writable } from 'svelte/store'
import { handleEvent, systemState, queueState } from './state.js'

export const wsStatus = writable('disconnected') // 'disconnected'|'connecting'|'connected'|'error'
export const events = writable([]) // all received GatewayEvents, capped at 500
export const eventLog = writable([]) // persisted to localStorage, used for replay

const STORAGE_KEY = 'substrate_event_log'
const MAX_EVENTS = 500
const MAX_LOG = 2000

let ws = null
let shouldReconnect = false
let reconnectDelay = 1000
let reconnectTimer = null
let saveTimer = null

// Load any persisted log on module init — display only, do NOT replay into state.
// State from a previous session is stale; live snapshots will populate panels on connect.
try {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved) {
    const parsed = JSON.parse(saved)
    if (Array.isArray(parsed)) {
      eventLog.set(parsed)
    }
  }
} catch (e) {
  // corrupt storage — ignore
}

function scheduleSave() {
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = setTimeout(() => {
    saveTimer = null
    let current = []
    const unsub = eventLog.subscribe((v) => { current = v })
    unsub()
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(current))
    } catch (e) {
      // quota exceeded — drop oldest half and retry once
      try {
        const trimmed = current.slice(Math.floor(current.length / 2))
        localStorage.setItem(STORAGE_KEY, JSON.stringify(trimmed))
        eventLog.set(trimmed)
      } catch (e2) { /* give up */ }
    }
  }, 1000)
}

function wsUrl() {
  if (import.meta.env && import.meta.env.DEV) {
    return 'ws://localhost:8400/events'
  }
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${window.location.host}/events`
}

export function connect() {
  shouldReconnect = true
  if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
    return
  }
  wsStatus.set('connecting')
  try {
    ws = new WebSocket(wsUrl())
  } catch (e) {
    wsStatus.set('error')
    scheduleReconnect()
    return
  }

  ws.onopen = () => {
    wsStatus.set('connected')
    reconnectDelay = 1000
    ws.send(JSON.stringify({ action: 'subscribe', topics: ['all'] }))

    // Snapshot current state immediately so panels don't wait for the next event.
    fetch('/api/inference/metrics')
      .then((r) => (r.ok ? r.json() : null))
      .then((d) => { if (d) systemState.set(d) })
      .catch(() => { /* ignore — panel polls will catch up */ })
    fetch('/api/inference/completions?state=pending,running&limit=20')
      .then((r) => (r.ok ? r.json() : null))
      .then((items) => { if (items) queueState.update((s) => ({ ...s, items })) })
      .catch(() => { /* ignore */ })
  }

  ws.onmessage = (msg) => {
    let evt
    try {
      evt = JSON.parse(msg.data)
    } catch (e) {
      return
    }
    events.update((arr) => {
      const next = [...arr, evt]
      return next.length > MAX_EVENTS ? next.slice(next.length - MAX_EVENTS) : next
    })
    eventLog.update((arr) => {
      const next = [...arr, evt]
      return next.length > MAX_LOG ? next.slice(next.length - MAX_LOG) : next
    })
    scheduleSave()
    try { handleEvent(evt) } catch (e) { /* ignore handler errors */ }
  }

  ws.onerror = () => {
    wsStatus.set('error')
  }

  ws.onclose = () => {
    ws = null
    if (shouldReconnect) {
      wsStatus.set('disconnected')
      scheduleReconnect()
    } else {
      wsStatus.set('disconnected')
    }
  }
}

function scheduleReconnect() {
  if (reconnectTimer) return
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    if (shouldReconnect) {
      reconnectDelay = Math.min(reconnectDelay * 2, 30000)
      connect()
    }
  }, reconnectDelay)
}

export function disconnect() {
  shouldReconnect = false
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  if (ws) {
    ws.close()
    ws = null
  }
  wsStatus.set('disconnected')
}

export function clearLog() {
  eventLog.set([])
  try { localStorage.removeItem(STORAGE_KEY) } catch (e) { /* ignore */ }
}

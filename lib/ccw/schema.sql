-- substrate-ccw persistence schema (SQLite, WAL mode set on open).
-- Append-only: add new tables / `ALTER TABLE ... IF NOT EXISTS`-style columns
-- at the bottom. Applied on every `Ccw::open` via execute_batch.

-- Accounts observed via `claude auth status`, keyed by registry name.
-- `config_dir` is NULL/empty for the machine default (CLAUDE_CONFIG_DIR unset).
CREATE TABLE IF NOT EXISTS accounts_seen (
    name              TEXT PRIMARY KEY,
    config_dir        TEXT,
    subscription_type TEXT,
    email             TEXT,
    logged_in         INTEGER NOT NULL DEFAULT 0,
    last_checked_at   TEXT
);

-- Budgets: schema-versioned rules JSON (v1 documented in budget.rs).
CREATE TABLE IF NOT EXISTS budgets (
    id         TEXT PRIMARY KEY,
    rules_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Every wrapped `claude` invocation attributed to a budget.
CREATE TABLE IF NOT EXISTS invocations (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    budget_id             TEXT,
    account               TEXT,
    session_id            TEXT,
    cwd                   TEXT,
    started_at            TEXT,
    ended_at              TEXT,
    usage_json            TEXT,   -- per-model usage blob (JSON)
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens          INTEGER NOT NULL DEFAULT 0,
    cost_usd              REAL NOT NULL DEFAULT 0,
    limit_hit             INTEGER NOT NULL DEFAULT 0,
    reset_prose           TEXT,
    exit_code             INTEGER
);
CREATE INDEX IF NOT EXISTS idx_invocations_budget ON invocations(budget_id);
CREATE INDEX IF NOT EXISTS idx_invocations_account ON invocations(account);

-- Raw `/usage` pool observations. Pool names stored as-reported strings.
CREATE TABLE IF NOT EXISTS limit_observations (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account      TEXT NOT NULL,
    pool         TEXT NOT NULL,
    pct          REAL NOT NULL,
    reset_at_raw TEXT,
    observed_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_obs_account_pool ON limit_observations(account, pool);

-- Learned tokens-per-percent calibration per (account, pool).
CREATE TABLE IF NOT EXISTS calibrations (
    account            TEXT NOT NULL,
    pool               TEXT NOT NULL,
    tokens_per_percent REAL NOT NULL,
    updated_at         TEXT,
    sample_count       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, pool)
);

-- ── v1 daemon (INTENT #208) ─────────────────────────────────────────────
-- CCW v1 owns claude invocation; sessions (threads) and their full event
-- streams are persisted here so the WS `sessions.history`/resume surfaces
-- serve from CCW's own event log (not from claude's on-disk transcripts).

-- One row per CCW session (thread). `claude_session_id` is the uuid CCW mints
-- and passes to `--session-id` / `--resume`. `spec_json` is the SessionSpec
-- (cwd, model, account, budget, tool config, options) captured at start.
CREATE TABLE IF NOT EXISTS sessions (
    id                TEXT PRIMARY KEY,   -- CCW session id (== claude session uuid)
    account           TEXT,
    cwd               TEXT,
    model             TEXT,
    budget_id         TEXT,
    spec_json         TEXT NOT NULL,      -- full SessionSpec blob
    status            TEXT NOT NULL DEFAULT 'idle',
    last_seq          INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account);

-- The CCW event log: every CcwEvent, monotonically sequenced per session.
-- (session_id, seq) is the resume/paging cursor the WS history call reads.
CREATE TABLE IF NOT EXISTS events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    kind         TEXT NOT NULL,           -- CcwEvent variant tag (for cheap filtering)
    payload_json TEXT NOT NULL,           -- the full serialized CcwEvent
    ts           TEXT NOT NULL,
    UNIQUE (session_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_events_session_seq ON events(session_id, seq);

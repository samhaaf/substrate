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

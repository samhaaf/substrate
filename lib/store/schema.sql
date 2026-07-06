-- substrate SQLite schema
-- Apply on every Store::open (all statements are idempotent).
-- Append-only: add new columns/tables at the bottom using ALTER TABLE IF NOT EXISTS.

PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

-- ---------------------------------------------------------------------------
-- completions
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS completions (
    id                   TEXT    PRIMARY KEY,
    model_id             TEXT    NOT NULL,
    prompt               TEXT    NOT NULL,
    params_json          TEXT    NOT NULL DEFAULT '{}',
    priority             INTEGER NOT NULL DEFAULT 0,
    preemption_threshold INTEGER NOT NULL DEFAULT 2,
    json_schema          TEXT,
    collection_id        TEXT    REFERENCES collections(id),
    metrics_flags        INTEGER NOT NULL DEFAULT 0,
    metadata_json        TEXT,

    state                TEXT    NOT NULL DEFAULT 'pending',
    preemption_count     INTEGER NOT NULL DEFAULT 0,
    error_retry_count    INTEGER NOT NULL DEFAULT 0,

    created_at           TEXT    NOT NULL,
    started_at           TEXT,
    completed_at         TEXT,
    recovered_at         TEXT                         -- set on crash-recovery requeue
);

CREATE INDEX IF NOT EXISTS idx_completions_state    ON completions(state);
CREATE INDEX IF NOT EXISTS idx_completions_priority ON completions(priority DESC, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_completions_model    ON completions(model_id, state);

-- ---------------------------------------------------------------------------
-- collections
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS collections (
    id                          TEXT    PRIMARY KEY,
    name                        TEXT    NOT NULL,
    description                 TEXT,
    cancel_on_failure           INTEGER NOT NULL DEFAULT 0,
    request_full_system         INTEGER NOT NULL DEFAULT 0,
    start_with_no_model_loaded  INTEGER NOT NULL DEFAULT 0,
    save_partial_results        INTEGER NOT NULL DEFAULT 1,
    metadata_json               TEXT,

    state                       TEXT    NOT NULL DEFAULT 'active',
    created_at                  TEXT    NOT NULL,
    started_at                  TEXT,
    completed_at                TEXT
);

-- ---------------------------------------------------------------------------
-- results (completion output blobs — stored separately to keep completions lean)
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS results (
    completion_id       TEXT    PRIMARY KEY REFERENCES completions(id),
    text                TEXT,
    termination_json    TEXT,
    metrics_json        TEXT,
    stored_at           TEXT    NOT NULL
);

-- ---------------------------------------------------------------------------
-- models
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS models (
    id                  TEXT    PRIMARY KEY,
    source              TEXT    NOT NULL,
    prompt_template     TEXT    NOT NULL,
    context_length      INTEGER NOT NULL,
    n_gpu_layers        INTEGER,
    max_slots           INTEGER,

    is_downloaded       INTEGER NOT NULL DEFAULT 0,
    is_loaded           INTEGER NOT NULL DEFAULT 0,
    file_path           TEXT,
    file_bytes          INTEGER,

    registered_at       TEXT    NOT NULL,
    last_used_at        TEXT
);

-- ---------------------------------------------------------------------------
-- benchmark_runs
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS benchmark_runs (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    model_id            TEXT    NOT NULL REFERENCES models(id),
    collection_id       TEXT    REFERENCES collections(id),
    prompt_tokens       INTEGER NOT NULL,
    max_tokens          INTEGER NOT NULL,
    concurrency         INTEGER NOT NULL,
    tokens_per_second   REAL,
    wall_time_ms        INTEGER,
    recorded_at         TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_benchmark_model ON benchmark_runs(model_id, recorded_at DESC);

-- ---------------------------------------------------------------------------
-- kv_cache_entries
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS kv_cache_entries (
    id                  TEXT    PRIMARY KEY,
    model_id            TEXT    NOT NULL REFERENCES models(id),
    prompt_hash         TEXT    NOT NULL,
    file_path           TEXT    NOT NULL,
    bytes               INTEGER NOT NULL DEFAULT 0,
    hit_count           INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL,
    last_accessed_at    TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_kv_cache_model     ON kv_cache_entries(model_id);
CREATE INDEX IF NOT EXISTS idx_kv_cache_prompt    ON kv_cache_entries(prompt_hash);
CREATE INDEX IF NOT EXISTS idx_kv_cache_lru       ON kv_cache_entries(last_accessed_at ASC);

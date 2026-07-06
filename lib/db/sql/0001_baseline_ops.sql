-- db's own control-plane baseline (Postgres: cloud + local). design §3.1.
-- Shipped and applied like any other migration. The `ops` schema is the SINGLE,
-- un-cloned control plane; every per-handler/per-dispatch/per-audit row carries `env`.

CREATE SCHEMA IF NOT EXISTS ops;

-- ── 3.1.a  Migration ledger — THE sole source of truth (H6 + C1 + M2) ────────
CREATE SEQUENCE IF NOT EXISTS ops.applied_seq_seq;

CREATE TABLE IF NOT EXISTS ops.applied_migrations (
  env          text        NOT NULL DEFAULT '',
  schema       text        NOT NULL,
  id           text        NOT NULL,
  checksum     text        NOT NULL,
  applied_seq  bigint      NOT NULL DEFAULT nextval('ops.applied_seq_seq'),
  applied_at   timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (env, id)
);
CREATE INDEX IF NOT EXISTS applied_migrations_env_seq
  ON ops.applied_migrations (env, applied_seq);

-- ── 3.1.b  Crawl attestations (M2) ───────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ops.crawl_attestations (
  schema       text        NOT NULL,
  id           text        NOT NULL,
  checksum     text        NOT NULL,
  crawled_at   timestamptz NOT NULL DEFAULT now(),
  crawled_by   text,
  PRIMARY KEY (id, checksum)
);

-- ── 3.1.c  Logical handlers ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ops.handlers (
  name         text PRIMARY KEY,
  description  text,
  created_at   timestamptz NOT NULL DEFAULT now()
);

-- ── 3.1.d  Immutable, versioned implementations ──────────────────────────────
CREATE TABLE IF NOT EXISTS ops.handler_versions (
  id            bigserial PRIMARY KEY,
  handler       text NOT NULL REFERENCES ops.handlers(name),
  version       text NOT NULL,
  kind          text NOT NULL CHECK (kind IN ('sql','edge')),
  invocation    text NOT NULL DEFAULT 'effect-async'
                  CHECK (invocation IN ('effect-async','validator-sync')),
  blocking      boolean NOT NULL DEFAULT false,
  target_ref    text NOT NULL,
  contract      jsonb NOT NULL,
  source_hash   text NOT NULL,
  timeout_ms    int,
  fail_policy   text NOT NULL DEFAULT 'fail-closed'
                  CHECK (fail_policy IN ('fail-closed','fail-open')),
  breaker       boolean NOT NULL DEFAULT false,
  created_at    timestamptz NOT NULL DEFAULT now(),
  UNIQUE (handler, version)
);

-- ── 3.1.e  Which version is ACTIVE — keyed by (handler, env) (M1) ────────────
CREATE TABLE IF NOT EXISTS ops.handler_active (
  handler                 text NOT NULL REFERENCES ops.handlers(name),
  env                     text NOT NULL DEFAULT '',
  version_id              bigint NOT NULL REFERENCES ops.handler_versions(id),
  activated_by_env        text NOT NULL DEFAULT '',
  activated_by_migration  text,
  activated_at            timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (handler, env),
  FOREIGN KEY (activated_by_env, activated_by_migration)
    REFERENCES ops.applied_migrations(env, id)
);

-- ── 3.1.f  Edge deploy registry ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ops.edge_deploys (
  slug          text PRIMARY KEY,
  handler       text NOT NULL REFERENCES ops.handlers(name),
  version       text NOT NULL,
  env           text NOT NULL DEFAULT '',
  source_hash   text NOT NULL,
  bundle_ref    text NOT NULL,
  deployed_at   timestamptz NOT NULL DEFAULT now(),
  deployed_by   text,
  UNIQUE (handler, version, env)
);

-- ── 3.1.g  Transactional outbox (§8) ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ops.handler_dispatch (
  id              bigserial PRIMARY KEY,
  env             text NOT NULL DEFAULT '',
  handler         text NOT NULL,
  version_id      bigint NOT NULL REFERENCES ops.handler_versions(id),
  payload         jsonb NOT NULL,
  idempotency_key text NOT NULL,
  status          text NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','fired','failed','done','quarantined')),
  enqueued_at     timestamptz NOT NULL DEFAULT now(),
  fired_at        timestamptz,
  attempts        int NOT NULL DEFAULT 0,
  last_error      text
);
CREATE INDEX IF NOT EXISTS handler_dispatch_status_env
  ON ops.handler_dispatch (status, env) WHERE status IN ('pending','failed');

-- ── 3.1.h  Validator audit ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ops.validator_audit (
  id              bigserial PRIMARY KEY,
  env             text NOT NULL DEFAULT '',
  handler         text NOT NULL,
  version_id      bigint NOT NULL REFERENCES ops.handler_versions(id),
  decision        text NOT NULL CHECK (decision IN ('allow','deny')),
  reason          text,
  code            text,
  latency_ms      int,
  timed_out       boolean NOT NULL DEFAULT false,
  policy_applied  text,
  breaker_state   text,
  idempotency_key text,
  at              timestamptz NOT NULL DEFAULT now()
);

-- ── current_env() resolution (§3.3) ──────────────────────────────────────────
CREATE OR REPLACE FUNCTION ops.current_env() RETURNS text
  LANGUAGE sql STABLE AS $$
  SELECT COALESCE(current_setting('app.db_env', true), '')
$$;

-- ── call_handler(name, params) — the single indirection (§3.3, §8.1) ──────────
-- Resolves the ACTIVE version for (handler, current_env()), falling back to the
-- prod (handler,'') row when the stage has no override (§3.3 [DEFAULT]). Branches on
-- KIND: `sql` → EXECUTE the registered function (target_ref) and return its jsonb;
-- `edge` → for effect-async, write a transactional-outbox intent (in the caller's txn,
-- §8.1) and return void-jsonb; validator-sync edge dispatch (the http round-trip) is
-- emitted by codegen at the trigger layer, so call_handler's edge branch here is the
-- async enqueue path. Returns jsonb (validator_result for validators; NULL for async).
CREATE OR REPLACE FUNCTION ops.call_handler(p_name text, p_params jsonb)
  RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
  v         ops.handler_versions%ROWTYPE;
  v_env     text := ops.current_env();
  v_key     text;
  v_result  jsonb;
BEGIN
  -- Resolve active version for (handler, env), falling back to prod ('') (§3.3).
  SELECT hv.* INTO v
    FROM ops.handler_active ha
    JOIN ops.handler_versions hv ON hv.id = ha.version_id
   WHERE ha.handler = p_name
     AND ha.env = (
       SELECT ha2.env FROM ops.handler_active ha2
        WHERE ha2.handler = p_name AND ha2.env IN (v_env, '')
        ORDER BY (ha2.env = v_env) DESC LIMIT 1
     );
  IF NOT FOUND THEN
    RAISE EXCEPTION 'call_handler: no active version for handler % in env %', p_name, v_env
      USING ERRCODE = 'no_data_found';
  END IF;

  IF v.kind = 'sql' THEN
    -- Dispatch to the registered SQL function (target_ref), passing the jsonb params.
    EXECUTE format('SELECT %s($1)', v.target_ref) INTO v_result USING p_params;
    RETURN v_result;
  ELSIF v.kind = 'edge' AND v.invocation = 'effect-async' THEN
    -- Transactional outbox enqueue (§8.1): the intent rides the caller's txn.
    -- idempotency_key: templated from the contract if resolvable, else synthetic
    -- sha256(handler‖version_id‖payload‖clock) fallback (H5, §4.4).
    v_key := ops.render_idem_key(v.contract->>'idempotency_key', p_params);
    IF v_key IS NULL OR v_key = '' THEN
      v_key := encode(sha256(convert_to(
        p_name || ':' || v.id::text || ':' || p_params::text || ':' || clock_timestamp()::text,
        'UTF8')), 'hex');
    END IF;
    INSERT INTO ops.handler_dispatch (env, handler, version_id, payload, idempotency_key)
      VALUES (v_env, p_name, v.id, p_params, v_key);
    RETURN NULL;  -- async: no synchronous result
  ELSE
    -- validator-sync edge dispatch is handled at the codegen'd trigger via the http
    -- extension; a bare call_handler on it here is not the sync path.
    RAISE EXCEPTION 'call_handler: handler % (kind=%, invocation=%) has no synchronous SQL dispatch path',
      p_name, v.kind, v.invocation USING ERRCODE = 'feature_not_supported';
  END IF;
END $$;

-- Render an idempotency_key template like '{user_id}:{org_id}' over a jsonb payload.
-- Substitutes each {key} with params->>'key'; returns '' if any referenced key is null.
CREATE OR REPLACE FUNCTION ops.render_idem_key(p_template text, p_params jsonb)
  RETURNS text LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
  m    text;
  out  text := p_template;
  val  text;
BEGIN
  IF p_template IS NULL OR p_template = '' THEN
    RETURN NULL;
  END IF;
  FOR m IN SELECT (regexp_matches(p_template, '\{([^}]+)\}', 'g'))[1] LOOP
    val := p_params->>m;
    IF val IS NULL THEN
      RETURN '';  -- templates-to-empty → caller uses synthetic fallback (H5).
    END IF;
    out := replace(out, '{' || m || '}', val);
  END LOOP;
  RETURN out;
END $$;

-- ── assert_row_shape(schema, table, row) — validator return-side guard (M4, §8.6) ──
-- Checks every key in `row` is a REAL column of the target table AND its json value is
-- coercible to the column's type; RAISEs on an unknown key or a type mismatch (rejects
-- the write rather than silently coercing / privilege-escalating).
CREATE OR REPLACE FUNCTION ops.assert_row_shape(p_schema text, p_table text, p_row jsonb)
  RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  k        text;
  v        jsonb;
  col_type text;
BEGIN
  FOR k, v IN SELECT * FROM jsonb_each(p_row) LOOP
    SELECT c.data_type INTO col_type
      FROM information_schema.columns c
     WHERE c.table_schema = p_schema AND c.table_name = p_table AND c.column_name = k;
    IF col_type IS NULL THEN
      RAISE EXCEPTION 'assert_row_shape: unknown column % on %.%', k, p_schema, p_table
        USING ERRCODE = 'undefined_column';
    END IF;
    -- Type coercibility probe: attempt the cast; a failure RAISEs (caught → mismatch).
    BEGIN
      IF jsonb_typeof(v) <> 'null' THEN
        EXECUTE format('SELECT ($1 #>> ''{}'')::%s', col_type) USING v;
      END IF;
    EXCEPTION WHEN others THEN
      RAISE EXCEPTION 'assert_row_shape: value for % not coercible to % on %.%',
        k, col_type, p_schema, p_table USING ERRCODE = 'datatype_mismatch';
    END;
  END LOOP;
END $$;

-- sqlite `ops` baseline (design §3.4) — MINIMAL, ledger-only. v1 sqlite is
-- introspect / query / SQL-migration only; no registry/outbox/audit/edge (those raise
-- NotImplemented). No schemas in sqlite → the `ops.` prefix becomes `ops_`.

CREATE TABLE IF NOT EXISTS ops_applied_migrations (
  env         TEXT    NOT NULL DEFAULT '',
  schema      TEXT    NOT NULL,
  id          TEXT    NOT NULL,
  checksum    TEXT    NOT NULL,
  applied_seq INTEGER NOT NULL,
  applied_at  TEXT    NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (env, id)
);

-- =============================================================================
-- V0027: authoring-run event journal (ADR 0002, evolution §3.4) — the
-- append-only per-run timeline. Every observable run transition (draft, verify,
-- revise, commit, mine, annotate, summarize, checkpoint, status change) appends
-- one row here AFTER the state change commits; the journal is the streamable
-- timeline view, never the source of truth (that stays the authoring_run tables
-- — ADR D3.3). SSE delivers each row over `/events?topic=run:<id>` with `seq`
-- as the resume token (ADR D4).
--
-- Additive and optional: a run that predates V0027 upgrades cleanly with zero
-- rows, and a run whose observability plumbing is never exercised is entirely
-- unaffected. This is a NEW table only — no existing column changes.
--
-- Append-only by contract (ADR D1): the repository exposes append + list only,
-- with NO update or delete path. Rows ride the run's own lifecycle — the FK
-- CASCADE takes a purged run's timeline with it.
--
-- Storage-class note: `created_at` is INTEGER unix microseconds, matching every
-- sibling table (authoring_run, canon_delta, quantity_state) and the
-- `row::time` / `timestamp_to_micros` house helpers. The ADR D1 sketch writes
-- the abstract "unix micros, house convention" comment on this column; the
-- binding contract is the column set and the append-only lifecycle, not
-- SQLite's storage class.
--
--   * id               — `authoring_run_event:*` ULID, prefix-CHECKed like every
--     sibling id column.
--   * authoring_run_id — FK CASCADE: a deleted run takes its journal with it
--     (the timeline is observational, not history worth orphaning — ADR D1).
--   * seq              — 1-based, DENSE per run. Assigned at append under the
--     single-writer connection discipline (SqlitePool serializes all writes
--     through one dedicated writer thread, so MAX(seq)+1 within one write
--     closure is race-free). UNIQUE(authoring_run_id, seq) is the resume-token
--     integrity guard (ADR D3.4).
--   * kind             — the ADR D2 vocabulary (validated app-side against a
--     closed set; not a DB CHECK so additive kinds ship without a migration —
--     ADR D3.2).
--   * payload          — typed JSON, json_valid-CHECKed like every JSON column.
--     Ids/paths/counts/enums only — NEVER prose, fact/secret text, evidence, or
--     model output (ADR D3.1).
-- =============================================================================

CREATE TABLE authoring_run_event (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'authoring_run_event:%'),
    authoring_run_id  TEXT    NOT NULL REFERENCES authoring_run(id) ON DELETE CASCADE,
    seq               INTEGER NOT NULL,
    kind              TEXT    NOT NULL,
    payload           TEXT    NOT NULL CHECK (json_valid(payload)),
    created_at        INTEGER NOT NULL,
    UNIQUE (authoring_run_id, seq)
);

-- Drives the SSE replay + list read: "this run's events in seq order, optionally
-- after a resume token" (list_run_events filtered by run, ordered by seq). The
-- UNIQUE(authoring_run_id, seq) index above already covers this ordering, but an
-- explicit named index documents the read intent and keeps the plan stable.
CREATE INDEX idx_authoring_run_event_run_seq
    ON authoring_run_event(authoring_run_id, seq);

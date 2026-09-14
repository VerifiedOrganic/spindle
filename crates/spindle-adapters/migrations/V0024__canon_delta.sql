-- =============================================================================
-- V0024: canon-delta staging (ADR 0001, evolution §3.1) — the ratification
-- queue. Every committed scene is mined into *proposed* deltas the operator
-- ratifies, inverting Spindle's bookkeeping economy from hand-authored tool
-- calls to reviewed diffs.
--
-- Additive and optional: a project that never mines a scene is entirely
-- unaffected (no rows). This is a NEW table only — no existing column changes —
-- so a database created before V0024 upgrades cleanly and its rows keep
-- deserializing byte-identically (ADR reversal-cost: additions are additive).
--
-- Storage-class note: `created_at`/`updated_at`/`decided_at` are INTEGER unix
-- microseconds, matching every sibling table (quantity_state, character_state,
-- authoring_run) and the `row::time` / `timestamp_to_micros` house helpers. The
-- ADR D2 sketch writes `TEXT` for these as the abstract "timestamp" notation
-- (it likewise writes `payload (typed JSON)` where the real storage class is
-- TEXT); the binding contract is the column SET and the lifecycle, not SQLite's
-- storage class. The spindle-core `CanonDelta` read model still exposes them as
-- ISO-8601 strings at the adapter boundary (mirrors SessionActivity).
--
--   * id            — `canon_delta:*` ULID, prefix-CHECKed like every sibling.
--   * project_id/branch_id — FK CASCADE (a deleted project/branch takes its
--     staged deltas with it; they are proposals, not history worth orphaning).
--   * scene_id      — provenance: the scene mined from. FK CASCADE: a deleted
--     scene's proposals are meaningless.
--   * authoring_run_id — NULL when mined outside a run. FK SET NULL so a purged
--     run leaves the decided-audit trail intact (the delta itself is history).
--   * delta_class   — one of the fourteen v1 classes (validated in the app at
--     the staging boundary against spindle-core::CANON_DELTA_CLASSES; not a DB
--     CHECK, so additive classes ship without a migration per the ADR).
--   * target_id     — existing entity this modifies; NULL proposes a new one.
--   * payload       — typed per-class JSON, json_valid-CHECKed like every JSON
--     column in the schema.
--   * evidence      — sanitized prose excerpt (≤300 chars, enforced app-side);
--     mandatory and NOT NULL — a delta with no quotable evidence is not
--     stageable (ADR D2).
--   * confidence    — high | medium | low (CHECK-constrained: a small closed
--     vocabulary, unlike the additive class set).
--   * status        — staged | applied | rejected | superseded (CHECK), default
--     'staged'.
--   * decided_at/by — ratification audit; NULL until a decision is recorded.
-- =============================================================================

CREATE TABLE canon_delta (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'canon_delta:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)       ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id)  ON DELETE CASCADE,
    scene_id         TEXT    NOT NULL REFERENCES scene(id)         ON DELETE CASCADE,
    authoring_run_id TEXT             REFERENCES authoring_run(id) ON DELETE SET NULL,
    delta_class      TEXT    NOT NULL,
    target_id        TEXT,
    payload          TEXT    NOT NULL CHECK (json_valid(payload)),
    evidence         TEXT    NOT NULL,
    confidence       TEXT    NOT NULL CHECK (confidence IN ('high', 'medium', 'low')),
    status           TEXT    NOT NULL DEFAULT 'staged'
                             CHECK (status IN ('staged', 'applied', 'rejected', 'superseded')),
    decided_at       INTEGER,
    decided_by       TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- Drives the ratify-queue read: "the staged deltas on this branch"
-- (list_canon_deltas filtered by project/branch/status).
CREATE INDEX idx_canon_delta_project_branch_status
    ON canon_delta(project_id, branch_id, status);

-- Drives the supersede-on-remine sweep and per-scene queue view
-- (supersede_scene_deltas / list_canon_deltas filtered by scene).
CREATE INDEX idx_canon_delta_scene_status
    ON canon_delta(scene_id, status);

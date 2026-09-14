-- =============================================================================
-- V0029: plan-amendment staging (ADR 0003, evolution §3.5) — the living-outline
-- ratification queue. After each chapter summary a replan pass compares realized
-- reality (summaries, promise events, beat annotations, arc deltas) against the
-- *not-yet-drafted* chapters' plans and stages amendment proposals the operator
-- ratifies. Mirrors `canon_delta` (V0024) shape and lifecycle exactly (ADR D2):
-- staged rows persist, decide tools render them, applied amendments become the
-- recoverable history of the outline.
--
-- Additive and optional: a project that never runs a replan pass is entirely
-- unaffected (no rows). The table is a NEW table plus one NULLable ADD COLUMN on
-- `chapter_plan` — no existing column changes — so a database created before
-- V0029 upgrades cleanly and its rows keep deserializing byte-identically (ADR
-- reversal-cost: additions are additive).
--
-- Storage-class note: `created_at`/`updated_at`/`decided_at` are INTEGER unix
-- microseconds, matching every sibling table (canon_delta, quantity_state,
-- authoring_run) and the `row::time` / `timestamp_to_micros` house helpers. The
-- ADR D2 sketch writes the abstract "timestamp" notation (and "payload (typed
-- JSON)" where the real storage class is TEXT); the binding contract is the
-- column SET and the lifecycle, not SQLite's storage class. The spindle-core
-- `PlanAmendment` read model exposes them as ISO-8601 strings at the adapter
-- boundary (mirrors CanonDelta / SessionActivity).
--
--   * id             — `plan_amendment:*` ULID, prefix-CHECKed like every sibling.
--   * project_id/branch_id — FK CASCADE (a deleted project/branch takes its
--     staged amendments with it; they are proposals, not history worth orphaning).
--   * source_chapter — provenance: the summarized chapter that triggered the
--     replan pass. Not an FK (chapters are addressed by (book, number), not a
--     row id, throughout the plan surface).
--   * book_number    — the book the source/target chapter numbers belong to.
--   * authoring_run_id — NULL when replanned outside a run. FK SET NULL so a
--     purged run leaves the decided-audit trail intact (the amendment is history).
--   * amendment_class — one of the eight v1 classes (validated app-side at the
--     staging boundary against spindle-core::PLAN_AMENDMENT_CLASSES; not a DB
--     CHECK, so additive classes ship without a migration per the ADR).
--   * target_chapter — the future chapter this amends. NULL ONLY for
--     `promise_followup` (which targets a future placement, not a chapter row);
--     required for every other class. Enforced in staging validation, not SQL —
--     the rule is class-conditional, which a column CHECK cannot express.
--   * payload        — typed per-class JSON, json_valid-CHECKed like every JSON
--     column in the schema. Deserializes into the plan-write inputs the apply
--     dispatcher (Part B) replays.
--   * rationale      — the replanner's stated reasoning (ids/summaries only, no
--     prose quotes — ADR D2); mandatory, non-empty, ≤500 chars enforced app-side.
--   * confidence     — high | medium | low (CHECK-constrained: a small closed
--     vocabulary, unlike the additive class set).
--   * status         — staged | applied | rejected | superseded (CHECK), default
--     'staged'.
--   * decided_at/by  — ratification audit; NULL until a decision is recorded.
--   * prior_state    — ADR D4: at apply time the affected plan slice is
--     snapshotted here before the write, so outline history = the ordered applied
--     amendments with their prior states (no separate history table). NULL until
--     an apply captures it (Part B).
-- =============================================================================

CREATE TABLE plan_amendment (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'plan_amendment:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)       ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id)  ON DELETE CASCADE,
    source_chapter   INTEGER NOT NULL,
    book_number      INTEGER NOT NULL,
    authoring_run_id TEXT             REFERENCES authoring_run(id) ON DELETE SET NULL,
    amendment_class  TEXT    NOT NULL,
    target_chapter   INTEGER,
    payload          TEXT    NOT NULL CHECK (json_valid(payload)),
    rationale        TEXT    NOT NULL,
    confidence       TEXT    NOT NULL CHECK (confidence IN ('high', 'medium', 'low')),
    status           TEXT    NOT NULL DEFAULT 'staged'
                             CHECK (status IN ('staged', 'applied', 'rejected', 'superseded')),
    decided_at       INTEGER,
    decided_by       TEXT,
    prior_state      TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

-- Drives the ratify-queue read: "the staged amendments on this branch"
-- (list_plan_amendments filtered by project/branch/status).
CREATE INDEX idx_plan_amendment_project_branch_status
    ON plan_amendment(project_id, branch_id, status);

-- Drives the supersede-on-replan sweep and per-source-chapter queue view
-- (supersede_source_chapter_amendments / list_plan_amendments filtered by the
-- triggering chapter).
CREATE INDEX idx_plan_amendment_book_source_status
    ON plan_amendment(book_number, source_chapter, status);

-- ADR D4: outline history via per-chapter plan-revision counter. NULL = 0,
-- incremented by the apply dispatcher (Part B) each time an amendment rewrites
-- the chapter's plan. Additive NULLable column — pre-V0029 plans read as 0.
ALTER TABLE chapter_plan ADD COLUMN plan_revision INTEGER;

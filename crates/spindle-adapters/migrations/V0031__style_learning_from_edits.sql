-- =============================================================================
-- V0031: style learning from operator edits (evolution §3.9, P5.4).
--
-- When a scene the agent drafted is re-saved by the OPERATOR with different
-- prose, the before/after pair is captured as a *candidate* that feeds the
-- EXISTING style-profile refresh flow (preview_refresh_style_profile →
-- refresh_style_profile). Opt-in per project; candidates are reviewable before
-- any profile refresh (evolution I4 — nothing flows into a profile without the
-- operator running refresh, which is already an explicit action).
--
-- Both additions are optional and additive: a database created before V0031
-- upgrades cleanly and every existing row deserializes byte-identically,
-- because NULL/absent is the disabled/empty state.
--
--   * project.style_learning — NULL (pre-upgrade + default) = learning DISABLED
--     for the project; no edit is ever captured. Enabled by setting it to a
--     truthy integer (1) through the existing update_entity column path (the
--     column is added to the update allowlist). NULL, not a DB CHECK, so the
--     opt-in vocabulary can grow without a migration. Mirrors V0025's
--     authoring_run.mining_policy NULL-is-disabled convention.
--
--   * style_edit_candidate — one staged before/after pair per captured edit.
--     Additive and optional: a project that never opts in (or whose agent
--     drafts are never operator-edited) has no rows and is entirely unaffected.
--     Storage-class note: created_at/updated_at are INTEGER unix microseconds,
--     matching every sibling table (canon_delta, quantity_state, scene) and the
--     row::time / timestamp_to_micros house helpers.
--
--       * id            — `style_edit_candidate:*` ULID, prefix-CHECKed like
--         every sibling id.
--       * project_id/branch_id — FK CASCADE (a deleted project/branch takes its
--         candidates with it; they are proposals, not history worth orphaning).
--       * scene_id      — provenance: the scene the edit was made on. FK CASCADE
--         — a deleted scene's candidate is meaningless. Also the dedupe key:
--         at most one PENDING candidate per scene (the latest operator edit over
--         the same agent draft REPLACES the prior pending row).
--       * book_number/chapter_number/scene_order — placement provenance so the
--         preview can name the scene ref without re-reading the scene row.
--       * agent_draft   — the agent's prose BEFORE the operator edit (the
--         contrast example). Stored whole — scenes already live in the DB.
--       * operator_edit — the operator's prose AFTER the edit (the positive
--         example fed to refresh). Stored whole.
--       * content_rating — the scene's rating at capture, lowercased. Drives the
--         source-side rating discipline (evolution §4): an explicit candidate is
--         withheld from refresh unless the style route's resolved agent declares
--         explicit.
--       * status        — pending | consumed | dismissed (CHECK), default
--         'pending'. `consumed` once a refresh has fed it into a profile;
--         `dismissed` when the operator drops it.
--       * created_at/updated_at — INTEGER unix micros.
-- =============================================================================

ALTER TABLE project ADD COLUMN style_learning INTEGER;

CREATE TABLE style_edit_candidate (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'style_edit_candidate:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id        TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    scene_order     INTEGER NOT NULL,
    agent_draft     TEXT    NOT NULL,
    operator_edit   TEXT    NOT NULL,
    content_rating  TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending', 'consumed', 'dismissed')),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- Drives the refresh-feed read: "the pending candidates on this branch"
-- (preview_refresh_style_profile / refresh_style_profile filter by
-- project/branch/status).
CREATE INDEX idx_style_edit_candidate_project_branch_status
    ON style_edit_candidate(project_id, branch_id, status);

-- Drives the per-scene dedupe/replace sweep (one pending candidate per scene).
CREATE INDEX idx_style_edit_candidate_scene_status
    ON style_edit_candidate(scene_id, status);

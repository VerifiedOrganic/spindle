-- =============================================================================
-- V0046: per-(scene, source) relationship delta idempotency and branch-scoped
-- canon override uniqueness indexes.
--
-- 1. `relationship_scene_delta` records applied trust/tension deltas keyed by
--    `(branch_id, in_id, out_id, scene_id, source)` and story position so
--    recommitting a revised scene, retrying after failure, or applying a mined
--    `relationship_shift` on top of a drafter continuity package replaces the
--    scene's prior delta rather than accumulating.
-- 2. Scopes canon entity uniqueness indexes by `branch_id` so child branches
--    can override parent-branch canon rows without cross-branch UNIQUE
--    constraint collisions.
-- =============================================================================

CREATE TABLE relationship_scene_delta (
    branch_id      TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    in_id          TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    out_id         TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    scene_id       TEXT    NOT NULL,
    source         TEXT    NOT NULL DEFAULT 'scene',
    book_number    INTEGER NOT NULL,
    chapter_number INTEGER NOT NULL,
    scene_order    INTEGER NOT NULL,
    trust_delta    INTEGER NOT NULL,
    tension_delta  INTEGER NOT NULL,
    reason         TEXT,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (branch_id, in_id, out_id, scene_id, source)
);

CREATE INDEX idx_relationship_scene_delta_position
    ON relationship_scene_delta(branch_id, in_id, out_id, book_number, chapter_number, scene_order);

DROP INDEX IF EXISTS idx_world_rule_project_type_name;
CREATE UNIQUE INDEX idx_world_rule_project_type_name
    ON world_rule(project_id, branch_id, rule_type, rule_name);

DROP INDEX IF EXISTS idx_faction_project_name;
CREATE UNIQUE INDEX idx_faction_project_name
    ON faction(project_id, branch_id, normalized_name);

DROP INDEX IF EXISTS idx_religion_project_name;
CREATE UNIQUE INDEX idx_religion_project_name
    ON religion(project_id, branch_id, normalized_name);

DROP INDEX IF EXISTS idx_economy_project_name;
CREATE UNIQUE INDEX idx_economy_project_name
    ON economy(project_id, branch_id, normalized_name);

DROP INDEX IF EXISTS idx_term_project_name;
CREATE UNIQUE INDEX idx_term_project_name
    ON term(project_id, branch_id, normalized_term);

DROP INDEX IF EXISTS idx_plot_line_project_name;
CREATE UNIQUE INDEX idx_plot_line_project_name
    ON plot_line(project_id, branch_id, normalized_name);

DROP INDEX IF EXISTS idx_conflict_project_name;
CREATE UNIQUE INDEX idx_conflict_project_name
    ON conflict(project_id, branch_id, normalized_name);

DROP INDEX IF EXISTS idx_motif_project_name;
CREATE UNIQUE INDEX idx_motif_project_name
    ON motif(project_id, branch_id, normalized_name);

-- =============================================================================
-- V0020: quantity-continuity layer (per-project quantity schemes + stamped
-- per-subject quantity state). Money is the first vertical, but the primitive
-- is generic — LitRPG/cultivation stats and reputation reuse it.
--
-- Additive and optional: a project that never declares a quantity scheme is
-- entirely unaffected. Quantity state is append-only and position-stamped,
-- mirroring `character_state`; schemes are one row per (project, branch,
-- measure), mirroring `project_calendar`.
-- =============================================================================

-- One scheme per (project, branch, measure): ordered denominations + bands.
CREATE TABLE project_quantity_scheme (
    project_id    TEXT NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id     TEXT NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    measure       TEXT NOT NULL,
    denominations TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(denominations)),
    bands         TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(bands)),
    max_band_jump INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (project_id, branch_id, measure)
);

-- Append-only, position-stamped quantity reading for a subject's measure.
-- `band` is the primary signal; `amount`/`unit` are an optional refinement.
CREATE TABLE quantity_state (
    id             TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'quantity_state:%'),
    project_id     TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id      TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    subject_table  TEXT    NOT NULL,
    subject_id     TEXT    NOT NULL,
    measure        TEXT    NOT NULL,
    amount         REAL,
    unit           TEXT,
    band           TEXT,
    change_reason  TEXT,
    scene_id       TEXT,
    book_number    INTEGER NOT NULL,
    chapter_number INTEGER NOT NULL,
    scene_order    INTEGER NOT NULL,
    created_at     INTEGER NOT NULL
);

-- Drives the "current amount/band at or before cursor" read.
CREATE INDEX idx_quantity_state_subject
    ON quantity_state(project_id, branch_id, subject_table, subject_id, measure,
                      book_number, chapter_number, scene_order);

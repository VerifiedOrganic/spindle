-- =============================================================================
-- V0038: canonical_fact.scene_id becomes optional (planned-and-pending facts).
--
-- register_canonical_fact required a scene because scene_id was NOT NULL,
-- which made it impossible to register a fact decided during PLANNING but
-- not yet dramatised — e.g. a character name locked by author decision
-- before the scene that establishes it is written. The author's only
-- workaround was carrying the decision in prose fields and hoping every
-- drafting model reads them: exactly the soft constraint that produces drift
-- across hundreds of chapters.
--
-- The column is now nullable. A fact with NULL scene_id is
-- planned-and-pending: it still carries book_number/chapter_number (both
-- NOT NULL) as its planned placement, participates in consistency checks
-- from that placement onward, and can be bound to its scene later via
-- bind_canonical_fact_to_scene. The ON DELETE CASCADE FK to scene simply
-- does not apply to unbound rows.
--
-- SQLite cannot drop NOT NULL in place, so this is the standard rebuild:
-- create the new shape, copy rows, drop, rename, recreate indexes. Column
-- set and order are otherwise identical to V0001 + V0023 (secret,
-- concealment_note).
--
-- FOREIGN KEYS: pool.rs opens every connection with PRAGMA foreign_keys=ON,
-- and refinery wraps each migration in a transaction, so we cannot turn FKs
-- off here (the pragma is a no-op inside a transaction). SQLite DOES enforce
-- FKs inside a transaction. `knowledge_fact.secret_of_fact_id` (V0023)
-- references canonical_fact(id); DROP TABLE fails when any of those values
-- are non-null. Null the links, rebuild, then restore them. Empty databases
-- skip the backup tables and still succeed.
-- =============================================================================

CREATE TEMP TABLE knowledge_fact_secret_backup AS
SELECT id, secret_of_fact_id
FROM knowledge_fact
WHERE secret_of_fact_id IS NOT NULL;

UPDATE knowledge_fact
SET secret_of_fact_id = NULL
WHERE secret_of_fact_id IS NOT NULL;

CREATE TABLE canonical_fact_new (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'canonical_fact:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id        TEXT             REFERENCES scene(id)        ON DELETE CASCADE,
    source_scene_id TEXT             REFERENCES scene(id)         ON DELETE SET NULL,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    subject_table   TEXT    NOT NULL,
    -- Polymorphic record ref.
    subject_id      TEXT,
    predicate       TEXT    NOT NULL,
    value_kind      TEXT    NOT NULL,
    value_number    REAL,
    value_text      TEXT,
    value_json      TEXT             CHECK (value_json IS NULL OR json_valid(value_json)),
    unit            TEXT,
    aliases         TEXT    NOT NULL CHECK (json_valid(aliases)),
    scope           TEXT    NOT NULL,
    valid_from      TEXT             CHECK (valid_from IS NULL OR json_valid(valid_from)),
    valid_until     TEXT             CHECK (valid_until IS NULL OR json_valid(valid_until)),
    superseded_by   TEXT             REFERENCES canonical_fact(id) ON DELETE SET NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    secret          INTEGER NOT NULL DEFAULT 0,
    concealment_note TEXT
);

INSERT INTO canonical_fact_new (
    id, project_id, branch_id, scene_id, source_scene_id, book_number, chapter_number,
    subject_table, subject_id, predicate, value_kind, value_number, value_text, value_json,
    unit, aliases, scope, valid_from, valid_until, superseded_by, created_at, updated_at,
    secret, concealment_note
)
SELECT
    id, project_id, branch_id, scene_id, source_scene_id, book_number, chapter_number,
    subject_table, subject_id, predicate, value_kind, value_number, value_text, value_json,
    unit, aliases, scope, valid_from, valid_until, superseded_by, created_at, updated_at,
    secret, concealment_note
FROM canonical_fact;

DROP TABLE canonical_fact;
ALTER TABLE canonical_fact_new RENAME TO canonical_fact;

CREATE INDEX canonical_fact_subject_predicate_idx
    ON canonical_fact(project_id, branch_id, subject_table, subject_id, predicate);
CREATE INDEX canonical_fact_scope_idx
    ON canonical_fact(project_id, branch_id, scope);
CREATE INDEX idx_canonical_fact_scene  ON canonical_fact(scene_id);
CREATE INDEX idx_canonical_fact_branch ON canonical_fact(branch_id);

UPDATE knowledge_fact
SET secret_of_fact_id = (
    SELECT secret_of_fact_id
    FROM knowledge_fact_secret_backup
    WHERE knowledge_fact_secret_backup.id = knowledge_fact.id
)
WHERE id IN (SELECT id FROM knowledge_fact_secret_backup);

DROP TABLE knowledge_fact_secret_backup;

PRAGMA foreign_key_check;

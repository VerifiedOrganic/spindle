-- =============================================================================
-- V0056: Provider/dimension-aware embedding index generations (P2 PR H).
--
-- Replaces the single hard-coded FLOAT[64] vec0 table and its triggers with:
--   * a source table carrying generation_id + dimension (variable-length BLOB),
--   * a generation registry,
--   * per-project active/previous index state for atomic switch and rollback,
--   * one vec0 table per generation/dimension created by validated service code.
--
-- The legacy token-hash-v1 64-dimension data is preserved as generation
-- `token-hash-v1:64`; no vectors are padded, truncated or reinterpreted.
-- =============================================================================

DROP TRIGGER IF EXISTS trg_search_embedding_vec_insert;
DROP TRIGGER IF EXISTS trg_search_embedding_vec_update;
DROP TRIGGER IF EXISTS trg_search_embedding_vec_delete;
DROP TABLE IF EXISTS vec_search_embedding;

ALTER TABLE search_embedding RENAME TO search_embedding_pre_v56;

CREATE TABLE search_embedding (
    id                 TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'search_embedding:%'),
    project_id         TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id          TEXT             REFERENCES bible_branch(id) ON DELETE CASCADE,
    entity_table       TEXT    NOT NULL,
    entity_id          TEXT    NOT NULL,
    title              TEXT    NOT NULL,
    excerpt            TEXT    NOT NULL,
    content            TEXT    NOT NULL,
    generation_id      TEXT    NOT NULL,
    embedding_version  TEXT    NOT NULL,
    dimension          INTEGER NOT NULL CHECK (dimension > 0),
    embedding          BLOB    NOT NULL CHECK (length(embedding) = dimension * 4),
    updated_at         INTEGER NOT NULL
);

INSERT INTO search_embedding (
    id, project_id, branch_id, entity_table, entity_id, title, excerpt, content,
    generation_id, embedding_version, dimension, embedding, updated_at
)
SELECT
    id, project_id, branch_id, entity_table, entity_id, title, excerpt, content,
    'token-hash-v1:64', embedding_version, 64, embedding, updated_at
FROM search_embedding_pre_v56;

DROP TABLE search_embedding_pre_v56;

CREATE UNIQUE INDEX idx_search_embedding_entity
    ON search_embedding(project_id, branch_id, entity_table, entity_id, generation_id);

CREATE INDEX idx_search_embedding_generation
    ON search_embedding(project_id, generation_id, branch_id);

CREATE TABLE embedding_generation (
    generation_id     TEXT    PRIMARY KEY NOT NULL,
    provider          TEXT    NOT NULL CHECK(provider IN ('token_hash', 'http')),
    model             TEXT    NOT NULL,
    dimension         INTEGER NOT NULL CHECK(dimension > 0),
    metric            TEXT    NOT NULL CHECK(metric IN ('cosine', 'l2')),
    normalization     TEXT    NOT NULL CHECK(normalization IN ('l2', 'none')),
    version           TEXT    NOT NULL,
    table_name        TEXT    NOT NULL UNIQUE,
    status            TEXT    NOT NULL CHECK(status IN ('building', 'active', 'retired', 'failed')),
    created_at        INTEGER NOT NULL,
    activated_at      INTEGER,
    retired_at        INTEGER,
    rollback_until    INTEGER
);

CREATE TABLE embedding_index_state (
    project_id              TEXT    PRIMARY KEY NOT NULL
                                    REFERENCES project(id) ON DELETE CASCADE,
    active_generation_id    TEXT    NOT NULL
                                    REFERENCES embedding_generation(generation_id),
    previous_generation_id  TEXT
                                    REFERENCES embedding_generation(generation_id),
    rollback_until          INTEGER,
    updated_at              INTEGER NOT NULL
);

INSERT INTO embedding_generation (
    generation_id, provider, model, dimension, metric, normalization, version,
    table_name, status, created_at, activated_at
) VALUES (
    'token-hash-v1:64', 'token_hash', 'token-hash-v1', 64, 'cosine', 'l2',
    'token-hash-v1', 'vec_embedding_gen_token_hash_v1_64', 'active',
    CAST(strftime('%s','now') AS INTEGER) * 1000000,
    CAST(strftime('%s','now') AS INTEGER) * 1000000
);

INSERT INTO embedding_index_state (
    project_id, active_generation_id, previous_generation_id, rollback_until, updated_at
)
SELECT id, 'token-hash-v1:64', NULL, NULL,
       CAST(strftime('%s','now') AS INTEGER) * 1000000
FROM project;

-- Per-project lifecycle for a generation. The generation registry/spec and its
-- vec0 table are shared, but build/active/retired/failed status is per project:
-- project A activating a spec must not change project B's build or service.
CREATE TABLE embedding_generation_project (
    project_id      TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    generation_id   TEXT    NOT NULL REFERENCES embedding_generation(generation_id),
    status          TEXT    NOT NULL CHECK(status IN ('building', 'active', 'retired', 'failed')),
    created_at      INTEGER NOT NULL,
    activated_at    INTEGER,
    retired_at      INTEGER,
    failed_at       INTEGER,
    PRIMARY KEY (project_id, generation_id)
);

CREATE INDEX idx_embedding_generation_project_status
    ON embedding_generation_project(project_id, status);

INSERT INTO embedding_generation_project (
    project_id, generation_id, status, created_at, activated_at
)
SELECT id, 'token-hash-v1:64', 'active',
       CAST(strftime('%s','now') AS INTEGER) * 1000000,
       CAST(strftime('%s','now') AS INTEGER) * 1000000
FROM project;

CREATE VIRTUAL TABLE vec_embedding_gen_token_hash_v1_64 USING vec0(
    se_id   TEXT PRIMARY KEY,
    embedding FLOAT[64]
);

INSERT INTO vec_embedding_gen_token_hash_v1_64(se_id, embedding)
SELECT id, embedding FROM search_embedding WHERE generation_id = 'token-hash-v1:64';

-- Phase 5: vec0 mirror of `search_embedding` for kNN.
--
-- `search_embedding` keeps storing every row (used for diagnostic listing,
-- per-project filtering, and FTS5-style fallbacks). `vec_search_embedding`
-- is the dense kNN index — populated by AFTER triggers on the source table
-- so the application code never has to write to both.
--
-- The `vec0` extension creates a virtual table whose rowid binds dense vector
-- rows to a "partition" primary key. We use the search_embedding row id (a
-- TEXT 'search_embedding:ulid') as the partition column and store the BLOB
-- of 64 packed f32s alongside.
--
-- sqlite-vec is registered as an auto-extension by the SqlitePool, so the
-- USING vec0 syntax is available on every connection opened against the DB.

CREATE VIRTUAL TABLE vec_search_embedding USING vec0(
    se_id   TEXT PRIMARY KEY,
    embedding FLOAT[64]
);

-- INSERT mirror.
CREATE TRIGGER trg_search_embedding_vec_insert
AFTER INSERT ON search_embedding
BEGIN
    INSERT INTO vec_search_embedding(se_id, embedding) VALUES (NEW.id, NEW.embedding);
END;

-- UPDATE mirror — vec0 doesn't support direct UPDATE on a vector column in
-- all versions, so we DELETE then re-INSERT to keep behavior portable.
CREATE TRIGGER trg_search_embedding_vec_update
AFTER UPDATE OF embedding ON search_embedding
BEGIN
    DELETE FROM vec_search_embedding WHERE se_id = OLD.id;
    INSERT INTO vec_search_embedding(se_id, embedding) VALUES (NEW.id, NEW.embedding);
END;

-- DELETE mirror.
CREATE TRIGGER trg_search_embedding_vec_delete
AFTER DELETE ON search_embedding
BEGIN
    DELETE FROM vec_search_embedding WHERE se_id = OLD.id;
END;

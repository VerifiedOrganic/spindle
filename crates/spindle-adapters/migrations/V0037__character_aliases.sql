-- =============================================================================
-- V0037: character aliases (rename-without-rename + rename safety net).
--
-- Two authoring needs land on the same column:
--
--   * A character record often exists before the in-world moment where it is
--     named (the record must hold a voice profile, an arc, and relationships
--     before the naming scene is drafted). Aliases let the record keep its
--     stable working label while carrying the in-world name — no rename
--     needed at all. canonical_fact already carries an `aliases` column with
--     exactly this purpose; characters now match.
--
--   * When a character IS renamed (update_entity with allow_rename), the old
--     name is preserved here automatically so existing prose references and
--     prior knowledge stay resolvable to the record.
--
-- Column mechanics:
--
--   * character.aliases — JSON array of strings, NOT NULL DEFAULT '[]'. The
--     constant default keeps every pre-V0037 row, every branch-restore
--     insert from an older snapshot, and every INSERT that omits the column
--     reading back an empty list. Added to the update allowlist so authors
--     can amend aliases through update_entity after creation.
--
--   * fts_character gains an `aliases` column so find_entity's exact-name
--     path resolves a character by alias exactly as it does by name. FTS5
--     tables cannot ALTER, so the table and its three sync triggers are
--     rebuilt and backfilled; the JSON array tokenizes cleanly under the
--     unicode61 tokenizer (punctuation is a separator).
-- =============================================================================

ALTER TABLE character ADD COLUMN aliases TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(aliases));

DROP TRIGGER trg_fts_character_ai;
DROP TRIGGER trg_fts_character_ad;
DROP TRIGGER trg_fts_character_au;
DROP TABLE fts_character;

CREATE VIRTUAL TABLE fts_character USING fts5(
    character_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    name,
    summary,
    role,
    notes,
    appearance,
    aliases,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_character_ai AFTER INSERT ON character BEGIN
    INSERT INTO fts_character(character_id, project_id, branch_id, name, summary, role, notes, appearance, aliases)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.summary, NEW.role,
            COALESCE(NEW.notes, ''), COALESCE(NEW.appearance, ''), NEW.aliases);
END;
CREATE TRIGGER trg_fts_character_ad AFTER DELETE ON character BEGIN
    DELETE FROM fts_character WHERE character_id = OLD.id;
END;
CREATE TRIGGER trg_fts_character_au
AFTER UPDATE OF name, summary, role, notes, appearance, aliases, branch_id ON character BEGIN
    DELETE FROM fts_character WHERE character_id = OLD.id;
    INSERT INTO fts_character(character_id, project_id, branch_id, name, summary, role, notes, appearance, aliases)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.summary, NEW.role,
            COALESCE(NEW.notes, ''), COALESCE(NEW.appearance, ''), NEW.aliases);
END;

INSERT INTO fts_character(character_id, project_id, branch_id, name, summary, role, notes, appearance, aliases)
SELECT id, project_id, branch_id, name, summary, role,
       COALESCE(notes, ''), COALESCE(appearance, ''), aliases
FROM character;

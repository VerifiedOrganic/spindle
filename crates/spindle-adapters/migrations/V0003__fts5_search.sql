-- Phase 5 (part 2): FTS5 indexes over searchable text columns.
--
-- Mirrors the embedding-based kNN with a lexical search path. Each FTS5
-- virtual table holds the text fields a user would naturally search for an
-- entity; sync triggers on the source tables keep them current.
--
-- The schema is contentless (no `content=`) so updates require explicit
-- DELETE+INSERT in triggers, but it's the most portable shape and avoids
-- introducing computed columns on the source rows.

-- Scene prose + summary — by far the largest single index.
CREATE VIRTUAL TABLE fts_scene USING fts5(
    scene_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    summary,
    full_text,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_scene_ai AFTER INSERT ON scene BEGIN
    INSERT INTO fts_scene(scene_id, project_id, branch_id, summary, full_text)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.summary, NEW.full_text);
END;
CREATE TRIGGER trg_fts_scene_ad AFTER DELETE ON scene BEGIN
    DELETE FROM fts_scene WHERE scene_id = OLD.id;
END;
CREATE TRIGGER trg_fts_scene_au AFTER UPDATE OF summary, full_text, branch_id, project_id ON scene BEGIN
    DELETE FROM fts_scene WHERE scene_id = OLD.id;
    INSERT INTO fts_scene(scene_id, project_id, branch_id, summary, full_text)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.summary, NEW.full_text);
END;

-- Characters: name + summary + role + notes + appearance.
CREATE VIRTUAL TABLE fts_character USING fts5(
    character_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    name,
    summary,
    role,
    notes,
    appearance,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_character_ai AFTER INSERT ON character BEGIN
    INSERT INTO fts_character(character_id, project_id, branch_id, name, summary, role, notes, appearance)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.summary, NEW.role,
            COALESCE(NEW.notes, ''), COALESCE(NEW.appearance, ''));
END;
CREATE TRIGGER trg_fts_character_ad AFTER DELETE ON character BEGIN
    DELETE FROM fts_character WHERE character_id = OLD.id;
END;
CREATE TRIGGER trg_fts_character_au
AFTER UPDATE OF name, summary, role, notes, appearance, branch_id ON character BEGIN
    DELETE FROM fts_character WHERE character_id = OLD.id;
    INSERT INTO fts_character(character_id, project_id, branch_id, name, summary, role, notes, appearance)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.summary, NEW.role,
            COALESCE(NEW.notes, ''), COALESCE(NEW.appearance, ''));
END;

-- Locations: name + kind + summary + notes.
CREATE VIRTUAL TABLE fts_location USING fts5(
    location_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    name,
    kind,
    summary,
    notes,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_location_ai AFTER INSERT ON location BEGIN
    INSERT INTO fts_location(location_id, project_id, branch_id, name, kind, summary, notes)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.kind, NEW.summary, COALESCE(NEW.notes, ''));
END;
CREATE TRIGGER trg_fts_location_ad AFTER DELETE ON location BEGIN
    DELETE FROM fts_location WHERE location_id = OLD.id;
END;
CREATE TRIGGER trg_fts_location_au
AFTER UPDATE OF name, kind, summary, notes, branch_id ON location BEGIN
    DELETE FROM fts_location WHERE location_id = OLD.id;
    INSERT INTO fts_location(location_id, project_id, branch_id, name, kind, summary, notes)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.name, NEW.kind, NEW.summary, COALESCE(NEW.notes, ''));
END;

-- World rules: rule_name + description + notes.
CREATE VIRTUAL TABLE fts_world_rule USING fts5(
    world_rule_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    rule_name,
    description,
    notes,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_world_rule_ai AFTER INSERT ON world_rule BEGIN
    INSERT INTO fts_world_rule(world_rule_id, project_id, branch_id, rule_name, description, notes)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.rule_name, NEW.description, COALESCE(NEW.notes, ''));
END;
CREATE TRIGGER trg_fts_world_rule_ad AFTER DELETE ON world_rule BEGIN
    DELETE FROM fts_world_rule WHERE world_rule_id = OLD.id;
END;
CREATE TRIGGER trg_fts_world_rule_au
AFTER UPDATE OF rule_name, description, notes, branch_id ON world_rule BEGIN
    DELETE FROM fts_world_rule WHERE world_rule_id = OLD.id;
    INSERT INTO fts_world_rule(world_rule_id, project_id, branch_id, rule_name, description, notes)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.rule_name, NEW.description, COALESCE(NEW.notes, ''));
END;

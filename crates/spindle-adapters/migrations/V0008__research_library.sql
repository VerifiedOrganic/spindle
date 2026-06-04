-- =============================================================================
-- V0008: add project-local research library tables.
-- =============================================================================

CREATE TABLE research_source (
    id               TEXT PRIMARY KEY NOT NULL CHECK (id LIKE 'research_source:%'),
    project_id       TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    branch_id        TEXT,
    title            TEXT NOT NULL,
    source_type      TEXT NOT NULL,
    url              TEXT,
    file_path        TEXT,
    author           TEXT,
    publisher        TEXT,
    published_date   TEXT,
    accessed_at      INTEGER NOT NULL,
    reliability      TEXT NOT NULL,
    tags             TEXT NOT NULL CHECK (json_valid(tags)),
    summary          TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX idx_research_source_project ON research_source(project_id);
CREATE INDEX idx_research_source_branch ON research_source(branch_id);

CREATE TABLE research_note (
    id               TEXT PRIMARY KEY NOT NULL CHECK (id LIKE 'research_note:%'),
    project_id       TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    source_id        TEXT REFERENCES research_source(id) ON DELETE SET NULL,
    branch_id        TEXT,
    note             TEXT NOT NULL,
    quote            TEXT,
    locator          TEXT,
    tags             TEXT NOT NULL CHECK (json_valid(tags)),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX idx_research_note_project ON research_note(project_id);
CREATE INDEX idx_research_note_source ON research_note(source_id);
CREATE INDEX idx_research_note_branch ON research_note(branch_id);

CREATE TABLE research_claim (
    id               TEXT PRIMARY KEY NOT NULL CHECK (id LIKE 'research_claim:%'),
    project_id       TEXT NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    source_id        TEXT REFERENCES research_source(id) ON DELETE SET NULL,
    note_id          TEXT REFERENCES research_note(id) ON DELETE SET NULL,
    branch_id        TEXT,
    claim            TEXT NOT NULL,
    topic            TEXT,
    time_period      TEXT,
    location         TEXT,
    confidence       TEXT NOT NULL,
    tags             TEXT NOT NULL CHECK (json_valid(tags)),
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE INDEX idx_research_claim_project ON research_claim(project_id);
CREATE INDEX idx_research_claim_source ON research_claim(source_id);
CREATE INDEX idx_research_claim_note ON research_claim(note_id);
CREATE INDEX idx_research_claim_branch ON research_claim(branch_id);

-- Virtual tables for Full-Text Search using FTS5 (matching Spindle's existing FTS pattern)
CREATE VIRTUAL TABLE fts_research_source USING fts5(
    source_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    title,
    summary,
    author,
    publisher,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_research_source_ai AFTER INSERT ON research_source BEGIN
    INSERT INTO fts_research_source(source_id, project_id, branch_id, title, summary, author, publisher)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.title, COALESCE(NEW.summary, ''), COALESCE(NEW.author, ''), COALESCE(NEW.publisher, ''));
END;

CREATE TRIGGER trg_fts_research_source_ad AFTER DELETE ON research_source BEGIN
    DELETE FROM fts_research_source WHERE source_id = OLD.id;
END;

CREATE TRIGGER trg_fts_research_source_au AFTER UPDATE OF title, summary, author, publisher, branch_id ON research_source BEGIN
    DELETE FROM fts_research_source WHERE source_id = OLD.id;
    INSERT INTO fts_research_source(source_id, project_id, branch_id, title, summary, author, publisher)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.title, COALESCE(NEW.summary, ''), COALESCE(NEW.author, ''), COALESCE(NEW.publisher, ''));
END;


CREATE VIRTUAL TABLE fts_research_note USING fts5(
    note_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    note,
    quote,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_research_note_ai AFTER INSERT ON research_note BEGIN
    INSERT INTO fts_research_note(note_id, project_id, branch_id, note, quote)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.note, COALESCE(NEW.quote, ''));
END;

CREATE TRIGGER trg_fts_research_note_ad AFTER DELETE ON research_note BEGIN
    DELETE FROM fts_research_note WHERE note_id = OLD.id;
END;

CREATE TRIGGER trg_fts_research_note_au AFTER UPDATE OF note, quote, branch_id ON research_note BEGIN
    DELETE FROM fts_research_note WHERE note_id = OLD.id;
    INSERT INTO fts_research_note(note_id, project_id, branch_id, note, quote)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.note, COALESCE(NEW.quote, ''));
END;


CREATE VIRTUAL TABLE fts_research_claim USING fts5(
    claim_id UNINDEXED,
    project_id UNINDEXED,
    branch_id UNINDEXED,
    claim,
    topic,
    location,
    tokenize = 'porter unicode61'
);

CREATE TRIGGER trg_fts_research_claim_ai AFTER INSERT ON research_claim BEGIN
    INSERT INTO fts_research_claim(claim_id, project_id, branch_id, claim, topic, location)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.claim, COALESCE(NEW.topic, ''), COALESCE(NEW.location, ''));
END;

CREATE TRIGGER trg_fts_research_claim_ad AFTER DELETE ON research_claim BEGIN
    DELETE FROM fts_research_claim WHERE claim_id = OLD.id;
END;

CREATE TRIGGER trg_fts_research_claim_au AFTER UPDATE OF claim, topic, location, branch_id ON research_claim BEGIN
    DELETE FROM fts_research_claim WHERE claim_id = OLD.id;
    INSERT INTO fts_research_claim(claim_id, project_id, branch_id, claim, topic, location)
    VALUES (NEW.id, NEW.project_id, NEW.branch_id, NEW.claim, COALESCE(NEW.topic, ''), COALESCE(NEW.location, ''));
END;

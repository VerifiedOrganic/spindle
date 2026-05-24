-- Spindle SQLite schema, v0001.
--
-- Consolidated snapshot of the post-v032 SurrealDB schema, translated to
-- SQLite per docs/spindle-sqlite-migration-plan.md and the FK audit at
-- docs/spindle-sqlite-fk-audit.md.
--
-- Conventions:
--   * IDs are TEXT, formatted "table:ulid" to preserve the public record-id
--     contract from SurrealDB. The app mints them via ulid::Ulid::new().
--   * Datetimes are INTEGER unix microseconds (signed). Conversion lives in
--     the Rust row helpers added in Phase 3.
--   * Booleans are INTEGER 0/1 with a CHECK guard.
--   * SurrealDB `object FLEXIBLE` and `array<...>` map to TEXT JSON with
--     CHECK(json_valid(col)). Nullable variants use `col IS NULL OR json_valid(col)`.
--   * Embeddings are BLOB (sqlite-vec float32 layout). Dimension is pinned to 64
--     to match the default TokenHash backend; a mirroring vec0 virtual table is
--     declared at the bottom of this file and Phase 5 wires triggers.
--   * Dynamic-record refs (any-table polymorphic) are TEXT with no FK; app
--     validates.
--
-- This file is the only schema migration committed at v0.1. Existing dev
-- databases are discarded; there is no SurrealDB-to-SQLite data migration.

PRAGMA foreign_keys = ON;

-- =============================================================================
-- Root: project
-- =============================================================================

CREATE TABLE project (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'project:%'),
    name              TEXT    NOT NULL,
    project_type      TEXT    NOT NULL,
    genre             TEXT    NOT NULL,
    reader_contract   TEXT    NOT NULL CHECK (json_valid(reader_contract)),
    notes             TEXT,
    active_branch_id  TEXT,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- =============================================================================
-- Branches and save points
-- =============================================================================

CREATE TABLE bible_branch (
    id                          TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'bible_branch:%'),
    project_id                  TEXT             REFERENCES project(id)      ON DELETE CASCADE,
    parent_branch_id            TEXT             REFERENCES bible_branch(id) ON DELETE SET NULL,
    name                        TEXT    NOT NULL,
    status                      TEXT    NOT NULL,
    branch_type                 TEXT,
    description                 TEXT,
    created_from_save_point_id  TEXT             /* FK declared after save_point below via no-op self-doc */,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER
);

CREATE UNIQUE INDEX idx_branch_project_name ON bible_branch(project_id, name);

CREATE TABLE save_point (
    id           TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'save_point:%'),
    project_id   TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id    TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name         TEXT    NOT NULL,
    description  TEXT,
    snapshot_file_path     TEXT,
    snapshot_format        TEXT,
    snapshot_record_count  INTEGER,
    snapshot_created_at    INTEGER,
    snapshot_sha256        TEXT,
    created_at   INTEGER NOT NULL
);

-- The forward FK from bible_branch.created_from_save_point_id to save_point(id)
-- cannot be added with ALTER TABLE in SQLite. Referential integrity is enforced
-- by the app at write time (validators.rs). SET NULL semantics are emulated
-- by an AFTER DELETE trigger:

CREATE TRIGGER trg_save_point_branch_ancestry
AFTER DELETE ON save_point
BEGIN
    UPDATE bible_branch
       SET created_from_save_point_id = NULL
     WHERE created_from_save_point_id = OLD.id;
END;

-- =============================================================================
-- Manuscript hierarchy: book -> chapter -> scene
-- =============================================================================

CREATE TABLE book (
    id           TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'book:%'),
    project_id   TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    book_number  INTEGER NOT NULL,
    title        TEXT,
    created_at   INTEGER NOT NULL
);

CREATE TABLE chapter (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'chapter:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    book_id         TEXT    NOT NULL REFERENCES book(id)    ON DELETE CASCADE,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    title           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER
);

CREATE TABLE scene (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'scene:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_id         TEXT    NOT NULL REFERENCES book(id)         ON DELETE CASCADE,
    chapter_id      TEXT    NOT NULL REFERENCES chapter(id)      ON DELETE CASCADE,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    scene_order     INTEGER NOT NULL,
    full_text       TEXT    NOT NULL,
    summary         TEXT    NOT NULL,
    content_rating  TEXT    NOT NULL CHECK (content_rating IN ('General','Teen','Mature','Explicit')),
    tone            TEXT,
    draft_origin    TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_scene_natural_key
    ON scene(project_id, branch_id, book_number, chapter_number, scene_order);

-- =============================================================================
-- Characters and character-attached state
-- =============================================================================

CREATE TABLE character (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'character:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name             TEXT    NOT NULL,
    normalized_name  TEXT    NOT NULL,
    summary          TEXT    NOT NULL,
    role             TEXT    NOT NULL,
    realm            TEXT,
    notes            TEXT,
    appearance       TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_character_project_name ON character(project_id, normalized_name);

CREATE TABLE character_voice_profile (
    id                         TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'character_voice_profile:%'),
    character_id               TEXT    NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    vocabulary                 TEXT    NOT NULL CHECK (json_valid(vocabulary)),
    sentence_structure         TEXT    NOT NULL CHECK (json_valid(sentence_structure)),
    tics                       TEXT    NOT NULL CHECK (json_valid(tics)),
    forbidden_words            TEXT    NOT NULL CHECK (json_valid(forbidden_words)),
    example_lines              TEXT    NOT NULL CHECK (json_valid(example_lines)),
    tone                       TEXT,
    established_in_scene_id    TEXT             REFERENCES scene(id) ON DELETE SET NULL,
    created_at                 INTEGER NOT NULL,
    updated_at                 INTEGER
);

CREATE UNIQUE INDEX idx_voice_character ON character_voice_profile(character_id);

CREATE TABLE character_emotional_profile (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'character_emotional_profile:%'),
    character_id        TEXT    NOT NULL REFERENCES character(id) ON DELETE CASCADE,
    base_emotions       TEXT    NOT NULL CHECK (json_valid(base_emotions)),
    suppressed          TEXT    NOT NULL CHECK (json_valid(suppressed)),
    triggers            TEXT    NOT NULL CHECK (json_valid(triggers)),
    defense_mechanisms  TEXT    NOT NULL CHECK (json_valid(defense_mechanisms)),
    flex_range          TEXT             CHECK (flex_range IS NULL OR json_valid(flex_range)),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER
);

CREATE UNIQUE INDEX idx_emotional_character ON character_emotional_profile(character_id);

CREATE TABLE character_state (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'character_state:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    character_id     TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    scene_id         TEXT             REFERENCES scene(id)        ON DELETE SET NULL,
    book_number      INTEGER NOT NULL,
    chapter_number   INTEGER NOT NULL,
    scene_order      INTEGER NOT NULL,
    emotional_state  TEXT    NOT NULL CHECK (json_valid(emotional_state)),
    goals            TEXT    NOT NULL CHECK (json_valid(goals)),
    status           TEXT    NOT NULL CHECK (json_valid(status)),
    notes            TEXT    NOT NULL CHECK (json_valid(notes)),
    source_summary   TEXT,
    created_at       INTEGER NOT NULL
);

CREATE INDEX idx_character_state_lookup
    ON character_state(character_id, book_number, chapter_number, scene_order);

CREATE TABLE character_arc (
    id                    TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'character_arc:%'),
    project_id            TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id             TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    character_id          TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    arc_type              TEXT    NOT NULL,
    starting_state        TEXT    NOT NULL,
    ending_state          TEXT    NOT NULL,
    milestones            TEXT    NOT NULL CHECK (json_valid(milestones)),
    thematic_purpose      TEXT    NOT NULL,
    connected_theme_ids   TEXT    NOT NULL CHECK (json_valid(connected_theme_ids)),
    status                TEXT    NOT NULL,
    progress              REAL    NOT NULL,
    notes                 TEXT,
    archived_at           INTEGER,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);

-- =============================================================================
-- World: locations, world_state, world_rule, faction/religion/economy/term
-- =============================================================================

CREATE TABLE location (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'location:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name             TEXT    NOT NULL,
    normalized_name  TEXT    NOT NULL,
    kind             TEXT    NOT NULL,
    realm            TEXT,
    summary          TEXT    NOT NULL,
    notes            TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE world_state (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'world_state:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    location_id          TEXT    NOT NULL REFERENCES location(id)     ON DELETE CASCADE,
    controlling_faction  TEXT,
    status               TEXT,
    prosperity           TEXT,
    stability            TEXT,
    threat_level         TEXT,
    sensory_details      TEXT    NOT NULL CHECK (json_valid(sensory_details)),
    updated_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_world_state_location ON world_state(branch_id, location_id);

CREATE TABLE world_rule (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'world_rule:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    rule_name       TEXT    NOT NULL,
    rule_type       TEXT    NOT NULL,
    description     TEXT    NOT NULL,
    established_in  TEXT             CHECK (established_in IS NULL OR json_valid(established_in)),
    relevance_tags  TEXT             CHECK (relevance_tags IS NULL OR json_valid(relevance_tags)),
    scan_pattern    TEXT,
    notes           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER
);

CREATE UNIQUE INDEX idx_world_rule_project_type_name
    ON world_rule(project_id, rule_type, rule_name);

CREATE TABLE faction (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'faction:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name             TEXT    NOT NULL,
    normalized_name  TEXT    NOT NULL,
    faction_type     TEXT    NOT NULL,
    realm            TEXT,
    summary          TEXT    NOT NULL,
    tags             TEXT    NOT NULL CHECK (json_valid(tags)),
    notes            TEXT,
    archived_at      INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_faction_project_name ON faction(project_id, normalized_name);

CREATE TABLE religion (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'religion:%'),
    project_id          TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id           TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name                TEXT    NOT NULL,
    normalized_name     TEXT    NOT NULL,
    deity_or_principle  TEXT    NOT NULL,
    summary             TEXT    NOT NULL,
    tags                TEXT    NOT NULL CHECK (json_valid(tags)),
    notes               TEXT,
    archived_at         INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_religion_project_name ON religion(project_id, normalized_name);

CREATE TABLE economy (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'economy:%'),
    project_id        TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id         TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name              TEXT    NOT NULL,
    normalized_name   TEXT    NOT NULL,
    realm             TEXT,
    summary           TEXT    NOT NULL,
    scarce_resources  TEXT    NOT NULL CHECK (json_valid(scarce_resources)),
    trade_goods       TEXT    NOT NULL CHECK (json_valid(trade_goods)),
    currency          TEXT,
    notes             TEXT,
    archived_at       INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_economy_project_name ON economy(project_id, normalized_name);

CREATE TABLE term (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'term:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    term_text        TEXT    NOT NULL,
    normalized_term  TEXT    NOT NULL,
    pronunciation    TEXT,
    definition       TEXT    NOT NULL,
    usage_context    TEXT,
    origin           TEXT,
    notes            TEXT,
    archived_at      INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_term_project_name ON term(project_id, normalized_term);

-- =============================================================================
-- Narrative architecture: plot_line, conflict, theme, motif, narrative_promise
-- =============================================================================

CREATE TABLE plot_line (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'plot_line:%'),
    project_id          TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id           TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name                TEXT    NOT NULL,
    normalized_name     TEXT    NOT NULL,
    plot_type           TEXT    NOT NULL,
    summary             TEXT    NOT NULL,
    status              TEXT    NOT NULL,
    convergence_points  TEXT    NOT NULL CHECK (json_valid(convergence_points)),
    notes               TEXT,
    archived_at         INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_plot_line_project_name ON plot_line(project_id, normalized_name);

CREATE TABLE conflict (
    id                       TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'conflict:%'),
    project_id               TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id                TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name                     TEXT    NOT NULL,
    normalized_name          TEXT    NOT NULL,
    conflict_type            TEXT    NOT NULL,
    stakes                   TEXT    NOT NULL,
    escalation_stages        TEXT    NOT NULL CHECK (json_valid(escalation_stages)),
    expected_total_cycles    INTEGER,
    try_fail_cycles          TEXT    NOT NULL CHECK (json_valid(try_fail_cycles)),
    stated_consequences      TEXT    NOT NULL CHECK (json_valid(stated_consequences)),
    resolution_summary       TEXT,
    notes                    TEXT,
    archived_at              INTEGER,
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_conflict_project_name ON conflict(project_id, normalized_name);

CREATE TABLE theme (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'theme:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    theme_statement      TEXT    NOT NULL,
    thesis_antithesis    TEXT    NOT NULL,
    introduction_point   TEXT             CHECK (introduction_point IS NULL OR json_valid(introduction_point)),
    resolution_point     TEXT             CHECK (resolution_point IS NULL OR json_valid(resolution_point)),
    notes                TEXT,
    archived_at          INTEGER,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE TABLE motif (
    id                     TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'motif:%'),
    project_id             TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id              TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    name                   TEXT    NOT NULL,
    normalized_name        TEXT    NOT NULL,
    description            TEXT    NOT NULL,
    max_uses_per_chapter   INTEGER,
    connected_theme_ids    TEXT    NOT NULL CHECK (json_valid(connected_theme_ids)),
    notes                  TEXT,
    archived_at            INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_motif_project_name ON motif(project_id, normalized_name);

CREATE TABLE narrative_promise (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'narrative_promise:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    promise_type    TEXT    NOT NULL,
    description     TEXT    NOT NULL,
    status          TEXT    NOT NULL,
    planted_at      TEXT    NOT NULL CHECK (json_valid(planted_at)),
    planned_payoff  TEXT             CHECK (planned_payoff IS NULL OR json_valid(planned_payoff)),
    notes           TEXT,
    archived_at     INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- =============================================================================
-- Pacing
-- =============================================================================

CREATE TABLE pacing_config (
    id                       TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'pacing_config:%'),
    project_id               TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id                TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    total_planned_books      INTEGER NOT NULL,
    avg_chapters_per_book    INTEGER NOT NULL,
    avg_scenes_per_chapter   INTEGER NOT NULL,
    tension_model            TEXT    NOT NULL,
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_pacing_config_project_branch ON pacing_config(project_id, branch_id);

CREATE TABLE pacing_curve (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'pacing_curve:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_number          INTEGER NOT NULL,
    act_breakpoints      TEXT    NOT NULL CHECK (json_valid(act_breakpoints)),
    scene_type_density   TEXT    NOT NULL CHECK (json_valid(scene_type_density)),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_pacing_curve_book ON pacing_curve(project_id, branch_id, book_number);

CREATE TABLE pacing_tracker (
    id                         TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'pacing_tracker:%'),
    project_id                 TEXT    NOT NULL REFERENCES project(id)        ON DELETE CASCADE,
    branch_id                  TEXT    NOT NULL REFERENCES bible_branch(id)   ON DELETE CASCADE,
    character_arc_id           TEXT    NOT NULL REFERENCES character_arc(id)  ON DELETE CASCADE,
    per_book_budget            TEXT    NOT NULL CHECK (json_valid(per_book_budget)),
    max_progress_per_chapter   REAL,
    milestone_spacing          INTEGER,
    sprint_allowance           INTEGER,
    regression_budget          REAL,
    current_progress           REAL    NOT NULL,
    budget_remaining           REAL    NOT NULL,
    velocity                   TEXT    NOT NULL,
    status                     TEXT    NOT NULL,
    next_milestone             TEXT,
    warnings                   TEXT    NOT NULL CHECK (json_valid(warnings)),
    updated_at                 INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_pacing_tracker_arc ON pacing_tracker(branch_id, character_arc_id);

-- =============================================================================
-- Chapter planning, summaries, outlines, scene annotations
-- =============================================================================

CREATE TABLE chapter_plan (
    id                     TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'chapter_plan:%'),
    project_id             TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id              TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_number            INTEGER NOT NULL,
    chapter_number         INTEGER NOT NULL,
    pov_character_id       TEXT             REFERENCES character(id)    ON DELETE SET NULL,
    synopsis               TEXT    NOT NULL,
    target_theme_ids       TEXT    NOT NULL CHECK (json_valid(target_theme_ids)),
    target_conflict_ids    TEXT    NOT NULL CHECK (json_valid(target_conflict_ids)),
    target_plot_line_ids   TEXT    NOT NULL CHECK (json_valid(target_plot_line_ids)),
    scenes                 TEXT    NOT NULL CHECK (json_valid(scenes)),
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_chapter_plan_key
    ON chapter_plan(project_id, branch_id, book_number, chapter_number);

CREATE TABLE chapter_summary (
    id                    TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'chapter_summary:%'),
    project_id            TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id             TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_number           INTEGER NOT NULL,
    chapter_number        INTEGER NOT NULL,
    summary               TEXT    NOT NULL,
    key_events            TEXT    NOT NULL CHECK (json_valid(key_events)),
    character_changes     TEXT    NOT NULL CHECK (json_valid(character_changes)),
    relationship_shifts   TEXT    NOT NULL CHECK (json_valid(relationship_shifts)),
    arc_advances          TEXT    NOT NULL CHECK (json_valid(arc_advances)),
    promise_events        TEXT    NOT NULL CHECK (json_valid(promise_events)),
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_chapter_summary_key
    ON chapter_summary(project_id, branch_id, book_number, chapter_number);

CREATE TABLE scene_beat_annotation (
    id            TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'scene_beat_annotation:%'),
    project_id    TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id     TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id      TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    beats         TEXT    NOT NULL CHECK (json_valid(beats)),
    motif_ids     TEXT    NOT NULL CHECK (json_valid(motif_ids)),
    theme_ids     TEXT    NOT NULL CHECK (json_valid(theme_ids)),
    conflict_ids  TEXT    NOT NULL CHECK (json_valid(conflict_ids)),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_scene_beat_annotation_scene
    ON scene_beat_annotation(branch_id, scene_id);

CREATE TABLE book_outline (
    id          TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'book_outline:%'),
    book_id     TEXT    NOT NULL REFERENCES book(id)         ON DELETE CASCADE,
    branch_id   TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    format      TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX book_outline_book_branch ON book_outline(book_id, branch_id);

CREATE TABLE chapter_outline (
    id           TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'chapter_outline:%'),
    chapter_id   TEXT    NOT NULL REFERENCES chapter(id)      ON DELETE CASCADE,
    branch_id    TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    format       TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    beats        TEXT    NOT NULL CHECK (json_valid(beats)),
    updated_at   INTEGER NOT NULL
);

CREATE UNIQUE INDEX chapter_outline_chapter_branch ON chapter_outline(chapter_id, branch_id);

-- =============================================================================
-- Timeline, temporal interventions, system overlays
-- =============================================================================

CREATE TABLE timeline_event (
    id                 TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'timeline_event:%'),
    project_id         TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    title              TEXT    NOT NULL,
    event_type         TEXT    NOT NULL,
    placement          TEXT    NOT NULL CHECK (json_valid(placement)),
    summary            TEXT    NOT NULL,
    related_entity_ids TEXT    NOT NULL CHECK (json_valid(related_entity_ids)),
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_timeline_event_position ON timeline_event(project_id, branch_id, title);

CREATE TABLE temporal_intervention (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'temporal_intervention:%'),
    project_id          TEXT    NOT NULL REFERENCES project(id)         ON DELETE CASCADE,
    branch_id           TEXT    NOT NULL REFERENCES bible_branch(id)    ON DELETE CASCADE,
    title               TEXT    NOT NULL,
    intervention_type   TEXT    NOT NULL,
    source_event_id     TEXT             REFERENCES timeline_event(id)  ON DELETE SET NULL,
    target_event_id     TEXT             REFERENCES timeline_event(id)  ON DELETE SET NULL,
    summary             TEXT    NOT NULL,
    consequences        TEXT    NOT NULL CHECK (json_valid(consequences)),
    status              TEXT    NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE TABLE system_overlay (
    id                     TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'system_overlay:%'),
    project_id             TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id              TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    system_name            TEXT    NOT NULL,
    normalized_name        TEXT    NOT NULL,
    system_type            TEXT    NOT NULL,
    rules                  TEXT    NOT NULL,
    visibility             TEXT    NOT NULL,
    progression_currency   TEXT,
    stats                  TEXT    NOT NULL CHECK (json_valid(stats)),
    advancement_tiers      TEXT    NOT NULL CHECK (json_valid(advancement_tiers)),
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_system_overlay_name ON system_overlay(project_id, branch_id, normalized_name);

-- =============================================================================
-- Knowledge: future_knowledge, knowledge_fact
-- =============================================================================

CREATE TABLE future_knowledge (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'future_knowledge:%'),
    project_id           TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id            TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    character_id         TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    knowledge_summary    TEXT    NOT NULL,
    source               TEXT    NOT NULL,
    learned_at           TEXT    NOT NULL CHECK (json_valid(learned_at)),
    expires_at           TEXT             CHECK (expires_at IS NULL OR json_valid(expires_at)),
    notes                TEXT    NOT NULL CHECK (json_valid(notes)),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

-- knowledge_fact references import_session via source_import_session_id, but
-- import_session is declared lower down; SQLite resolves the FK at row time, so
-- ordering doesn't matter here. The same applies to knows below.
CREATE TABLE knowledge_fact (
    id                          TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'knowledge_fact:%'),
    project_id                  TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id                   TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    character_id                TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    fact                        TEXT    NOT NULL,
    normalized_fact             TEXT    NOT NULL,
    source_summary              TEXT    NOT NULL,
    learned_at                  TEXT             CHECK (learned_at IS NULL OR json_valid(learned_at)),
    confidence                  REAL,
    tags                        TEXT    NOT NULL CHECK (json_valid(tags)),
    reader_visible              INTEGER NOT NULL CHECK (reader_visible IN (0,1)),
    source_import_session_id    TEXT             REFERENCES import_session(id) ON DELETE SET NULL,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_knowledge_fact_lookup
    ON knowledge_fact(project_id, branch_id, character_id, normalized_fact);
CREATE INDEX idx_knowledge_fact_branch_character
    ON knowledge_fact(branch_id, character_id);
CREATE INDEX idx_knowledge_fact_import_session
    ON knowledge_fact(source_import_session_id);

-- =============================================================================
-- Reviews and revisions: dual_persona_review, revision_marker, scene_version,
-- validator_finding, progression_event
-- =============================================================================

CREATE TABLE dual_persona_review (
    id                          TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'dual_persona_review:%'),
    project_id                  TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id                   TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id                    TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    scene_revision_fingerprint  TEXT    NOT NULL,
    rounds_completed            INTEGER NOT NULL,
    status                      TEXT    NOT NULL,
    review_rounds               TEXT    NOT NULL CHECK (json_valid(review_rounds)),
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_dual_persona_review_scene ON dual_persona_review(branch_id, scene_id);

CREATE TABLE revision_marker (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'revision_marker:%'),
    project_id        TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id         TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id          TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    marker_type       TEXT    NOT NULL,
    -- Polymorphic record ref (any table). App validates; no FK.
    target_record_id  TEXT,
    position          TEXT    NOT NULL,
    note              TEXT    NOT NULL,
    status            TEXT    NOT NULL,
    created_at        INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_revision_marker_unique
    ON revision_marker(branch_id, scene_id, marker_type, target_record_id);

CREATE TABLE scene_version (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'scene_version:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id        TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    version_number  INTEGER NOT NULL,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    scene_order     INTEGER NOT NULL,
    full_text       TEXT    NOT NULL,
    summary         TEXT    NOT NULL,
    content_rating  TEXT    NOT NULL CHECK (content_rating IN ('General','Teen','Mature','Explicit')),
    tone            TEXT,
    created_at      INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_scene_version_number ON scene_version(scene_id, version_number);
CREATE INDEX idx_scene_version_list        ON scene_version(scene_id, created_at);

CREATE TABLE validator_finding (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'validator_finding:%'),
    project_id        TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id         TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id          TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
    scene_text_hash   TEXT    NOT NULL,
    validator_id      TEXT    NOT NULL,
    finding_id        TEXT    NOT NULL,
    severity          TEXT    NOT NULL,
    message           TEXT    NOT NULL,
    byte_range        TEXT             CHECK (byte_range IS NULL OR json_valid(byte_range)),
    details_json      TEXT             CHECK (details_json IS NULL OR json_valid(details_json)),
    created_at        INTEGER NOT NULL,
    resolved_at       INTEGER
);

CREATE INDEX validator_finding_scene_validator_hash
    ON validator_finding(branch_id, scene_id, validator_id, scene_text_hash);
CREATE INDEX validator_finding_active_by_validator
    ON validator_finding(project_id, branch_id, validator_id, resolved_at);

CREATE TABLE progression_event (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'progression_event:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id)              ON DELETE CASCADE,
    branch_id        TEXT    NOT NULL REFERENCES bible_branch(id)         ON DELETE CASCADE,
    subject_table    TEXT    NOT NULL,
    -- Polymorphic record ref. App validates against subject_table.
    subject_id       TEXT    NOT NULL,
    overlay_id       TEXT             REFERENCES system_overlay(id)        ON DELETE SET NULL,
    kind             TEXT    NOT NULL,
    delta_json       TEXT    NOT NULL CHECK (json_valid(delta_json)),
    source_scene_id  TEXT             REFERENCES scene(id)                 ON DELETE SET NULL,
    placement        TEXT             CHECK (placement IS NULL OR json_valid(placement)),
    created_at       INTEGER NOT NULL
);

CREATE INDEX progression_event_subject_time ON progression_event(subject_table, subject_id, created_at);
CREATE INDEX progression_event_overlay_time ON progression_event(overlay_id, created_at);

-- =============================================================================
-- Session activity, research log, writer position
-- =============================================================================

CREATE TABLE session_activity (
    id             TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'session_activity:%'),
    project_id     TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id      TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    kind           TEXT    NOT NULL,
    subject_table  TEXT,
    -- Polymorphic record ref.
    subject_id     TEXT,
    summary        TEXT    NOT NULL,
    details_json   TEXT             CHECK (details_json IS NULL OR json_valid(details_json)),
    created_at     INTEGER NOT NULL
);

CREATE INDEX session_activity_project_branch_time
    ON session_activity(project_id, branch_id, created_at);

CREATE TABLE research_log (
    id               TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'research_log:%'),
    project_id       TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    query            TEXT    NOT NULL,
    context_hint     TEXT,
    model            TEXT    NOT NULL,
    response         TEXT    NOT NULL,
    context_summary  TEXT    NOT NULL,
    created_at       INTEGER NOT NULL
);

CREATE INDEX idx_research_log_project_created ON research_log(project_id, created_at);

CREATE TABLE writer_position (
    id          TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'writer_position:%'),
    project_id  TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id   TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    book_id     TEXT             REFERENCES book(id)         ON DELETE SET NULL,
    chapter_id  TEXT             REFERENCES chapter(id)      ON DELETE SET NULL,
    scene_id    TEXT             REFERENCES scene(id)        ON DELETE SET NULL,
    intent      TEXT    NOT NULL,
    next_focus  TEXT,
    updated_by  TEXT    NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE UNIQUE INDEX writer_position_project_branch ON writer_position(project_id, branch_id);

-- =============================================================================
-- Canonical facts and scene source links (divergence tracking)
-- =============================================================================

CREATE TABLE canonical_fact (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'canonical_fact:%'),
    project_id      TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id       TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    scene_id        TEXT    NOT NULL REFERENCES scene(id)        ON DELETE CASCADE,
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
    updated_at      INTEGER NOT NULL
);

CREATE INDEX canonical_fact_subject_predicate_idx
    ON canonical_fact(project_id, branch_id, subject_table, subject_id, predicate);
CREATE INDEX canonical_fact_scope_idx
    ON canonical_fact(project_id, branch_id, scope);

CREATE TABLE scene_source_link (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'scene_source_link:%'),
    project_id          TEXT    NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    scene_id            TEXT    NOT NULL REFERENCES scene(id)   ON DELETE CASCADE,
    source_path         TEXT    NOT NULL,
    content_sha256      TEXT    NOT NULL,
    source_start_offset INTEGER,
    source_end_offset   INTEGER,
    linked_at           INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

-- =============================================================================
-- Import pipeline
-- =============================================================================

CREATE TABLE import_session (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_session:%'),
    project_id          TEXT             REFERENCES project(id)      ON DELETE CASCADE,
    target_branch_id    TEXT             REFERENCES bible_branch(id) ON DELETE CASCADE,
    source_format       TEXT,
    active_pass         TEXT    NOT NULL,
    progress            TEXT    NOT NULL CHECK (json_valid(progress)),
    session_status      TEXT    NOT NULL,
    hydrate_mode        TEXT    NOT NULL,
    source_count        INTEGER NOT NULL,
    hydration_report    TEXT             CHECK (hydration_report IS NULL OR json_valid(hydration_report)),
    imported_at         INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE INDEX idx_import_session_project_status ON import_session(project_id, session_status);
CREATE INDEX idx_import_session_pass_status    ON import_session(active_pass, session_status);

CREATE TABLE import_source_document (
    id                   TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_source_document:%'),
    session_id           TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    project_id           TEXT             REFERENCES project(id)         ON DELETE CASCADE,
    display_name         TEXT    NOT NULL,
    source_path          TEXT    NOT NULL,
    copied_path          TEXT    NOT NULL,
    source_format        TEXT    NOT NULL,
    original_sha256      TEXT    NOT NULL,
    normalized_sha256    TEXT    NOT NULL,
    normalized_text_ref  TEXT    NOT NULL,
    word_count           INTEGER NOT NULL,
    chapter_hint         TEXT,
    source_order         INTEGER NOT NULL,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_source_document_order ON import_source_document(session_id, source_order);
CREATE UNIQUE INDEX idx_import_source_document_hash  ON import_source_document(session_id, original_sha256);

CREATE TABLE import_segment (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_segment:%'),
    session_id          TEXT    NOT NULL REFERENCES import_session(id)         ON DELETE CASCADE,
    source_document_id  TEXT    NOT NULL REFERENCES import_source_document(id) ON DELETE CASCADE,
    parent_segment_id   TEXT             REFERENCES import_segment(id)          ON DELETE SET NULL,
    segment_type        TEXT    NOT NULL,
    source_order        INTEGER NOT NULL,
    book_number         INTEGER,
    chapter_number      INTEGER,
    scene_order         INTEGER,
    label               TEXT,
    start_offset        INTEGER NOT NULL,
    end_offset          INTEGER NOT NULL,
    word_count          INTEGER NOT NULL,
    character_count     INTEGER NOT NULL,
    pov_guess           TEXT             CHECK (pov_guess IS NULL OR json_valid(pov_guess)),
    confidence          REAL    NOT NULL,
    segment_status      TEXT    NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_segment_order
    ON import_segment(session_id, source_document_id, source_order);
CREATE INDEX idx_import_segment_story_position
    ON import_segment(session_id, book_number, chapter_number, scene_order);

CREATE TABLE import_entity_mention (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_entity_mention:%'),
    session_id        TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    segment_id        TEXT    NOT NULL REFERENCES import_segment(id) ON DELETE CASCADE,
    entity_kind       TEXT    NOT NULL,
    surface_form      TEXT    NOT NULL,
    normalized_name   TEXT    NOT NULL,
    alias_hint        TEXT,
    surrounding_text  TEXT,
    confidence        REAL    NOT NULL,
    extraction_pass   TEXT    NOT NULL,
    created_at        INTEGER NOT NULL
);

CREATE INDEX idx_import_entity_mention_name
    ON import_entity_mention(session_id, entity_kind, normalized_name);
CREATE INDEX idx_import_entity_mention_segment
    ON import_entity_mention(segment_id, entity_kind);

CREATE TABLE import_entity_cluster (
    id                TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_entity_cluster:%'),
    session_id        TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    entity_kind       TEXT    NOT NULL,
    canonical_name    TEXT    NOT NULL,
    normalized_name   TEXT    NOT NULL,
    aliases           TEXT    NOT NULL CHECK (json_valid(aliases)),
    -- JSON TEXT of record-id strings; app-validated, no FK
    mention_ids       TEXT    NOT NULL CHECK (json_valid(mention_ids)),
    first_segment_id  TEXT             REFERENCES import_segment(id) ON DELETE SET NULL,
    last_segment_id   TEXT             REFERENCES import_segment(id) ON DELETE SET NULL,
    importance_rank   INTEGER NOT NULL,
    merge_confidence  REAL    NOT NULL,
    review_required   INTEGER NOT NULL CHECK (review_required IN (0,1)),
    notes             TEXT    NOT NULL CHECK (json_valid(notes)),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX idx_import_entity_cluster_name
    ON import_entity_cluster(session_id, entity_kind, normalized_name);

CREATE TABLE import_character_dossier (
    id                       TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_character_dossier:%'),
    session_id               TEXT    NOT NULL REFERENCES import_session(id)         ON DELETE CASCADE,
    cluster_id               TEXT    NOT NULL REFERENCES import_entity_cluster(id)  ON DELETE CASCADE,
    canonical_name           TEXT    NOT NULL,
    aliases                  TEXT    NOT NULL CHECK (json_valid(aliases)),
    importance_rank          INTEGER NOT NULL,
    voice_profile            TEXT    NOT NULL CHECK (json_valid(voice_profile)),
    emotional_profile        TEXT    NOT NULL CHECK (json_valid(emotional_profile)),
    state_trajectory         TEXT    NOT NULL CHECK (json_valid(state_trajectory)),
    relationship_inferences  TEXT    NOT NULL CHECK (json_valid(relationship_inferences)),
    decision_patterns        TEXT    NOT NULL CHECK (json_valid(decision_patterns)),
    dialogue_samples         TEXT    NOT NULL CHECK (json_valid(dialogue_samples)),
    confidence               REAL    NOT NULL,
    review_required          INTEGER NOT NULL CHECK (review_required IN (0,1)),
    created_at               INTEGER NOT NULL,
    updated_at               INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_character_dossier_cluster
    ON import_character_dossier(session_id, cluster_id);

CREATE TABLE import_world_dossier (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_world_dossier:%'),
    session_id      TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    world_rules     TEXT    NOT NULL CHECK (json_valid(world_rules)),
    locations       TEXT    NOT NULL CHECK (json_valid(locations)),
    entities        TEXT    NOT NULL CHECK (json_valid(entities)),
    system_signals  TEXT    NOT NULL CHECK (json_valid(system_signals)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_world_dossier_session ON import_world_dossier(session_id);

CREATE TABLE import_narrative_dossier (
    id                  TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_narrative_dossier:%'),
    session_id          TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    plot_lines          TEXT    NOT NULL CHECK (json_valid(plot_lines)),
    conflicts           TEXT    NOT NULL CHECK (json_valid(conflicts)),
    narrative_promises  TEXT    NOT NULL CHECK (json_valid(narrative_promises)),
    arcs                TEXT    NOT NULL CHECK (json_valid(arcs)),
    themes              TEXT    NOT NULL CHECK (json_valid(themes)),
    motifs              TEXT    NOT NULL CHECK (json_valid(motifs)),
    reader_contract     TEXT    NOT NULL CHECK (json_valid(reader_contract)),
    pacing_hints        TEXT    NOT NULL CHECK (json_valid(pacing_hints)),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_narrative_dossier_session ON import_narrative_dossier(session_id);

CREATE TABLE import_resume_snapshot (
    id              TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_resume_snapshot:%'),
    session_id      TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    book_number     INTEGER NOT NULL,
    chapter_number  INTEGER NOT NULL,
    scene_order     INTEGER,
    summary         TEXT    NOT NULL,
    characters      TEXT    NOT NULL CHECK (json_valid(characters)),
    relationships   TEXT    NOT NULL CHECK (json_valid(relationships)),
    locations       TEXT    NOT NULL CHECK (json_valid(locations)),
    plot_threads    TEXT    NOT NULL CHECK (json_valid(plot_threads)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_import_resume_snapshot_session ON import_resume_snapshot(session_id);

CREATE TABLE import_review_item (
    id                    TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'import_review_item:%'),
    session_id            TEXT    NOT NULL REFERENCES import_session(id) ON DELETE CASCADE,
    pass_name             TEXT    NOT NULL,
    item_kind             TEXT    NOT NULL,
    severity              TEXT    NOT NULL,
    status                TEXT    NOT NULL,
    title                 TEXT    NOT NULL,
    description           TEXT    NOT NULL,
    -- JSON array of import_segment ids; cascade is implicit via session.
    related_segment_ids   TEXT    NOT NULL CHECK (json_valid(related_segment_ids)),
    related_entity_ids    TEXT    NOT NULL CHECK (json_valid(related_entity_ids)),
    confidence            REAL,
    proposed_correction   TEXT             CHECK (proposed_correction IS NULL OR json_valid(proposed_correction)),
    resolver_notes        TEXT,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
);

CREATE INDEX idx_import_review_item_status ON import_review_item(session_id, status);
CREATE INDEX idx_import_review_item_pass   ON import_review_item(session_id, pass_name, status);

-- =============================================================================
-- Edge tables (formerly SurrealDB RELATIONs)
-- =============================================================================

-- relates_to: character -> character relationships, branch-scoped.
CREATE TABLE relates_to (
    in_id              TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    out_id             TEXT    NOT NULL REFERENCES character(id)    ON DELETE CASCADE,
    branch_id          TEXT    NOT NULL REFERENCES bible_branch(id) ON DELETE CASCADE,
    relationship_type  TEXT    NOT NULL,
    trust              INTEGER NOT NULL,
    tension            INTEGER NOT NULL,
    dynamics           TEXT    NOT NULL CHECK (json_valid(dynamics)),
    reason             TEXT,
    last_scene_id      TEXT             REFERENCES scene(id) ON DELETE SET NULL,
    updated_at         INTEGER NOT NULL,
    PRIMARY KEY (branch_id, in_id, out_id)
);

-- knows: character -> knowledge_fact knowledge edges, project- and branch-scoped.
CREATE TABLE knows (
    in_id                       TEXT    NOT NULL REFERENCES character(id)       ON DELETE CASCADE,
    out_id                      TEXT    NOT NULL REFERENCES knowledge_fact(id)  ON DELETE CASCADE,
    project_id                  TEXT    NOT NULL REFERENCES project(id)         ON DELETE CASCADE,
    branch_id                   TEXT    NOT NULL REFERENCES bible_branch(id)    ON DELETE CASCADE,
    source_summary              TEXT,
    learned_at                  TEXT             CHECK (learned_at IS NULL OR json_valid(learned_at)),
    confidence                  REAL,
    reader_visible              INTEGER NOT NULL CHECK (reader_visible IN (0,1)),
    source_import_session_id    TEXT             REFERENCES import_session(id) ON DELETE SET NULL,
    created_at                  INTEGER NOT NULL,
    updated_at                  INTEGER NOT NULL,
    PRIMARY KEY (branch_id, in_id, out_id)
);

CREATE INDEX idx_knows_subject_lookup ON knows(branch_id, in_id);

-- =============================================================================
-- Search embeddings (Phase 5 wires the paired vec0 virtual table)
-- =============================================================================

-- Plain row store. embedding is sqlite-vec float32 layout, 64 dims = 256 bytes.
CREATE TABLE search_embedding (
    id                 TEXT    PRIMARY KEY NOT NULL CHECK (id LIKE 'search_embedding:%'),
    project_id         TEXT    NOT NULL REFERENCES project(id)      ON DELETE CASCADE,
    branch_id          TEXT             REFERENCES bible_branch(id) ON DELETE CASCADE,
    entity_table       TEXT    NOT NULL,
    -- Polymorphic record ref keyed by entity_table; app-validated.
    entity_id          TEXT    NOT NULL,
    title              TEXT    NOT NULL,
    excerpt            TEXT    NOT NULL,
    content            TEXT    NOT NULL,
    embedding_version  TEXT    NOT NULL,
    embedding          BLOB    NOT NULL CHECK (length(embedding) = 256),
    updated_at         INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_search_embedding_entity
    ON search_embedding(project_id, branch_id, entity_table, entity_id);

-- =============================================================================
-- Foreign-key indexes for cascade performance
-- =============================================================================
--
-- SQLite uses indexes to find children when cascading. Without an index on the
-- child FK column, every cascade triggers a full scan. The unique/lookup
-- indexes above cover most parents; below are the few that aren't already
-- indexed but participate in heavy cascade paths.

CREATE INDEX idx_book_project                    ON book(project_id);
CREATE INDEX idx_chapter_book                    ON chapter(book_id);
CREATE INDEX idx_scene_branch                    ON scene(branch_id);
CREATE INDEX idx_scene_chapter                   ON scene(chapter_id);
CREATE INDEX idx_character_branch                ON character(branch_id);
CREATE INDEX idx_character_state_branch          ON character_state(branch_id);
CREATE INDEX idx_character_state_scene           ON character_state(scene_id);
CREATE INDEX idx_character_arc_branch            ON character_arc(branch_id);
CREATE INDEX idx_character_arc_character         ON character_arc(character_id);
CREATE INDEX idx_location_branch                 ON location(branch_id);
CREATE INDEX idx_world_rule_branch               ON world_rule(branch_id);
CREATE INDEX idx_faction_branch                  ON faction(branch_id);
CREATE INDEX idx_religion_branch                 ON religion(branch_id);
CREATE INDEX idx_economy_branch                  ON economy(branch_id);
CREATE INDEX idx_term_branch                     ON term(branch_id);
CREATE INDEX idx_plot_line_branch                ON plot_line(branch_id);
CREATE INDEX idx_conflict_branch                 ON conflict(branch_id);
CREATE INDEX idx_theme_project_branch            ON theme(project_id, branch_id);
CREATE INDEX idx_motif_branch                    ON motif(branch_id);
CREATE INDEX idx_narrative_promise_branch        ON narrative_promise(branch_id);
CREATE INDEX idx_future_knowledge_branch_char    ON future_knowledge(branch_id, character_id);
CREATE INDEX idx_scene_version_project_branch    ON scene_version(project_id, branch_id);
CREATE INDEX idx_revision_marker_branch          ON revision_marker(branch_id);
CREATE INDEX idx_canonical_fact_scene            ON canonical_fact(scene_id);
CREATE INDEX idx_canonical_fact_branch           ON canonical_fact(branch_id);
CREATE INDEX idx_scene_source_link_scene         ON scene_source_link(scene_id);
CREATE INDEX idx_save_point_branch               ON save_point(branch_id);
CREATE INDEX idx_relates_to_out                  ON relates_to(out_id);
CREATE INDEX idx_knows_out                       ON knows(out_id);

-- =============================================================================
-- Seed: default main branch is created by the app, not the schema, because the
-- branch needs a project_id and there is no project yet at migration time.
-- The repository's create_project() inserts a 'bible_branch:main' row.
-- =============================================================================

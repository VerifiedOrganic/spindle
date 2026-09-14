-- =============================================================================
-- V0017: in-world story-time layer (project calendar + per-entity story clocks).
--
-- Additive and optional: a project that never declares story-time is entirely
-- unaffected. Clocks live in side tables keyed by the parent id so the hot
-- record read paths (scene, character, timeline_event) stay untouched.
-- =============================================================================

CREATE TABLE project_calendar (
    project_id     TEXT PRIMARY KEY REFERENCES project(id) ON DELETE CASCADE,
    days_per_week  INTEGER NOT NULL,
    hours_per_day  INTEGER NOT NULL DEFAULT 24,
    week_day_names TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(week_day_names)),
    months         TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(months)),
    days_per_year  INTEGER NOT NULL,
    epoch_label    TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

-- Optional in-world placement of a scene, 1:1 with `scene`.
CREATE TABLE scene_clock (
    scene_id       TEXT PRIMARY KEY REFERENCES scene(id) ON DELETE CASCADE,
    project_id     TEXT NOT NULL,
    branch_id      TEXT NOT NULL,
    day_index      INTEGER,
    time_of_day    INTEGER,
    duration_days  REAL,
    precision      TEXT,
    temporal_mode  TEXT,
    thread_key     TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX idx_scene_clock_thread_time
    ON scene_clock(project_id, branch_id, thread_key, day_index);

-- In-world time an event occurs, distinct from its manuscript placement.
CREATE TABLE timeline_event_clock (
    timeline_event_id TEXT PRIMARY KEY REFERENCES timeline_event(id) ON DELETE CASCADE,
    project_id     TEXT NOT NULL,
    branch_id      TEXT NOT NULL,
    day_index      INTEGER,
    time_of_day    INTEGER,
    duration_days  REAL,
    precision      TEXT,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX idx_timeline_event_clock_occurs
    ON timeline_event_clock(project_id, branch_id, day_index);

-- Birth anchor for deriving a character's age at any story moment.
CREATE TABLE character_birth (
    character_id    TEXT PRIMARY KEY REFERENCES character(id) ON DELETE CASCADE,
    project_id      TEXT NOT NULL,
    birth_day_index INTEGER,
    time_of_day     INTEGER,
    precision       TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

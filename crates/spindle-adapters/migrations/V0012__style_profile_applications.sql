-- =============================================================================
-- V0012: add style profile applications audit history.
-- =============================================================================

CREATE TABLE style_profile_application (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    applied_at TEXT NOT NULL,
    apply_mode TEXT NOT NULL,
    before_narrator_voice_json TEXT NOT NULL,
    after_narrator_voice_json TEXT NOT NULL,
    before_style_notes_json TEXT NOT NULL,
    after_style_notes_json TEXT NOT NULL,
    added_style_notes_json TEXT NOT NULL,
    removed_style_notes_json TEXT NOT NULL,
    style_rule_id TEXT,
    style_rule_action TEXT NOT NULL,
    style_rule_previous_description TEXT,
    invalidated_validator_count INTEGER NOT NULL,
    rolled_back_at TEXT,
    rollback_status TEXT NOT NULL DEFAULT 'not_rolled_back'
);

CREATE INDEX idx_style_profile_application_project
    ON style_profile_application(project_id, applied_at);

-- =============================================================================
-- V0011: add style profiles and style profile sources tables.
-- =============================================================================

CREATE TABLE style_profile (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    card_json TEXT NOT NULL,
    metrics_json TEXT NOT NULL,
    guidance_json TEXT NOT NULL,
    source_policy_json TEXT NOT NULL,
    model_receipt_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_style_profile_project
    ON style_profile(project_id, created_at);

CREATE TABLE style_profile_source (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    word_count INTEGER NOT NULL,
    included INTEGER NOT NULL,
    skip_reason TEXT,
    source_order INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_style_profile_source_profile
    ON style_profile_source(profile_id, source_order);

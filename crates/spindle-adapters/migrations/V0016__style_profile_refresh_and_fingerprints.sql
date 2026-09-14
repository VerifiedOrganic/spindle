-- =============================================================================
-- V0016: add versioning and fingerprint columns for style profiles and sources.
-- =============================================================================

ALTER TABLE style_profile ADD COLUMN parent_profile_id TEXT;
ALTER TABLE style_profile ADD COLUMN refreshed_from_profile_id TEXT;
ALTER TABLE style_profile ADD COLUMN version_number INTEGER;
ALTER TABLE style_profile ADD COLUMN refreshed_at TEXT;

ALTER TABLE style_profile_source ADD COLUMN file_size INTEGER;
ALTER TABLE style_profile_source ADD COLUMN modified_at TEXT;
ALTER TABLE style_profile_source ADD COLUMN glob_policy_metadata TEXT;

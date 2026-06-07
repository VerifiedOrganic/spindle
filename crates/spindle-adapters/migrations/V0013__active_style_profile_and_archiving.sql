-- V0013: add active_style_profile_id to project and archived_at to style_profile

ALTER TABLE project ADD COLUMN active_style_profile_id TEXT;
ALTER TABLE style_profile ADD COLUMN archived_at TEXT;

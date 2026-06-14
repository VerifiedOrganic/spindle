-- Persist the location a scene is set in, so the pre-draft temporal anchor can
-- name where the previous scene ended (the spatial half of grounding) and so a
-- scene's location survives the save -- previously `location_id` was an
-- ephemeral `get_scene_context` input that was never written back to the scene.
-- Nullable and additive: existing scenes, and projects that never set a
-- location, are unaffected. ON DELETE SET NULL so removing a location clears the
-- reference rather than orphaning the scene.
ALTER TABLE scene ADD COLUMN location_id TEXT REFERENCES location(id) ON DELETE SET NULL;

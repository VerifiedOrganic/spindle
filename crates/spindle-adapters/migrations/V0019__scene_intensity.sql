-- =============================================================================
-- V0019: realized scene intensity for pacing-drift detection.
--
-- A single 0.0-1.0 intensity the author records when annotating a scene's beats.
-- Nullable and additive; scenes without a recorded intensity are simply skipped
-- by the pacing_drift check.
-- =============================================================================

ALTER TABLE scene_beat_annotation ADD COLUMN intensity REAL;

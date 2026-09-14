-- =============================================================================
-- V0022: owner-approved schema fields for three narrative-convergence audits.
--
-- Every column is ADDITIVE and DEFAULTED so existing rows keep deserializing:
--   * plot_line.connected_conflict_ids / connected_theme_ids — JSON id arrays
--     (mirrors motif.connected_theme_ids) letting the convergence audit know
--     which conflicts/themes a plot line expects a beat annotation to link at
--     its convergence chapter. Default '[]'.
--   * conflict.escalation_demonstrated — a JSON array index-aligned with
--     escalation_stages; each entry is a StoredStoryPlacement (or null) marking
--     where a stage was demonstrated. Default '[]' (all stages undemonstrated),
--     back-compat safe: a shorter-than-stages vector reads as all-None beyond
--     its length. Drives the escalation-order audit.
--   * pacing_curve.intensity_points — a JSON array of {position, intensity}
--     pairs (position = 0..1 fraction of the book) giving the curve a per-
--     position expected intensity the realized-intensity trend directive can
--     interpolate against. Default '[]'.
--
-- character_arc milestone `reached_at` needs NO column here: milestones already
-- live inside the `character_arc.milestones` JSON blob, so the marker is a
-- serde-default struct field only.
-- =============================================================================

ALTER TABLE plot_line
    ADD COLUMN connected_conflict_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(connected_conflict_ids));
ALTER TABLE plot_line
    ADD COLUMN connected_theme_ids TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(connected_theme_ids));

ALTER TABLE conflict
    ADD COLUMN escalation_demonstrated TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(escalation_demonstrated));

ALTER TABLE pacing_curve
    ADD COLUMN intensity_points TEXT NOT NULL DEFAULT '[]'
        CHECK (json_valid(intensity_points));

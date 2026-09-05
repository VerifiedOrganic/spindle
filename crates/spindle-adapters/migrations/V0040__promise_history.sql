-- Unknown historical dates stay unknown. Never backfill actuals from plans.
ALTER TABLE narrative_promise ADD COLUMN status_history TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(status_history) AND json_type(status_history) = 'array');

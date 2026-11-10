-- V0057: Optional leap-year rule for project calendars.
--
-- Pre-existing calendars default to {"kind":"none"}, which keeps their
-- fixed-length years unchanged. A project opts into Gregorian or custom leap
-- years by setting the rule through set_project_calendar.
ALTER TABLE project_calendar
    ADD COLUMN leap_rule TEXT NOT NULL DEFAULT '{"kind":"none"}'
    CHECK (json_valid(leap_rule));

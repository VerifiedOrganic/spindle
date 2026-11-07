# SQLite migrations

Refinery `embed` migrations, applied in version order with `abort_missing` semantics.

- `V0035` is intentionally absent: it was never created. When the deep-check
  cache migration was written, `V0036` had already shipped, so it was numbered
  `V0039` instead — inserting a `V0035` below the last applied version would
  make refinery refuse to open already-migrated workspaces (see the header of
  `V0039__deep_check_cache.sql`).
- Policy: migration versions form a strictly increasing linear chain; never
  insert a version below the highest shipped number — always append above it.

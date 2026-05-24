# Spindle conventions

- Treat `docs/spindle-architecture.md` and `docs/spindle-implementation-brief.md` as the source of truth for current architecture.
- Keep `docs/` focused on current material. Do not reintroduce a `docs/legacy/` or `docs/new/` directory.
- Keep `spindle-core` free of MCP transport concerns.
- Keep public tool/resource DTOs in `spindle-core`; MCP should parse, invoke
  services, and serialize responses.
- Keep root `skills/` as the source of truth for embedded skill assets.
- Keep SQLite migrations under `crates/spindle-adapters/migrations/`.
- Prefer explicit typed contracts over loose JSON blobs.
- Branch and revision workflows are public MCP surface area; keep docs, tests,
  and skills aligned when changing them.

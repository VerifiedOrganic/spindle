# Changelog

## 0.2.0 — initial public release

Spindle ships as a local-first MCP server for fiction planning, drafting,
branching, revision, and story-bible search.

Highlights of the initial public surface:

- Five-crate Rust workspace: `spindle-core`, `spindle-adapters`,
  `spindle-skills`, `spindle-mcp`, `spindle-harness`.
- Embedded SQLite persistence with compiled migrations under
  `crates/spindle-adapters/migrations/`.
- Public MCP tool surface covering project structure, branching, revision,
  world and entities, plot and arc tracking, pacing, planning, consistency
  validation, semantic and full-text search, canonical-fact registration,
  model-agent routing, EPUB and bible export, the full drafting loop,
  writer-state and lookup, scene-source bridging, manuscript import, and
  knowledge recording.
- MCP resource surface for skills, project-scoped entity reads, branch and
  timeline graphs, paginated history, and direct entity reads through
  `bible://{table}:{id}` templates.
- Optional streamable HTTP MCP transport (`SPINDLE_HTTP_ADDR`) alongside the
  default stdio transport.
- Operator-driven `spindle-harness` CLI for batch drafting with editorial
  checkpoints and resumable artifacts.
- Embedded skill prompts in the binary, sourced from the root `skills/`
  directory at build time.

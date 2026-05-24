# Spindle

Spindle is a local-first MCP server for fiction planning, drafting,
branching, revision, and story-bible search.

It runs as a single Rust binary that exposes an [MCP](https://modelcontextprotocol.io/)
tool and resource surface. An MCP client (Claude Code, Claude Desktop, or any
other MCP-aware tool) connects over stdio (or, optionally, streamable HTTP) and
drives long-running authoring sessions against a local SQLite-backed story
bible. All state lives on your machine.

## Status

This repository contains an implemented multi-crate Rust workspace that has
progressed well beyond the original v0.1 slice. The current architectural
brief and contributor orientation live in:

- [`docs/spindle-architecture.md`](docs/spindle-architecture.md) — quick start
- [`docs/spindle-implementation-brief.md`](docs/spindle-implementation-brief.md) — architectural brief

See [`docs/README.md`](docs/README.md) for the full docs map.

The current workspace includes:

- `spindle-core` for shared models and contracts
- `spindle-adapters` for SQLite persistence, repositories, services, model
  routing, embedded guidance access, and search or embedding logic
- `spindle-skills` for build-time embedding of root `skills/`
- `spindle-mcp` for the stdio MCP server, tools, resources, and optional
  streamable HTTP MCP transport plus read-only operational endpoints
- `spindle-harness` for operator-driven batch drafting, checkpointing, and
  resume automation over the MCP surface

The server currently ships:

- embedded SQLite storage with compiled migrations from
  `crates/spindle-adapters/migrations/`
- canonical repo-local skills from root `skills/`
- the original v0.1 authoring loop
- post-v0.1 planning, pacing, consistency, branch, revision, and search tools

## Build

Run the workspace validation commands from the repo root.

```bash
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Wall-clock performance regressions are gated out of the default suite. Run them
explicitly when needed.

```bash
cargo test -p spindle-core --features perf
cargo test -p spindle-adapters --features perf
```

## Run the MCP server

Run the stdio MCP server from the workspace root.

```bash
cargo run -p spindle-mcp
```

By default, the server stores local data under your platform local data
directory in `spindle/`. Set `SPINDLE_DATA_DIR` to override that path.

Spindle also supports optional runtime model-agent configuration through
`spindle.toml`. Set `SPINDLE_CONFIG` to point at a config file explicitly, or
place `spindle.toml` in the repo root or `~/.spindle/config.toml`. See
`docs/spindle-agent-config.md`. The existing `embedding` route can now be bound
to an OpenAI-compatible embedding model for higher-quality Bible search while
keeping `token-hash-v1` as the zero-config fallback.

## MCP tools

The current MCP tool surface includes:

- session and project catalog: `list_projects`, `set_active_project`
- project structure: `create_project`, `create_book`, `create_chapter`
- branches and revision: `create_branch`, `switch_branch`,
  `create_save_point`, `restore_save_point`, `diff_branches`, `merge_branch`,
  `revise_scene`, `generate_alternatives`, `compare_alternatives`,
  `select_alternative`, `list_revision_markers`, `resolve_revision_marker`,
  `list_scene_versions`, `restore_scene_version`
- world and entities: `create_character`, `create_location`,
  `create_faction`, `create_religion`, `create_economy`, `create_term`,
  `batch_create_terms`, `create_relationship`, `create_world_rule`,
  `update_world_rule`, `set_character_voice_profile`,
  `batch_set_character_voice_profiles`, `update_entity`, `archive_entity`
- plot and arc tracking: `create_plot_line`, `create_conflict`,
  `create_theme`, `create_motif`, `batch_create_motifs`,
  `create_narrative_promise`, `batch_create_narrative_promises`,
  `update_promise_status`, `create_character_arc`, `create_future_knowledge`,
  `create_timeline_event`, `create_temporal_intervention`,
  `create_system_overlay`
- pacing and planning: `create_pacing_config`, `create_pacing_curve`,
  `set_arc_pacing_constraints`, `plan_chapter`, `annotate_scene_beats`,
  `save_summary`, `set_book_outline`, `set_chapter_outline`
- analysis and search: `check_consistency`, `search_bible`,
  `find_scenes_referencing`, `rebuild_search_index`,
  `run_dual_persona_review`
- canonical facts: `register_canonical_fact`,
  `extract_canonical_facts_from_scene`, `migrate_canonical_fact`
- model config: `configure_agents`, `list_agents`, `test_agent`
- model generation, research, and export: `continue_generation`,
  `revise_generation`, `research_query`, `export_epub`,
  `preflight_book_export`, `export_bible`
- drafting loop: `get_scene_context`, `get_chapter_briefing`,
  `save_scene_draft`, `move_scene`, `delete_scene`, `operator_delete_scene`,
  `commit_scene_changes`, `commit_character_state`, `update_relationship`,
  `set_narrator_voice`
- writer state and lookup: `get_writer_state`, `get_entity`, `find_entity`,
  `get_character_snapshot`, `list_chapter_scenes`, `list_book_chapters`,
  `record_note`, `update_writer_position`
- scene source bridging: `backfill_scene_source_offsets`,
  `pull_chapter_from_file`, `push_chapter_to_file`
- import and knowledge: `import_manuscript`, `import_status`,
  `import_extract_entities`, `import_consolidate_entities`,
  `import_analyze_character`, `import_extract_world`,
  `import_analyze_narrative`, `import_compute_final_state`,
  `import_hydrate_bible`, `import_apply_review_decisions`,
  `record_knowledge`
- skill bootstrap: `init_grok_skills` (writes the embedded skill prompts to
  a target directory for clients that load skills from disk)

`get_scene_context` also accepts an optional `max_character_count` for explicit
large-cast trimming. When omitted, current drafting behavior is unchanged.

EPUB export recognizes LitRPG system UI blocks in scene prose using fenced
syntax. Supported classes are `system-box`, `system-notification`,
`system-pull`, and `system-quest`; plain `system` is accepted as an alias for
`system-box`. Either a pandoc-style fenced div or a backtick code fence whose
info string names a system class is accepted:

```text
::: system-box                ```system-box
STAGE CRED EARNED: +2.        STAGE CRED EARNED: +2.
:::                           ```
```

These blocks render as styled XHTML `div` elements in exported EPUB files.
Backtick fences with any other info string (e.g. ` ```rust `) are left as
ordinary prose.

## Skills and resources

The binary embeds every repo-local skill in `skills/*/SKILL.md` at build time.
The root `skills/` directory remains the canonical source of truth.

Current embedded skill resources include:

- `bible://skills/bible-librarian`
- `bible://skills/scene-writer`
- `bible://skills/character-creator`
- `bible://skills/continuity-editor`
- `bible://skills/editor`
- `bible://skills/manuscript-importer`
- `bible://skills/plot-architect`
- `bible://skills/revision-manager`
- `bible://skills/worldbuilder`

Project-scoped resources use opaque IDs and support nested paths under the
project prefix, for example:

- `bible://projects/{project_id}/characters`
- `bible://projects/{project_id}/locations`
- `bible://projects/{project_id}/factions`
- `bible://projects/{project_id}/world-rules`
- `bible://projects/{project_id}/books`
- `bible://projects/{project_id}/chapters`
- `bible://projects/{project_id}/chapter-summaries`
- `bible://projects/{project_id}/plot-lines`
- `bible://projects/{project_id}/conflicts`
- `bible://projects/{project_id}/themes`
- `bible://projects/{project_id}/motifs`
- `bible://projects/{project_id}/narrative-promises`
- `bible://projects/{project_id}/reader-contract`
- `bible://projects/{project_id}/branches`
- `bible://projects/{project_id}/continuity/health`
- `bible://projects/{project_id}/future-knowledge`
- `bible://projects/{project_id}/timeline-events`
- `bible://projects/{project_id}/timeline-graph/mermaid`
- `bible://projects/{project_id}/temporal-interventions`
- `bible://projects/{project_id}/system-overlays`
- `bible://projects/{project_id}/dual-persona-reviews`
- `bible://projects/{project_id}/relationships`
- `bible://projects/{project_id}/character-arcs`
- `bible://projects/{project_id}/religions`
- `bible://projects/{project_id}/economies`
- `bible://projects/{project_id}/terms`
- `bible://projects/{project_id}/research-log`
- `bible://projects/{project_id}/pacing/overview`
- `bible://projects/{project_id}/imports`
- `bible://projects/{project_id}/imports/{session_id}`
- `bible://projects/{project_id}/imports/{session_id}/structure`
- `bible://projects/{project_id}/imports/{session_id}/review-items`
- `bible://projects/{project_id}/imports/{session_id}/hydration-report`

The server also exposes:

- `bible://references/{reference-name}` for embedded craft references
- `bible://system/model-routes` for read-only adapter routing metadata
- `bible://config/agents` for sanitized configured agent state
- `bible://config/routing` for configured route assignments

Parameterized local reads are also available through MCP resource templates:

- `bible://{table}:{id}` for direct entity reads
- `bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes`
  for active-branch scene ids and summaries for one chapter
- `bible://projects/{project_id}/scene-delete-impact/{book_number}/{chapter_number}/{scene_order}`
  for a read-only scene deletion impact audit on the active branch
- `bible://projects/{project_id}/scene-move-impact/{from_book_number}/{from_chapter_number}/{from_scene_order}/{to_book_number}/{to_chapter_number}/{to_scene_order}`
  for a read-only scene move / reorder impact audit on the active branch
- `bible://projects/{project_id}/research-log/{offset}/{limit}`
  for paginated `research_query` history, newest first
- `bible://projects/{project_id}/conflicts/{offset}/{limit}`,
  `future-knowledge/{offset}/{limit}`, `timeline-events/{offset}/{limit}`,
  `temporal-interventions/{offset}/{limit}`, `dual-persona-reviews/{offset}/{limit}`,
  and `relationships/{offset}/{limit}` for paginated project records

Direct entity reads follow the `bible://{table}:{id}` template. For example:

- `bible://character:{character_id}`
- `bible://world_rule:{world_rule_id}`
- `bible://scene:{scene_id}` after `save_scene_draft` returns a scene id

## Optional HTTP mode

Run the binary with `SPINDLE_HTTP_ADDR` to expose the streamable HTTP MCP
transport at `/mcp` instead of stdio. The same router also exposes small
read-only operational endpoints.

<!-- prettier-ignore -->
> [!NOTE]
> This is an experimental feature currently under active development.
> When `SPINDLE_HTTP_ADDR` is unset, the binary starts only the stdio MCP
> transport for the current client and starts an internal HTTP MCP listener for
> secondary local clients. The public `SPINDLE_HTTP_ADDR` listener exposes the
> full MCP surface at `/mcp`; only `/health`, `/model-routes`, and `/events` are
> read-only operational endpoints.

```bash
SPINDLE_HTTP_ADDR=127.0.0.1:8787 cargo run -p spindle-mcp
```

Available endpoints:

- `/mcp` for streamable HTTP MCP clients
- `GET /health`
- `GET /model-routes`
- `GET /events` for SSE snapshots of model-route state only

## Harness

Use `spindle-harness` when you want operator-driven batch drafting with
checkpointed editorial review and resumable artifacts.

```bash
cargo run -p spindle-harness -- --help
```

See `docs/spindle-harness-usage.md` for the full workflow.

## Claude Code MCP config

Use the repo-local MCP config or mirror it in your own client config.

```json
{
  "mcpServers": {
    "spindle": {
      "command": "cargo",
      "args": ["run", "-p", "spindle-mcp"],
      "cwd": "/absolute/path/to/spindle"
    }
  }
}
```

## A first session

A typical session against a fresh Spindle install looks like this. Each step
is a single MCP tool call your client makes on your behalf.

1. `create_project` — create the story-bible project. The returned project id
   becomes the default for subsequent calls in the session.
2. `create_book` then `create_chapter` — set up the book and chapter you want
   to draft into.
3. `create_character`, `create_location`, `create_world_rule`, etc. — seed any
   canon you already know. The `worldbuilder` and `character-creator` skills
   walk through this.
4. `plan_chapter` — sketch the scenes for the current chapter. Returns scene
   slots you can later fill.
5. `get_scene_context` — assemble the full writing packet for a scene:
   characters, locations, relevant world rules, pacing directives, narrative
   promises due, recent summaries.
6. Generate prose in your MCP client using that context.
7. `save_scene_draft` — persist the draft and get back a scene id you can
   read at `bible://scene:{id}`.
8. `commit_scene_changes` — register character-state updates, canonical facts,
   and relationship deltas the scene implies.
9. `annotate_scene_beats` then `save_summary` — record beats and a chapter
   summary so the next scene's context stays tight.

Drive that loop across chapters and `spindle` keeps the story bible
consistent. For unattended batch drafting see
[`docs/spindle-harness-usage.md`](docs/spindle-harness-usage.md).

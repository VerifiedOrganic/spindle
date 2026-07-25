# Changelog

## Unreleased

### Added

- New `list_skills`, `get_skill`, and `get_reference` MCP tools expose the
  embedded workflow skills and craft references to clients without MCP resource
  support (e.g. Kimi): they return the same content as the `bible://skills/*`
  and `bible://references/*` resources and are available in every tool profile.

### Security

- An explicit generation receipt now authorizes exactly one scene. The first
  explicit `save_scene_draft` presenting a `generation_id` binds it to that
  scene's `(book, chapter, scene_order)` placement (migration V0034); a later
  explicit save on a *different* scene is rejected before any prose is
  persisted. Re-saving the same scene with the same receipt stays legal, so
  retries and operator re-saves still work. Previously one trip through the
  explicit-capable agent blanket-authorized explicit saves across every other
  scene, each stamped `agent:{id}` as though that agent had produced it.
- `get_scene_context` no longer hands an explicit scene's prose to a
  non-explicit drafting agent. When the preceding scene is Explicit and the
  scene being drafted is not, `previous_scene_tail.excerpt` carries the
  neighbour's summary instead of its closing prose and the new
  `elided_reason` field explains the substitution. Elision is a rating
  boundary, not a blanket filter — an Explicit scene following an Explicit one
  still receives the real prose — and it fails closed when the target scene's
  rating cannot be established. See "Mixed-rating chapters" in
  `docs/spindle-agent-config.md`.

### Fixed

- `docs/spindle-agent-config.md` no longer claims `save_scene_draft` persists
  the server-held generation output for explicit saves. The caller's
  `full_text` has been authoritative since the receipt-stub data-loss fix; the
  receipt is provenance only.
- `create_pacing_config` and `create_pacing_curve` no longer fail with
  "pacing_config not found" / "pacing_curve not found" when a row already
  exists for the project+branch (the upsert conflict path re-read by the
  freshly minted id, which the kept row never had; the read-back now uses the
  upsert key, and recreating returns the original row id).
- Authoring runs no longer deadlock over pre-existing content: a chapter with
  persisted prose and a persisted summary from outside any run is adopted on
  reconcile (scenes `beats_annotated`, `summary_saved` set) instead of blocking
  on the summary residue rule; runs already stuck at `draft_saved` self-heal on
  the next execute. `authoring_resolve_block` now also advances scenes in a
  run blocked at RUN level (and no longer requires an artifact for
  pre-existing-prose scenes); its refusal message distinguishes scene-level
  from run-level blocks.
- The save-summary step no longer honors a stale summary artifact as
  idempotency proof: the referenced `chapter_summary` row must still exist on
  the run's branch, and artifacts are stamped with (and checked against) the
  producing `run_id`; otherwise the summary is regenerated from the current
  scene packages and persisted.
- Re-saving a scene through `authoring_save_scene_draft` in a chapter whose
  summary was already persisted (the revise-during-checkpoint flow) deletes
  the now-stale summary row and artifact and resets `summary_saved`, so the
  run resumes and regenerates the summary post-approval instead of blocking.
  Completion-order validation now tolerates such reopened chapters (only a
  chapter with zero progress before a complete one is a `completion_gap`).
- `decide_canon_deltas` normalizes miner payload shapes before pre-flight and
  apply, so previously staged rows ratify without manual `edit`
  reconstruction: bare ULIDs gain their class table prefix
  (`narrative_promise:`/`conflict:`/`character_arc:`/`character:`), plural
  table/kind names singularize (`characters` → `character`),
  `description` maps to `summary`/`definition`, `role` defaults to `minor`,
  null profile structs are dropped, and a free-string `planted_at` coerces to
  the mined scene's placement (the string survives as the placement note).
- `create_character` accepts missing or null `voice_profile` and
  `emotional_profile` (they default to empty profiles).
- Deterministic `temporal_coherence` scan: "before lunch"-style deadline idioms,
  gnomic present-copula statements ("second breakfast is …"), and sentences
  with contracted past perfect ("I'd … at midnight") no longer anchor scene
  time; "two in the morning" parses as a clock hour (night) instead of the
  word "morning"; `by breakfast/lunch/supper/dinner/midnight` join the
  transition markers. The world-rule scan no longer promotes hits to Likely on
  the bare function words "without"/"despite".
- The model-backed world-rule compliance prompt now carries the scene's
  declared `content_rating` and instructs rating-scoped mandates to be judged
  against the declared tier, not on-page intensity.
- `run_dual_persona_review` rounds accumulate across reviews of the same prose
  version (fingerprint-matched top-up), so the checkpoint review-currency gate
  can be satisfied incrementally; a prose change still resets the count.
- `get_chapter_briefing` keeps recent chapter summaries under token-budget
  pressure: the section's trim priority rose above outlines/threads/plan, and
  a budget trim keeps the immediately preceding chapter's summary instead of
  clearing the list and falsely reporting "None recorded before this chapter".

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

# Changelog

## Unreleased

### Added

- New `list_skills`, `get_skill`, and `get_reference` MCP tools expose the
  embedded workflow skills and craft references to clients without MCP resource
  support (e.g. Kimi): they return the same content as the `bible://skills/*`
  and `bible://references/*` resources and are available in every tool profile.
- A panic in any MCP tool handler now returns an error naming the panic instead
  of unwinding past the response writer, which left the caller waiting on a
  response that never came (a ~60s hang with the panic visible only on stderr).
- `delete_scene`, `operator_delete_scene`, and `move_scene` accept an optional
  `scene_id` that identifies the row unambiguously. With `scene_id` the
  position fields become optional (and are validated against the row's actual
  placement when supplied); the scene must be on the project's active branch.
  Position-only addressing is unchanged, but it could not disambiguate rows
  sharing a position and silently acted on whichever row the resolver picked —
  the root hazard behind a live-run deletion of a real scene.
- `list_chapter_scenes` returns `unresolved_alternatives`: scenes on
  `alternative`-type branches that were never promoted to the active branch
  (the unselected output of `generate_alternatives`). They are reported
  beside the spine — never inside it — so they can no longer be mistaken for
  spine scenes or deleted by position.
- `compile_manuscript` returns `stub_scenes` (`chapter.scene` refs) alongside
  `missing_scenes`: drafted scenes whose body matches obvious placeholder
  text or sits below the minimum scene word count. They still render into the
  Markdown but are named so they are never read as finished prose.
- `preflight_book_export` flags stub scenes with a Blocking `scene_stub_text`
  issue, and `check_consistency` gained a `scene_stub_text` check
  (severity `info` — stubbiness is a manuscript-readiness observation, not an
  in-story consistency violation, and the authoring loop's checkpoint
  policies approve/block on errors+warnings). A scene is a stub when its
  entire body is an obvious placeholder marker (`placeholder`, `TODO`,
  `TBD`, …) or its word count is below the floor. The floor is per-project:
  `project.min_scene_word_count` via `update_entity` (migration V0036),
  defaulting to 20 words.
- New `update_arc_milestone` tool updates a single character-arc milestone by
  label — move its `placement` and/or stamp `reached_at` — while preserving
  the fields the caller didn't pass (`description`, `unlocks`). Previously the
  only write path was resending the whole milestones array through
  `update_entity`, but `get_entity(table="character_arc")` and
  `get_character_snapshot` never projected milestones, so the array was
  effectively write-once and a placement edit guaranteed data loss. Both
  snapshots now project the full milestone array, and the
  `arc_milestone_audit` advice names the new tool.
- Characters can be renamed. `update_entity` on a character with
  `changes: {"name": ...}` previously failed with "column 'name' is not in
  the update allowlist", so a co-lead created before its in-world naming
  scene could never receive its name — authors carried the real name in the
  summary and hoped every drafting model read it. Renaming is a controlled
  operation: pass `allow_rename: true` and the call moves the name's
  project-wide uniqueness key, keeps the old name as an alias, refreshes the
  search index (FTS + embeddings), and returns a `rename_report` listing the
  scenes, canonical facts, knowledge facts, and arcs still referencing the
  old name so the author can fix them — scene prose is never rewritten
  automatically. Without the flag the rename is rejected with guidance, and
  a colliding name (any branch) names the character that holds it.
- Characters have an `aliases` field (migration V0037): settable at
  `create_character` time and amendable through
  `update_entity { changes: {"aliases": [...]} }`. Writes are shape-validated
  as an array of strings before the row is touched (`json_valid` is not
  enough; a wrong-shaped value would persist and then break every later
  `get_character` / snapshot read). Aliases resolve through `find_entity`
  exactly like the primary name and render in subject snapshots, so a record
  can keep its stable working label and carry the in-world name once it is
  decided — no rename needed. Rename preserves the old name here
  automatically. The `fts_character` index was rebuilt to include aliases.
- `register_canonical_fact` no longer requires a scene: omit `scene_id` to
  register a fact decided during planning but not yet dramatised (e.g. a
  name locked by author decision before its scene exists). The fact is
  planned-and-pending — placed by `book_number`/`chapter_number` on the
  active branch, applying in consistency checks from that placement — and
  the new `bind_canonical_fact_to_scene` tool attaches it to its scene once
  that scene is drafted (migration V0038 makes the column nullable).

### Fixed

- `reader_contract` is no longer immutable. It was set once at `create_project`
  and could never be changed — `update_entity` rejected `reader_contract`,
  `promise`, `style_notes`, and `boundaries` as "not in the update allowlist"
  because the contract lives in a single JSON column the per-column allowlist
  cannot address. This corrupted a manuscript: a stale `style_notes` chapter
  length ("1,200–2,000 words") silently overrode an explicit author directive
  ("3,500–5,000") because narrator voice and the structure world rule could be
  updated but the contract could not, and four of five drafted chapters landed
  in the superseded band. Now `update_entity(entity_type="project", ...)`
  accepts `reader_contract` (a nested object) and the sub-fields `promise`,
  `style_notes`, `boundaries`, applied as a validated partial merge — unset
  fields are preserved, wrong-shaped values are rejected before any write, and
  the StyleCompliance cache is invalidated so drafting reads the new figure.
  `apply_style_profile` is unchanged: it still only touches profile-generated
  notes, so an author's original notes are edited deliberately through this
  path.
- New contradiction guard: when `reader_contract.style_notes`, the narrator
  voice, or a style world rule is updated, Spindle cross-checks the word-count
  targets across all three surfaces and returns a `warnings` entry on
  `update_entity` / `set_narrator_voice` / `update_world_rule` when they
  disagree (e.g. the contract still says 1,200–2,000 while the narrator voice
  says 3,500–5,000). Advisory, never blocking — so a stale figure is surfaced
  at update time instead of being noticed in a word count after export.
- `create_style_profile_from_markdown` no longer produces permanently
  unappliable profiles. Guidance synthesis is model-populated, but the prompt
  only referenced "the StyleProfileGuidance schema" by name and never included
  it, so real models could not conform: the response failed to parse and the
  call silently degraded to a `NeedsReview` profile with every guidance field
  empty — which `apply_style_profile` then rejected with no escape hatch. A
  profile could be created but never applied. Now:
  - the synthesis prompt injects the actual `StyleProfileGuidance` JSON Schema
    (generated from the type, so it cannot drift), telling the model the exact
    shape to emit;
  - parsing is lenient (`#[serde(default)]` on every guidance field) so a
    partial response degrades to empty fields instead of failing wholesale,
    and an all-empty result is detected explicitly;
  - `create_style_profile_from_markdown` fails loudly — before persisting —
    when guidance comes back empty, naming the route and the `metrics_only`
    opt-out, instead of reporting success and creating an unusable record
    (metrics-only profiles are exempt by design);
  - `apply_style_profile` gains `force: Option<bool>`: forcing activates a
    `NeedsReview` or metrics-only profile deliberately, and when there is no
    prose guidance it activates without touching the project's narrator voice,
    style notes, or style world rule — so metrics-only profiles can back drift
    detection without clobbering existing prose style. `force_apply` on create
    now also carries through to the apply step.
- `update_entity` allowlist audit: records are no longer near-immutable after
  creation. Authoring scaffolds a record first and deepens it as decisions
  land, but most planning fields were frozen at create time, forcing canon
  into free-text summary fields and defeating the structured records they
  belong in. Now updatable after creation:
  - `character_arc`: `ending_state`, `starting_state`, `thematic_purpose`,
    `arc_type`, `connected_theme_ids`, `status` (an arc's destination is
    among the likeliest fields to change during planning; a live record was
    stuck reading "OPEN, deliberately so" after the decision was made).
    `progress` stays locked (computed), as do the id links.
  - `character_emotional_profile`: `base_emotions`, `suppressed`, `triggers`,
    `defense_mechanisms`, `flex_range`. Profiles were write-once at
    `create_character`, so characters scaffolded before their psychology was
    worked out — most supporting cast — were permanently stuck with empty
    profiles. Writes are shape-validated against the stored types before any
    column is touched (the schema only checks `json_valid`, and a
    wrong-shaped value would poison every later read of the profile, which
    briefings embed).
  - `world_rule`: `rule_type`, `scan_pattern`, `established_in`,
    `relevance_tags` via `update_world_rule` (which routes through the same
    allowlist and whose cache invalidation already anticipated rule_type
    edits); `faction`: `faction_type`, `realm`, `tags`; `religion`: `tags`;
    `economy`: `realm`, `scarce_resources`, `trade_goods`; `term`:
    `usage_context`, `origin`; `plot_line`: `plot_type`; `theme`:
    `introduction_point`, `resolution_point`; `motif`:
    `max_uses_per_chapter`, `connected_theme_ids`; `narrative_promise`:
    `planted_at` (symmetric with `planned_payoff` after a renumber);
    `timeline_event`: `title`, `event_type`, `placement`, `summary`,
    `related_entity_ids` (previously entirely locked, so a renumber stranded
    event placements with no fix path); `temporal_intervention`:
    `intervention_type`, `summary`, `consequences`, `status`;
    `system_overlay`: `system_type`, `rules`, `visibility`,
    `progression_currency`, `stats`, `advancement_tiers`; `future_knowledge`:
    `knowledge_summary`, `source`, `learned_at`, `expires_at`.
  Still locked by design: name-family identity columns (they back uniqueness
  indexes and the search index — renames stay controlled operations), FKs,
  and columns with dedicated typed write paths (voice profiles, scene text,
  canonical facts).
- `set_character_birth` accepts negative `day_index`. Day 1 is the story's
  opening, so every adult character was born before it; the old `>= 0` bound
  made the tool unusable for any project not set at the literal dawn of its
  own timeline, forcing ages into prose summaries where nothing can derive or
  validate them. The clock is a signed offset from the epoch everywhere it is
  consumed (ordering, elapsed-time math, rendering), and births/backstory
  timeline events now anchor pre-epoch as expected. `duration_days` remains
  non-negative (it is a magnitude), and the intra-day `time_of_day` bounds
  are unchanged.
- Verified: mixed-type `update_entity` payloads — one string field plus one
  array field in the same `changes` object — apply every field at both the
  MCP parse boundary and the service write path. An earlier failure rejected
  the combination with "changes must be a JSON object" despite `changes`
  being a valid object (the message pointed at the wrong thing entirely);
  regression tests now pin the behavior at both layers, including the
  stringified-object form some clients send.
- `scan_pattern` no longer matches inside longer words. The patterns were
  compiled as bare substrings, so a Stat Growth Rate rule with pattern
  `stat` fired on the four bytes inside "statement" (and "station",
  "status", …). Plain patterns now anchor to word boundaries on each end
  whose adjacent character is a word character, with the pattern wrapped in
  a non-capturing group so alternations stay bounded. **Behavior change:**
  patterns that relied on prefix matching (for example `resurrect` catching
  "resurrected"/"resurrection") must opt in explicitly, for example
  `resurrect\w*`. The scanner semantics version bumped to v3, so cached
  `world_rule_semantic_drift` findings are recomputed on the next
  `check_consistency` run.
- `pull_chapter_from_file` round-trips real edits. Pull used to re-derive
  scene boundaries with the import slicer's structural heuristics
  (blank-line transitions, separator lines), so any edit that changed byte
  lengths — or added a paragraph pattern the slicer reads as a scene
  break — failed with "could not map N scene range(s)" or, worse, shifted
  later scenes onto the wrong text. Files written by `push_chapter_to_file`
  are now mapped by splitting on a dedicated scene marker
  (`<!-- spindle-scene -->`), not a markdown thematic break: length-changing
  edits pull cleanly, an editor's trailing newline is trimmed rather than
  adopted, and adding/removing a scene in the file fails with a clear
  count-mismatch error naming the recovery path. Files previously pushed
  with the old `\n\n---\n\n` delimiter still split that way when the piece
  count matches the chapter; otherwise (and for external manuscripts) the
  import slicer is used. The `scene_divergence` remediation now names
  `pull_chapter_from_file` as the way to adopt file edits (re-pushing
  discards them).
- `update_entity` now allowlists `narrative_promise.planned_payoff` and
  `scene.summary`. After a chapter renumber every promise kept pointing at
  its old payoff chapter and `narrative_promise_tracking` emitted permanent
  "past planned payoff" warnings with no API path to retarget them
  (`update_promise_status` takes only status/note); the tracking advice now
  names the `update_entity` path. Scene summaries were likewise frozen at
  `save_scene_draft` time, so renumbers left stale cross-references that
  `get_chapter_briefing` kept feeding into drafting context.
- `world_rule_semantic_drift` no longer fires secrecy-class rules on
  narration or HUD text. A secrecy rule ("Nate must never disclose the
  N.A.I.P.") constrains disclosure to other characters, but the scanner
  matched the literal `scan_pattern` anywhere in scene text — interior
  narration and bracketed interface readouts included — so every correct
  usage fired and real violations were buried. Secrecy-class rules
  (classified from the rule type/name, or non-disclosure phrasing in the
  description, so existing rows need no migration) now only flag hits inside
  quoted dialogue spans; the commit gate and `check_consistency` share the
  same classifier. The validator cache context hash now also mixes in a
  scanner semantics version plus the classification inputs (`rule_type`,
  `description`): previously it keyed on rule metadata alone, so findings
  computed by the old scanner kept being served from cache after the
  scanner changed, making the fix invisible until every scene or rule was
  touched. Stale rows are retired automatically on the next
  `check_consistency` run.
- `create_chapter`'s description claimed `chapter_number` is validated
  against the next sequential slot; it isn't — arbitrary numbers with gaps
  are accepted, which is what makes restructures possible. The doc now
  describes the real behavior, and the success response echoes the
  chapter's effective `title`.
- `push_chapter_to_file`'s outside-`data_dir` error named `source_path`;
  the tool's parameter is `target_path` and the message now says so.
- The stdio server no longer dies during startup against clients that probe with
  a vendor-specific request before `initialize`. Antigravity (`agy`) opens the
  pipe with a proprietary `server/discover` request, and rmcp treats any
  pre-`initialize` message except `ping` as fatal, so Spindle exited with
  `expect initialized request, but received: ... "server/discover"` and never
  advertised a tool — the client showed the server as errored. Client traffic
  now passes through a handshake shim that answers unknown pre-`initialize`
  requests with JSON-RPC `-32601` (what standard SDK servers do), drops unknown
  pre-`initialize` notifications, and hands everything else to rmcp untouched.
  After the `notifications/initialized` handshake the shim is a pass-through.
- `run_dual_persona_review` no longer loses everything to a client timeout.
  Rounds ran inline with a single persist at the very end, so when the MCP
  client gave up (~60s) the whole run was discarded — the `dual_persona_review`
  table held zero rows despite every model call having been paid for, and the
  retry restarted from nothing. On a reasoning model a two-round review is
  minutes of wall time, so this was the normal outcome, not an edge case. The
  call now starts (or joins) a detached job, waits 25s, and returns either the
  finished review or `status: "in_progress"` with the rounds banked so far; the
  job keeps running and persists EACH round as it lands. A retry joins the same
  job rather than starting a duplicate, and collects the finished result instead
  of stacking another `rounds` on top of it. An in-flight job whose scene was
  edited does not overwrite the newer fingerprint's banked rounds: persist is
  refused unless the live prose still matches the job.
- The three review personas now dispatch concurrently instead of in sequence, so
  a round costs one model call of wall time rather than three. (A two-round
  review with a style contract configured was six sequential calls.)
- `check_consistency` with `deep_check: true` now makes forward progress across
  retries over a chapter range. The per-scene `temporal_coherence` and
  `world_rule_compliance` tiers cache each scene's raw model output keyed on the
  scene's prose fingerprint (migration V0039) and fan out with bounded
  concurrency, so a retry re-parses analyzed scenes for free and only calls the
  model for the remainder. Editing a scene misses the cache and re-analyzes. The
  "a dead or uncleared route costs exactly one attempt" guarantee is preserved:
  the first scene is probed alone before the rest fan out.
- `authoring_status` no longer times out while the run it reports on is mid-step.
  Tool calls are serialized per project, and a drafting or review step holds that
  lock for as long as its model call takes — minutes on a reasoning model — so
  the one tool that could say whether the slow step was still alive queued behind
  it. Status now degrades to the existing non-persisting reconcile when the
  project lock is busy, instead of blocking.
- `authoring_execute_next` no longer hangs until the MCP client times out when
  `.spindle/runtime/spindle.addr` names a dead listener. Reclaiming a stale addr
  file ran its liveness probe on a nested current-thread runtime via
  `block_on`, which panics ("Cannot start a runtime from within a runtime") on
  the async dispatch path every real caller uses — so the stale addr was never
  reclaimed and the whole authoring loop (verify, commit, beats, summary, draft
  route resolution) stalled with no state advance. The probe now runs on the
  caller's runtime. The unit test that covered reclamation ran as a plain
  `#[test]`, off any runtime, and passed against the broken path; it and the new
  regression test now exercise it from async.
- `list_chapter_scenes` (and `list_book_chapters`) no longer report scenes
  from OTHER branches as if they were the chapter's spine. The listing query
  filtered by `chapter_id` only, but books and chapters are branch-shared —
  so the never-selected alternatives `generate_alternatives` stores on their
  own branches leaked into the listing, producing live-run chapters with five
  listed scenes and three of them at one `scene_order`. The listing now
  shares the ONE branch-scoped spine predicate with `compile_manuscript` and
  the position resolver (`Repository::list_scenes_by_chapter_and_branch`), so
  the listing, the compiled manuscript, and position addressing can never
  disagree about which scenes are in the spine. A contract test pins the
  invariant (listing ≡ compiled spine ≡ position-addressable).
- A FAILED `save_scene_draft` no longer consumes the explicit generation
  receipt. The receipt claim ran as its own committed write before placement
  validation, so a save that then failed (`book 0 not found for project`)
  left the receipt bound to the bogus placement and burned it — forcing a
  full regeneration of explicit prose that had already been produced and paid
  for. The claim now commits inside the scene's save transaction (migration
  V0034 semantics, enforced atomically): a save that fails at any point
  leaves the receipt unbound and reusable.
- `save_scene_draft` now resolves book/chapter placement from `chapter_id` as
  its schema documents, and rejects calls that name neither `chapter_id` nor
  positive `book_number`/`chapter_number` with a validation error. Previously
  the numbers silently defaulted to 0 and the save failed deep in persistence
  with `book 0 not found for project` — which is also what corrupted the
  receipt claim above. Contradictory `chapter_id` + numbers are rejected
  loudly, mirroring `get_scene_context`.
- `create_chapter` persists the `title` it accepts. Previously the field was
  silently dropped and a later `preflight_book_export` reported
  `chapter_missing_title` for a chapter created with a title. An existing
  chapter's title is never clobbered — the ensure path only fills a missing
  one.

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

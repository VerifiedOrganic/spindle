# Changelog

## Unreleased

### Added

- Per-route reasoning effort for OpenAI-compatible HTTP agents. A
  `[[routing]]` entry may set `reasoning_effort` (`none|low|medium|high|max`)
  and override the agent-level default, so one agent can draft at high effort
  while review and deep consistency checks run at low effort. Spindle sends the
  field as `reasoning_effort` in the request body, records the effective value
  on the `model_call` payload, and exposes it through `bible://config/routing`.
- `extra_body` tables on HTTP agents and routing rules for provider-specific
  request fields. Route entries merge over agent entries; core fields (`model`,
  `messages`, `max_tokens`, `stream`, `reasoning_effort`) are rejected at load
  time so they can never be silently overridden, and values are never logged.

### Fixed

- **Planning responses stay displayable.** `planning_status` now returns only
  `proposal_id`, `level`, `status` and `payload_hash` per proposal by default,
  and omits superseded and rejected history. Pass `include_payloads=true` to
  include operations and schedule, or name `proposal_ids` to fetch specific
  proposals (including superseded ones) by id. `planning_advance` and
  `planning_decide` return the same compact proposal form by default; pass
  `include_payloads=true` when the full artifact is needed. Host-generation
  prompts are bounded, and oversized parent artifacts are referenced by
  proposal id and payload hash instead of being repeated in full, so
  `planning_advance` no longer returns a 200k-character prompt. Routed model
  generation keeps full parent context because the routed model cannot fetch
  payloads; revision feedback is ordered newest-first so bounding cannot drop
  the latest author direction. Response schemas, tool snapshots and profile
  budgets are updated in the same change.
- **Respawn and resurrection no longer read as dead-character retcons.** A
  character state that replaces a recorded dead status with a living one
  supersedes the death from that scene onward, including when the state is
  committed in the same `commit_scene` or `commit_scene_changes` call as the
  scene. The scan uses the shared newest-state selector and treats the
  prospective same-scene state as newest. A per-character `mortality` setting
  (`respawns`, `resurrection`, `immortal`, and similar values) or a world rule
  that declares a return-from-death mechanism also exempts that character; a
  rule that forbids resurrection does not. A death followed by action with no
  respawn recorded still reports, as does a same-commit status that keeps the
  dead marker.
- **Deep consistency checks no longer lose their result to a client timeout.**
  `check_consistency` with `deep_check: true` now starts a detached job and
  returns `status: "in_progress"` with a `job_id`, the deep checks finished so
  far, and the findings banked so far. Calling again with the same project,
  scope, check selection, and unchanged prose joins the job or collects its
  finished result; fast checks stay synchronous. When the client supplies a
  progress token, the server emits `notifications/progress` after each deep
  check. Jobs are in-memory, so a server restart drops the job and the next call
  starts fresh while reusing per-scene deep-tier cache rows. Checkpoint audit
  recording rejects an in-progress payload instead of blessing partial coverage.
- Research search now splits multi-word queries into terms and matches them
  across source titles, note text, claim text, and tags, ranking results by the
  number of matching terms. Tag matching normalizes case, hyphens,
  underscores, and spaces, so `supply-lines`, `supply_lines`, and
  `supply lines` are equivalent. The authoring research gate uses the same
  matching, so a scene no longer stays blocked when its multi-word research
  tags overlap existing note tags.

## 0.4.1 — 2026-11-13

### Added

- **One MCP server again.** The generated `full` startup profile composes every
  model-facing skill and reference unit (177 tools), restoring one-server
  access to the complete skill-declared model surface without the removed
  `legacy-all` compatibility catalog. It is the largest catalog at about 43.5k
  input tokens of schema, so prefer a smaller profile unless one session must
  cover everything. Operator-only commands remain CLI-only, and unset still
  starts `workflow-write`.
- Runtime profile switching on one MCP server: `use_profile` validates a
  generated profile name, changes the session's active profile, returns the new
  tool list summary, and emits `notifications/tools/list_changed` (with the
  `tools.listChanged` capability declared), so capable clients refresh
  `tools/list`. Startup `SPINDLE_TOOL_PROFILE` remains the initial profile; the
  switch is session-scoped and not persisted. StdIO and HTTP sessions switch
  independently, and a secondary proxy cannot expand beyond its primary's
  catalog.

### Fixed

- **Planning actions are plain strings again.** `planning_advance` and
  `planning_decide` now publish their `action` fields as JSON Schema string
  enums instead of bare objects, matching the server's deserializer. Plain
  strings and the legacy `{"variant": null}` object form are both accepted;
  other shapes return an actionable error. The same schema/server mismatch
  affected tagged unions on `commit_scene` (`authorship`, `expected_phase`),
  `commit_scene_changes` (`continuity_gate`), `commit_character_state`
  (`anchor`), and `import_apply_review_decisions` (`correction`); those schemas
  now publish their union structure. A catalog-wide contract test now checks
  every tool's published input schema against its typed deserializer and
  validates canonical schemars examples against the published schema, so
  sanitizer transforms cannot silently erase a contract again.
- Semantic continuity precision round (the lane still ships **off by
  default**): quantity contradictions require balance/stock evidence,
  dead-character state is selected only from strictly prior scenes,
  memory/flashback/vision framing no longer reports a dead character as acting,
  and secret disclosures require a distinctive value detail or a public
  audience. Deterministic dedupe now covers same-subject/same-scene findings
  beyond world rules. Shadow and on runs are measured from their own passes;
  on-mode repairs are <=4.17% of calls on the precision sets, control warnings
  do not rise, warning precision is 1.0 where warnings exist on those sets, and
  the judge-policy revision is bumped so pre-change cached reports cannot bypass
  the new gates.

### Upgrade notes

- No new database migrations since 0.4.0. Back up `.spindle/` before upgrading,
  as usual.

### Known limitations

- The semantic continuity lane remains **off by default** and experimental.
  The blind held-out-4 measurement adds +1 localized plant over deterministic
  (30/55 vs 29/55 corrected; 31/56 vs 30/56 as frozen), with semantic warning
  precision 0.50 (1 TP / 1 FP). Activation gates are not met, so
  `semantic_mode=on` is not recommended.

## 0.4.0 — 2026-11-11

### Added

- An experimental semantic continuity lane: core candidate/judgment/evidence
  contracts, deterministic candidate building, a compact judge transport with a
  fail-closed contradiction verifier and validated-report caching (V0058),
  host-report input, `semantic_mode=off|shadow|on` plumbing, and a public-set
  evaluator. It ships **off by default**; `shadow` observes without changing
  findings.
- `commit_scene` accepts `prose_path` as an alternative to inline `full_text`
  (exactly one), reading long prose from a workspace-relative UTF-8 file with
  path confinement, a 2 MB cap, line-ending normalization, optional
  `expected_sha256`, and a prose-source receipt. The primary resolves the file
  for HTTP and secondary clients.

### Changed

- R+1 profile and alias cleanup: unset now starts the generated `workflow-write`
  profile; the legacy `minimal`, `write`, `import`, `authoring`, and `legacy-all`
  profiles are removed and unknown/removed names fail closed. The release-R
  style preview twins (`preview_refresh_style_profile`,
  `preview_apply_style_profile`, `preview_style_revision_patch`) are removed in
  favor of `dry_run` on `refresh_style_profile` / `apply_style_profile` and
  `create_style_revision_patch`. Stored history, scene versions, save points,
  and record IDs remain readable.

### Upgrade notes

- Back up `.spindle/` before the first 0.4.0 start: migration **V0058** runs on
  the project database.
- Legacy tool profiles were removed and unset now means generated
  `workflow-write`. Set `SPINDLE_TOOL_PROFILE` explicitly if you relied on an
  unset or legacy profile name.
- For long scenes, write the prose to a workspace file and pass `prose_path` to
  `commit_scene` instead of inline `full_text`.

### Known limitations

- The semantic continuity lane ships **off by default** and is experimental.
  `semantic_mode=shadow` is available. Public-set results improve held-out-3
  from 26/56 to 29/56 localized with warning precision 1.00, but activation
  gates are not met: control false positives remain on baseline/held-out-2 and
  aggregate lift is limited. Do not use `semantic_mode=on` in production yet.
- Deterministic continuity recall is unchanged from 0.3.1.

## 0.3.1 — 2026-11-10

### Fixed

- Character-state selection now uses SQLite rowid as the explicit recency tie-break at equal story positions, with created-at/id fallbacks; the recency key is retained in internal canon snapshots, and the retcon validator, subject snapshot and export paths share the newest-state rule instead of depending on load order.
- `get_writer_state` now succeeds at its default budget and keeps the response
  within any requested budget, compacts required sections, and reports typed
  `omissions` with fetch hints.
- `find_entity` now ranks exact matches accurately for shared first names and
  aliases, returning every owner of an ambiguous name instead of mislabeling
  fuzzy hits as exact.
- `commit_character_state` accepts a `story_start` anchor for backstory deaths
  and other pre-opening character state, without requiring a scene.
- Project calendars support optional leap years using proleptic Gregorian rules
  or custom month/cadence rules (migration V0057). Existing calendars keep
  fixed-length years until configured otherwise.
- `research_query` now uses provider-neutral discovery language.
- `.spindle/spindle.db.migration.lock` is covered by the workspace
  `.spindle/.gitignore`, so it no longer appears as an untracked book file.

### Upgrade notes

- Back up `.spindle/` before the first 0.3.1 start: migration V0057 runs on the
  project database.

## 0.3.0 — 2026-11-10

Spindle 0.3.0 is the compatibility release for the consolidated tool surface.
It keeps the release-R aliases working while recommending the generated
workflow profiles for new sessions.

### Highlights for authors

**Plan before drafting**

- Durable new-book and planning sessions: `planning_begin`, `planning_advance`,
  `planning_status`, and `planning_decide` stage proposals level by level and
  keep human decisions explicit. The `new-book` skill takes a premise through
  intake, staged planning, review, and a ready-to-draft handoff.
- Plan review and release smoke checks catch malformed or incomplete plans
  before they reach the drafting loop.

**Canon, memory, and retrieval**

- Shared claim-vs-canon validation lanes give miners, importers, and the
  authoring loop one place to distinguish explicit canon, inferred claims, and
  contradictions.
- Layered derived memory keeps compact summaries and retrieval context fresh as
  scenes, chapters, and books grow.
- Configurable embedding providers and dimensions are tracked by index
  generation, so changing the embedding route no longer silently mixes vector
  spaces.
- Continuity detectors resolve full names, forenames, surnames, cast aliases and
  a declared single POV's first-person narration without guessing across
  ambiguous names. Knowledge paraphrase matching uses two strong stemmed
  content tokens; dead-character checks read character state and canonical
  `life_status` and recognize irregular/third-person finite verbs; attribute
  checks cover numeric heights, sight/vision, handedness, colour, scars and
  build.
- Secret-leak detection also catches paraphrased disclosures through reported
  speech, written notices/broadsheets/letters read aloud, and role or collective
  actors, while holder/in-circle and framed sentences stay silent.

**Drafting packets, critic, and revision (off by default)**

- `prepare_scene` assembles a bounded scene packet with cast, setting, canon,
  continuity, promises, and voice guidance; `commit_scene` persists the prose
  and approved continuity effects atomically.
- The authoring supervisor can run deterministic verify/revise, dual-persona
  critic review, canon mining, checkpoint review, and replanning as opt-in run
  phases. These advanced loop features remain off by default pending
  measurement.
- Optional line edits and bounded chapter passes help revise without turning a
  single instruction into an unbounded rewrite.
- The scene critic now resolves quote-first anchors, requires a typed
  `no_change` reason when it proposes nothing, applies action/quiet-interior/
  dialogue/reveal focus, and uses a compact single repair prompt. Offline replay
  on the 90 G drafts improved invalid output 22.2% → 4.4%, actionable directives
  15.6% → 95.6%, and internal-judge promoted revisions 10.0% → 45.6%; a full G
  remains the activation gate.

**Tool surface**

- Generated workflow and specialist profiles are the recommended startup
  surface: `workflow-write`, `workflow-authoring`, `workflow-import`, and
  focused profiles for revision, worldbuilding, planning, new-book, research,
  style, release, and setup.
- `prepare_scene` and `commit_scene` provide the ordinary scene read/write
  boundary, with dry runs, stale-token checks, idempotency, and run-phase
  handoff.
- Typed `update_character`, `update_location`, `update_conflict`, and
  `update_character_arc` replace the supported fields of the open
  `update_entity` path. Atomic batch creates cover terms, motifs, narrative
  promises, and character voice profiles.
- `list_changes` gives one paginated history vocabulary; `undo_change` resolves
  an operation to its explicit revision scope and refuses automatic mutation
  where compensation is not atomic.
- Operator commands (`configure-agents`, `test-agent`, `rebuild-search-index`,
  `backfill-scene-source-offsets`, `operator-delete-scene`) are CLI-only and
  denied on model transports. The authoring harness keeps an internal,
  token-scoped `test_agent` dispatch.

**Security and reliability**

- Operator tools are hidden from every model profile and denied by name over
  stdio and HTTP.
- A panic in a tool handler returns a named error instead of hanging the
  caller; project serialization and internal-token dispatch are shared across
  transports.
- Scene addressing is unambiguous with `scene_id`, stale scene versions and
  preparation tokens are rejected, and commit paths avoid partial writes.
- Import, pull/push, review dispatch, and checkpoint paths no longer lose or
  duplicate work on retries, timeouts, or transport failures.

### Compatibility

0.3.0 is a **compatibility release**. The following release-R names still work
but are **deprecated** and will be removed in the next release:

- Legacy profiles: `authoring`, `write`, `minimal`, `import`, and `legacy-all`.
- Style preview twins: `preview_refresh_style_profile`,
  `preview_apply_style_profile`, and `preview_style_revision_patch`.
- Batch aliases: `batch_create_terms`, `batch_create_motifs`,
  `batch_create_narrative_promises`, and
  `batch_set_character_voice_profiles`.
- The generic `update_entity` path for entity types that now have typed
  updates; `save_scene_draft`, `revise_scene`, `authoring_save_scene_draft`,
  and `commit_scene_changes` retain their existing save-only and branch
  semantics.

Use the generated profiles and canonical tools for new work. See
[`docs/tool-profiles.md`](docs/tool-profiles.md) and the
[release-R migration guide](docs/tool-migration-r.md) for the mapping and
removal plan.

### Known limitations

- Held-out-3 recall was measured blind at 23/56 localized after the detector
  work (16/56 before it). After the false-alarm round it is 26/56 localized
  (46.43%) and 28/56 type-present (50%). Weakest areas remain secret-leak
  paraphrase (0/8) and timeline (3/8, up from 1/8 before the round).
- Held-out-3 is no longer blind for false alarms: the false-alarm fixes used
  its findings, so its post-round false-alarm numbers are regression evidence,
  not a blind-generalization claim.
- Held-out-3 control warnings fell from 17 to 0; the prior canonical-drift 8,
  dead 2, retcon 2 and temporal 5 warnings are all gone. Eight info-level
  world-rule "no violation context" diagnostics remain; they are not
  warnings/errors.
- Release reference measurements: baseline 28/28 with 3 control findings (2
  `world_rule_compliance` from a control-metadata defect, 1 info homonym);
  held-out #72 54/56 with 0; held-out-2 55/56 localized, 56/56 type-present,
  0 control warnings.
- The critique-revise drafting loop fails both end-to-end quality gates
  (vs legacy baseline 0.41 [0.33, 0.49]; vs packet-only 0.50 [0.45, 0.55]) and
  remains off by default. Other advanced loop phases (verify/revise,
  dual-persona critic, canon mining, self-clearing checkpoints) also remain
  off pending measurement.
- Semantic contradiction detection is degraded; deterministic claim-vs-canon
  lanes are the reliable path. A model-assisted semantic continuity lane is
  planned for 0.4.
- There is no operator CLI for rebuilding the embedding index; index generation
  changes are tracked, but rebuild tooling is not shipped.
- Unified undo is checkpoint-only for scene continuity and multi-step style
  rollback. Scene prose restores use `restore_scene_version`; style effects use
  the explicit rollback tools.

### Added

- Continuity uses one shared cast-mention resolver: full names, forenames,
  surnames and explicit aliases resolve without guessing across ambiguous
  names, with conservative pronoun handling. Dead-character detection reads
  both character state and canonical `life_status`, frames out memories,
  letters, dreams, recordings, corpses and graves, and emits a stable
  `dead_character_acting` alias; attribute drift covers eye/hair/skin colour,
  handedness, height, scars and build with plural heads and conservative
  negations.
- Added the blind held-out-3 continuity corpus and `held-out-3` evaluation
  mode for independent recall and false-alarm measurement.
- Re-ran the 90-draft G quality gates after critic hardening: the loop scored
  0.41 [0.33, 0.49] against the legacy baseline and 0.50 [0.45, 0.55] against
  packet-only, failing both promotion gates, so it remains disabled (see Known
  limitations).

- Quantity affordability now parses English number words alongside digits and
  resolves the spender name through the shared character-mention resolver
  (canonical name, first name, alias, conservative sentence-initial pronoun,
  and first-person POV). A resolved spender whose purchases exceed their
  tracked balance produces a warning; ambiguous spenders remain advisory info.

- World-rule prose scanning derives required cost/effect nouns from rule
  descriptions and promotes explicit omissions ("without a/the <cost>",
  "without <cost>", "no <cost>") to warning-level `world_rule_semantic_drift`
  findings, including when the prose does not repeat the rule's scan label.
  Vague rule mentions remain advisory. Scanner semantics version is now v5 so
  cached findings are recomputed.

- Held-out-2 continuity scoring now follows the detector contract: attribute
  coverage accepts `canonical_fact_prose_drift`, world-rule expectations use
  `world_rule_semantic_drift`, and a broad finding localizes by planted scene
  only when it cites the plant's own subject. Warnings/errors still count;
  info-level advisories do not.

- Unified history and undo scopes: `list_changes` paginates scene-version,
  checkpoint, and style receipts with explicit undo support; `undo_change`
  resolves an operation id and refuses automatic mutation for scopes whose
  compensation is not atomic, directing callers to the explicit revision tool.

- Canonical atomic batch creates: `create_terms`, `create_motifs`,
  `create_narrative_promises`, and `set_character_voice_profiles` accept 1-50
  items, support `dry_run`, preflight every item, and commit all-or-nothing in
  one branch transaction. The four `batch_*` names remain one-release aliases.

- Typed entity updates: `update_character`, `update_location`, `update_conflict`,
  and `update_character_arc` replace the supported fields of the open
  `update_entity` path with unknown-field-rejecting DTOs. `update_entity`
  remains a compatibility path for entity types without a typed replacement.

- Operator migration (release R): `configure-agents`, `test-agent`,
  `rebuild-search-index`, `backfill-scene-source-offsets`, and
  `operator-delete-scene` are now `spindle-mcp` CLI commands; the existing
  `init-grok-skills` CLI covers skill installation. Their old MCP names are
  hidden from every model profile (including unset and `legacy-all`) and denied
  by name on stdio/HTTP model transports. DB-mutating commands acquire the
  workspace primary lock, and `--data-dir`/`--config` override the matching
  environment variables. The authoring harness retains its internal
  `test_agent` dispatch through a server-issued token. Added
  `docs/tool-migration-r.md` with the CLI mapping, compatibility aliases,
  profile guidance, and the R+1 removal plan.
- `refresh_style_profile` and `apply_style_profile` accept `dry_run`. A dry run
  reports the proposed candidate profile or prose diff (narrator voice, style
  notes, world-rule action, metric deltas, and quality/apply-safety notes)
  without inserting a profile version, consuming or dismissing candidates,
  activating the profile, invalidating caches, or writing an application audit
  record. `preview_refresh_style_profile` and `preview_apply_style_profile`
  remain available through release R and delegate to the dry-run paths.
  `create_style_revision_patch` is the canonical non-mutating patch creation
  step; `preview_style_revision_patch` remains a release-R alias, and only
  `apply_style_revision_patch` writes prose.
- New `commit_scene` MCP tool is the single atomic write for ordinary scene
  work: it persists prose, the author-declared continuity package, beat
  annotations, an optional chapter-completion summary, and the writer cursor in
  one SQLite transaction. Deterministic validation gates run before the write,
  so blocking findings return no partial writes. `dry_run` previews the same
  effects without writing, stale preparation tokens and scene versions are
  rejected, idempotency keys make retries safe, and an authoring-run caller can
  advance the harness scene phase only after a live success. `save_scene_draft`,
  `revise_scene`, `authoring_save_scene_draft`, `commit_scene_changes`, and
  `annotate_scene_beats` keep their existing save-only and branch semantics.
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

- Temporal coherence now budgets prose time-of-day jumps against the recorded scene duration and compares explicit elapsed-time claims with recorded death clocks, so overnight scenes are not false alarms. Clean world-rule cost demonstrations no longer emit info-level semantic-drift noise.
- Continuity false alarms: passing death references ("since Maren drowned", "the late Bran", remembering/mourning) no longer count as post-mortem action, and canonical-fact drift warns only on positive contradictory evidence rather than a bare key mention.
- Test fixtures now drop their temporary SQLite directories instead of calling
  `TempDir::keep()`, preventing multi-gigabyte temp-dir accumulation across
  workspace test runs.
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

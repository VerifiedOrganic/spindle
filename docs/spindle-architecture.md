# Spindle architecture quick start

This guide gives you the fastest accurate path into the current Spindle code
architecture. Read this first if you are new to the repository, then jump to
deep plan docs only when you need historical context.

## What Spindle is

Spindle is a local-first MCP server for story planning, drafting, revision,
and continuity validation. The architecture centers on one rule: writing
tools must return actionable validation output without leaking storage or
transport concerns across crate boundaries.

## Core framing

Spindle treats continuity checks as a truth-oracle workflow:

1. Capture typed world and story facts in persistent records.
2. Build scene-scoped context from branch-aware data.
3. Run validator passes against scene text and metadata.
4. Surface findings directly in write-tool responses and consistency scans.
5. Expand deterministic validator coverage as those interfaces land.

This framing replaces older "best effort linting" assumptions.

## Six architecture pillars

The current architecture is easiest to reason about as six pillars.

1. Typed data over ad hoc blobs.
2. Branch-aware reads and writes as default behavior.
3. Validator framework rollout across shared context and finding contracts.
4. Thin transport layer over service-level business logic.
5. Explicit invalidation for continuity and consistency-related caches.
6. Documentation and skill prompts that match shipped interfaces.

## Workspace map

The workspace has five active crates with clear ownership boundaries.

- `crates/spindle-core`: shared models and validator contracts.
- `crates/spindle-adapters`: repository + services + model routing + guidance.
- `crates/spindle-skills`: embedded `skills/` asset packaging.
- `crates/spindle-mcp`: MCP tool/resource wiring and process entrypoint.
- `crates/spindle-harness`: operator automation over MCP for batch drafting,
  checkpoints, and resume workflows.

## Hexagonal Shape

Spindle uses a pragmatic hexagonal split. The dependency direction should be:

```text
spindle-mcp / spindle-harness -> spindle-adapters -> spindle-core
spindle-adapters -> spindle-skills
```

`spindle-core` owns public DTOs, domain value types, context-bundle contracts,
subject snapshots, and validator contracts. It must not depend on MCP,
SQLite, model-agent runtimes, filesystem layout, or embedded asset plumbing.

`spindle-adapters` owns outbound and application-service concerns: SQLite
persistence, import/export persistence, model routing, embeddings, guidance
lookup, and orchestration that combines repositories with pure formatting or
validation helpers.

`spindle-mcp` is an inbound adapter. It should parse MCP arguments, apply
session/defaulting and mutation-serialization policy, invoke service methods,
convert errors into MCP results, and expose resource/tool schemas. It should
not own story business rules, persistence query details, or public story DTOs.

`spindle-harness` is another inbound adapter, but as an operator/client over
the public MCP surface. It should not reach into repositories or SQLite.

`spindle-skills` is static embedded asset packaging only.

## Boundary Audit

Current audit findings:

- No reverse dependency from `spindle-core` to adapters, MCP, SQLite, or
  runtime model code was found.
- `spindle-mcp` previously owned several public tool/envelope DTOs
  (`set_active_project`, `init_grok_skills`, `get_writer_state`, and
  `get_scene_context`). These contracts now live in `spindle-core`.
- `spindle-mcp` previously assembled writer-state and scene-context envelopes,
  including markdown rendering and standards insertion. That presentation
  assembly now lives behind `SqliteSpindleService`; MCP only invokes the
  service and serializes the result.
- MCP still owns session defaulting, tool schema sanitization, lock scoping,
  and Grok skill-file installation. Those are transport/client-adapter
  concerns and should stay out of core unless they become reusable public
  contracts. Input and output tool schemas are scrubbed of schemars'
  non-standard numeric `format` annotations (`uint`, `int32`, `float`, …) at
  the generation seam so strict JSON-Schema clients do not warn per field.

The service layer currently lives under:

- `crates/spindle-adapters/src/sqlite/service.rs`
- `crates/spindle-adapters/src/sqlite/project_resources.rs`
- `crates/spindle-adapters/src/sqlite/source_bridge.rs`
- `crates/spindle-adapters/src/format.rs`

`service.rs` remains the main application-service entry point. Supporting
modules keep cohesive adapter concerns out of that file: `project_resources.rs`
owns `read_project_resource` pagination envelopes and record-to-JSON resource
projection helpers; `source_bridge.rs` owns external source lookups; and
`format.rs` owns pure presentation formatting shared by service methods.

Architecture-sensitive boundaries are enforced by
`crates/spindle-core/tests/architecture_boundaries.rs`. The guard test checks
that:

- `spindle-core` does not import adapters, MCP, SQLite, or skill packaging.
- `spindle-skills` stays static asset packaging only.
- `spindle-harness` does not import SQLite service/repository internals.
- `spindle-mcp` does not define public `*Input`, `*Output`, or `*Envelope`
  DTO structs.

Run the focused guard with:

```bash
cargo test -p spindle-core --test architecture_boundaries
```

It is also covered by the normal workspace test command.

## Validator architecture (in progress)

Continuity behavior is centered in service-layer checks and consistency
tooling. Validator-oriented plumbing exists in adapter services, but do not
assume the full target validator DTO and output surface is available on every
write tool response yet.

SQLite Phase-4 validator cache rows live in `validator_finding`. Cache hits are
keyed by branch, scene, `validator_id`, `scene_text_hash`, and a validator
context hash derived from the relevant branch-scoped facts, rules, characters,
timeline events, temporal interventions, and style context. Service write paths
that alter canon, voice, style, or timeline state must resolve the matching
validator cache id, and read paths must use explicit branch-aware repository
queries. The continuity health resource
`bible://projects/{project_id}/continuity/health` exposes open findings,
resolved cache rows, branch lineage, orphaned temporal interventions, and
duplicate active canonical-fact keys for operational checks.

## Scene write path

The normal write path keeps generation, persistence, and validation separate
but connected:

1. Client calls scene-writing tools (`save_scene_draft`, `revise_scene`,
   `commit_scene_changes`).
2. Services persist branch-local scene updates through repository methods.
3. Services derive warnings, diffs, and consistency-related results.
4. Tool responses return scene IDs and currently shipped response fields from
   `crates/spindle-core/src/models.rs`.

`check_consistency` remains the broad audit surface for branch-scoped
consistency review.

## The authoring-run editorial loop

The authoring supervisor (`spindle-harness`, driven through the `authoring_*`
MCP tools) runs a bounded per-scene editorial loop, one `authoring_execute_next`
call per step. The full spine is **draft → verify → revise → commit → mine →
annotate beats → chapter summary → replan → checkpoint**. The state machine
lives in `spindle-harness/src/plan.rs` (`NextAction` / `determine_next_action`);
it only grows — existing phase semantics
(`pending → draft_saved → changes_committed → beats_annotated`) are unchanged and
pre-upgrade runs resume unaffected. The current run flow, tool-by-tool, is
documented in `docs/authoring-supervisor.md`; this section states the shipped
shape.

The loop's default posture is **hybrid**: for General/Teen/Mature scenes
`authoring_execute_next` returns a host-draft instruction and the active
assistant drafts in-chat and saves via `authoring_save_scene_draft` (with its
required continuity package); `mode: "agent"` opts into full route offload.
Every automated pass has a host fallback, so a project that configures no routes
behaves as pure hybrid.

Four of the loop's steps beyond the classic draft/commit/annotate/summary spine
are **opt-in per run**, each defaulting to off so a run that opts into nothing is
byte-identical to the pre-evolution flow:

| Run knob (`authoring_start_run`) | Default | Enabled behavior |
| --- | --- | --- |
| `mining_policy` | `disabled` | `propose_all` inserts a `mine canon` step between commit and annotate; each committed scene is mined into staged canon deltas (§ staging below). |
| `max_revise_attempts` | `0` | `1` or `2` inserts a deterministic `verify scene` step after each draft; a finding ≥ `warning` sends the scene back to the same draft route (or the host, in hybrid) as a bounded `revise` before commit. |
| `checkpoint_policy` | `manual` | `auto_advisory` / `auto_strict` let the harness self-clear a checkpoint in-process on a severity threshold instead of blocking on `await_checkpoint_review`. |
| `replan_policy` | `disabled` | `propose_all` inserts a `replan future plans` step after each chapter summary; staged plan amendments chase the outline to realized reality (§ staging below). |

Verify is deterministic: it runs the scene-scoped subset
`spindle_core::models::SCENE_VERIFY_CHECKS` (knowledge_timing, chronology,
temporal_coherence Tier 1, quantity/currency/affordability, tone,
content-boundary, secret_leak, canonical-fact consistency, and the prose-drift /
world-rule / voice / style Phase-4 validators — no model calls; deep/model tiers
stay checkpoint-scoped for cost). The loop converges or stops: the same finding
set is never revised twice (an unchanged re-verify parks the scene), and parked
findings inherit to the checkpoint. Every pass records an **honest** per-scene /
per-chapter outcome in `authoring_status` (`mine_status`, `verify_status`,
`replan_status`, `auto_outcome`, …): a skip or error never reads as a clean pass,
and mining/verify/replan never block the run.

The auto checkpoint policies run the deep `check_consistency`, record the audit,
run sampled dual-persona reviews via the `review` route, and (under an auto
policy only) a cumulative reader simulation, then approve iff the deep-consistency
severity clears the policy threshold (`auto_advisory`: nothing ≥ `warning`;
`auto_strict`: zero findings). A blocked auto-checkpoint stays `pending_review`,
so the manual 4-step escape hatch still clears it. Preconditions are enforced at
`authoring_prepare_run`: an auto policy requires the `review` route resolve
rating-cleared for every rating in the run's range, and `mining_policy` requires
the mine-or-review ladder cover every planned rating — an offload gap fails at
prepare, not mid-run.

## The rating-clearance chokepoint (explicit-content offload)

Every model pass that carries scene prose is dispatched through **one** gate,
`resolve_cleared_route` in `crates/spindle-adapters/src/ai.rs`, which wraps route
resolution and additionally verifies the resolved agent's declared `ratings`
cover the scene's content rating (normalized compare) before any prose leaves the
process. No call site resolves a prose-bearing route directly. The prose-bearing
set is the `PROSE_BEARING_ROUTES` constant — `{draft, mine, line_edit,
reader_sim, review}`. When a route cannot serve a rating the pass **skips
honestly** (a `pass_skipped` journal event + an honest status/finding), falls back
to the host, or degrades per the pass's ladder — it is never silently downgraded
to an uncleared model, and Spindle never rewrites explicit prose to make it
routable.

Two documented refinements the code carries beyond the naïve "gate by route
name":

- **Split appendix.** The explicit gate is uniform across all prose-bearing
  routes, but the *system-prompt appendix* is not: `draft` receives
  `EXPLICIT_DRAFT_SYSTEM_APPENDIX` (which instructs on-page adult prose), while
  every other prose-bearing route (miner, auditor, reviewer) receives
  `EXPLICIT_ANALYSIS_SYSTEM_APPENDIX` instead — the invariant is uniform *gating*,
  not a uniform prompt (instructing a miner to write adult prose would be wrong).
- **Two exemptions.** The manuscript-import routes (`IMPORT_ROUTES` =
  `{import_extract, import_synthesize}`) are deliberately **not** in
  `PROSE_BEARING_ROUTES`: content ratings do not exist until analysis runs, and
  import is a direct operator action on the operator's own manuscript, so the
  guard is an advisory nudge (`import_chair_clearance_advisories`), not gating.
  The **replan** differ is non-prose-bearing (summaries + metadata only), so no
  rating clearance applies to it either. The two pre-existing model-backed deep
  tiers that predate the chokepoint (intra-scene `deep_temporal_coherence_issues`,
  semantic `deep_world_rule_compliance_issues`) now stamp each scene's rating on
  their `review` dispatch and honest-skip on `RatingNotCovered`, closing the
  former `rating: None` bypass.

The normative specification is `docs/spindle-evolution-design.md` §4; a recording
fake-router contract test pins that no request carrying an explicit scene's prose
ever reaches an uncleared agent.

## Staging & ratification (canon deltas + plan amendments)

Machine-derived canon never reaches the bible directly — it lands in a staging
table and is applied only through an explicit operator decision. Two staging
classes share one lifecycle:

- **Canon deltas** (`canon_delta`, migration V0024; ADR 0001). Each committed
  scene can be mined (`mine_scene_canon`, or the run's `mine canon` step) into
  proposed, per-class, evidence-quoted deltas across fourteen classes. Each class
  maps to **exactly one existing write tool** on apply — mining opens no new write
  path. The operator lists the ratify queue (`list_canon_deltas`) and decides a
  batch (`decide_canon_deltas`: apply/reject, optional edit-before-apply, all-or-
  nothing pre-flight, apply dispatched per class to `register_canonical_fact` /
  `update_promise_status` / `update_relationship` / `commit_character_state` /
  `record_knowledge` / `annotate_scene_beats` / `commit_quantity_state` /
  `update_entity` / the `create_*` tools). Re-mining supersedes a scene's prior
  `staged` rows; decided rows are history.
- **Plan amendments** (`plan_amendment`, migration V0029; ADR 0003). After each
  chapter summary the replan differ compares realized reality against the
  not-yet-drafted chapters' plans and stages amendment proposals across eight
  classes, applied through the existing plan write paths
  (`plan_chapter` / `create_narrative_promise`). The lifecycle mirrors
  `canon_delta` exactly (`list_plan_amendments` / `decide_plan_amendments`); an
  **immutability guard** rejects any amendment whose target chapter already has a
  drafted scene at apply time (the outline chases the story, never the reverse),
  and applying snapshots the prior plan into `prior_state` and bumps
  `chapter_plan.plan_revision` so the outline keeps recoverable history.

Default policy for both is **propose-only** — nothing auto-applies. Canon deltas
support an opt-in per-class `auto_accept`, but `entity_candidate` and any delta
carrying `secret_of_fact_id` are excluded unconditionally (circle expansion is
always a human decision). Plan amendments have **no** auto-accept at all, not even
as policy. Both class vocabularies are **one-way doors** fixed by their ADRs
(0001, 0003) — renames/reshapes strand staged rows and break decided-audit replay.
The `canon-steward` skill teaches both ratification workflows.

## Circle-of-trust secrets

A canonical fact can be declared **secret** (`register_canonical_fact { secrecy:
{ holder_ids, concealment_note? } }`, migration V0023): `canonical_fact.secret`
plus one `knowledge_fact` row per holder linked back via
`knowledge_fact.secret_of_fact_id`. The circle of trust is thereafter **derived,
never duplicated** — `circle(fact)` is the set of characters with a linked
knowledge row — and is cursor-aware via each row's `learned_at`. Full design and
non-goals: `docs/secret-knowledge-gating-design.md`.

Three enforcement surfaces:

- **Context gate.** A single `SecretVisibility` resolver (`format.rs`, called by
  the assemblers in `service.rs`) enforces one rule per scene: if no circle member
  is present the fact is **withheld** from every context carrier (hard
  constraints, subject snapshots, knowledge briefing, semantic recall, digest
  open-threads); if a circle member is present it renders in a non-truncatable
  `[SECRETS IN PLAY]` hard-constraint block naming who knows, who is present and
  unaware, and (POV-only variant) that narration may carry private awareness while
  dialogue and others' behavior must not. A reveal is `record_knowledge` (or a
  `knowledge_learned` save-package entry / mined delta) with `secret_of_fact_id`
  set; it expands the circle from that placement forward and never leaks backward.
- **`secret_leak` audit** — the audience-direction complement to
  `knowledge_timing`. A deterministic dialogue-attribution tier always runs; a
  model-backed behavioral tier (`deep_secret_behavioral_leak_issues`) runs under
  `deep_check`, rating-gated through the chokepoint with honest-skip.
- **Reader-visibility rule for reader artifacts** (below) is deliberately stricter
  than the character-facing context gate.

## Run journal, SSE & operator console

Each authoring run appends an **append-only event journal** (`authoring_run_event`,
migration V0027; ADR 0002): one row per observable transition, written *after* the
state change commits (a journal-write error logs at `warn` and never fails the run
step). Payloads carry **ids, artifact paths, counts, and enums only — never prose,
fact text, evidence, or model output** — so the stream is safe to leave open on a
shared screen. The kind vocabulary and payload shapes are a **one-way door** fixed
by ADR 0002; consumers must ignore unknown kinds/keys. `authoring_status` (the run
tables) remains the source of truth; the journal is the timeline view.

The journal streams over the existing HTTP surface at
`GET /events?topic=run:<authoring_run_id>` (SSE `id` = `seq`, `event` = kind,
`data` = payload JSON), replayed from `Last-Event-ID`+1 then followed live;
`/events` with no topic is the unchanged model-routes snapshot.

The **operator console** is served at `/console` when HTTP mode is on
(`SPINDLE_HTTP_ADDR`): a single embedded HTML page with no build step or external
assets. It reads run status, the compiled manuscript, and staged canon/plan
queues through localhost-only `GET /console/api/*` service reads, and follows a
run's journal live over the SSE topic above. Series, reader memory, editorial
decisions, and local release actions use an MCP session at `/mcp`; ratification
still goes through the guarded `decide_*` tools. The GET endpoints never mutate.
Cross-site browser requests are rejected. When an Origin header is present, it
must match the loopback authority; native MCP clients without one remain supported.

## Craft evaluation layer

Two deep, checkpoint-scoped craft passes ride the same deep-check / honest-skip
pattern as `promise_payoff_detection`:

- **`scene_purpose_fulfillment`** (`check_consistency` deep tier) asks whether a
  drafted scene accomplishes its chapter-plan `purpose` field; a `fulfilled:false`
  verdict is one advisory `info` finding (purpose drift is often the story
  improving, not breaking), rating-gated per scene with honest-skip.
- **Cumulative reader simulation** runs under an auto checkpoint policy: a persona
  derived from the reader contract reads the checkpoint range's chapters in order
  with rolling memory (`reader-sim-notes.json` per run) and reports per-chapter
  engagement. It is **enrichment, not a gate** — concerns never fold into the
  approve/block verdict — and is fully rating-gated via the `reader_sim` route
  (falling back to `review`) with honest-skip.

**Style learning from operator edits** (migration V0031, opt-in per project via
`project.style_learning`) captures the before/after pair when an operator re-saves
an agent-drafted scene with changed prose as a *style-edit candidate* feeding the
existing `preview_refresh_style_profile` → `refresh_style_profile` review flow —
no new tools, nothing enters a profile without an explicit refresh, and
explicit-rated candidates are withheld unless the `style_analyze` route is
explicit-cleared. Provenance across the domain is modeled by the
`spindle_core::provenance::Provenance` vocabulary (scene / chapter / book / file /
asserted-by-author / imported / derived), which records where a value came from.

## Canonical fact model in practice

Canonical facts are active architecture work. Treat the canonical-fact
validator flow as transitional rather than final.

- Any schema/tool change that affects facts must update docs and tests in the
  same change.

## Reader-facing artifacts (spoiler-bounded)

`export_recap` and `export_series_bible` (evolution §3.8) are pure read models
over existing branch data — no model calls. Both assemble reader-facing Markdown
bounded at a story-index cursor (end of `through_chapter` / `through`, whole
book/project when absent) and both take an opt-in `write_to_workspace` that
reuses the same workspace `artifacts/` write as `compile_manuscript`
(canonicalize + `starts_with` confinement).

A single resolver (`reader_withheld_secret_values` in
`crates/spindle-adapters/src/sqlite/service.rs`) enforces the reader-secret rule
for both tools, and is deliberately stricter than the scene-context circle gate:
a `secret` canonical fact is only surfaced once a linked `knowledge_fact` reveal
row is `reader_visible` with a dated `learned_at` at or before the cursor. The
standing circle-of-trust holder row a secret declaration writes (learned_at
NULL) is dramatic irony, not a reader reveal, so it does not lift the veil.
Operator-facing lookup/recap flows live in the `bible-librarian` skill.

The read-only operator console v1 (evolution §3.7) is served at `/console` when
HTTP mode is on (`SPINDLE_HTTP_ADDR`): a single embedded HTML page (no build
step, no external requests) that reads run status, the compiled manuscript, and
the staged canon-delta / plan-amendment ratify queues through localhost-only
`GET /console/api/*` service reads, and follows a run's event journal live over
the existing `/events?topic=run:<id>` SSE. It never mutates (I5); ratification
stays in the `decide_*` tools. Actions and the thread board are v2.

## Quantity & economy continuity

Money, named prices, and progression systems (LitRPG/cultivation) are tracked as
typed canon, not free-text lore. Economies and their *price facts* (numeric
canonical facts with a unit) surface in scene context; a per-project **quantity
scheme** (`project_quantity_scheme`, migration V0020) plus append-only,
position-stamped **quantity state** (`quantity_state`) track wealth/progression as
ordered *bands* (band-primary; amount optional). Enforcement rides the existing
rails — the `quantity_drift`, `currency_consistency`, and `affordability` arms of
`check_consistency`, a `[WEALTH/STATE]` scene-context hard constraint, a WarnOnly
band-jump advisory on `commit_quantity_state`, and branch-merge carrying. Full
design and roadmap: `docs/continuity-quantity-design.md`.

## Temporal coherence (in-scene)

In-world *timing* is guarded on two axes. The **between-scene** axis is the
`chronology` arm of `check_consistency` (a later scene rewinding the story
clock). The **within-scene** axis is the `temporal_coherence` arm: a pure,
deterministic prose scan (`spindle_core::temporal::scan_temporal_coherence`,
infrastructure-free domain logic) that reads one scene's drafted prose and flags
an unsignaled time-of-day jump (*teleporting time*), prose that contradicts its
own established time of day (*drifting time*), or a scene whose clock declares a
multi-day `duration_days` but whose prose renders the span as one unbroken block
(*unrendered declared span*). It is prose-only and runs without a calendar; its
time lexicon covers parts of day, meal names, canonical hours, and meridian clock
times. High-precision/low-recall (conservative band-jump thresholds, word-boundary
matching, greeting/recollection guards, suppression on `temporal_mode` and coarse
`precision`), so findings are advisory `warning`s.

The same deterministic scan (via the shared `scan_temporal_findings` adapter
helper, mapping each hit to a `ConsistencyIssue`) surfaces at **three enforcement
points**, all advisory: the immediate `temporal_findings` field on
`save_scene_draft` / `revise_scene`; a `temporal_findings` field on
`commit_scene_changes` that is held separate from `blocking_continuity_findings`
and emitted at `warning` severity, so it can never block a commit under any
`continuity_gate` (including `BlockErrors`); and the branch-wide
`check_consistency` arm.

That deterministic scan is **Tier 1**. `check_consistency` with `deep_check:
true` adds **Tier 2** — `deep_temporal_coherence_issues`, a model-router-driven
semantic pass (one `review`-route call per scene) that recovers the idiomatic /
implied time jumps the fixed lexicon cannot, mirroring the existing
`deep_world_rule_compliance_issues`. It parses structured findings (tolerant of
code fences) and degrades to no extra findings when no review model is
configured, so Tier 1 always stands alone. Both tiers emit the same
`temporal_coherence` check_type at `warning` severity. The prevention half is the `[IN-WORLD TIME]` hard
constraint built by `SqliteSpindleService::temporal_anchor_constraint` and
surfaced in both `get_scene_context` and `get_chapter_briefing`: it feeds the
previous scene's end clock **and location** forward as the expected start, so the
drafting model anchors *where* and *when* a scene takes place — the temporal twin
of the spatial "White Room" grounding rule. A scene's location is persisted via
`scene.location_id` (migration V0021) from the `location_id` on
`SaveSceneDraftInput`. The deferred-NL-parsing scope note that motivated this
layer is in `docs/continuity-timing-design.md`.

## Voice drift and retcon checks

Voice-drift (`voice_drift`) and retcon/reachability (`retcon_reachability`) are
shipped Phase-4 validators (`crates/spindle-adapters/src/sqlite/validators.rs`),
cached per `scene_text_hash` + context hash like the other Phase-4 validators and
run by default in `check_consistency`. `voice_drift` is also in the scene-scoped
`SCENE_VERIFY_CHECKS` subset the in-run verify step uses. The `continuity-editor`
skill documents both alongside the rest of the check catalog.

## Migration ledger

SQLite migrations are the source of truth for the schema
(`crates/spindle-adapters/migrations/`, applied in order; every column added by
the evolution work is nullable or defaulted so pre-upgrade databases and runs
resume unchanged). The continuity and editorial-loop migrations:

| Migration | Adds |
| --- | --- |
| V0020 | `project_quantity_scheme` + append-only `quantity_state` (money/progression bands) |
| V0021 | `scene.location_id` (spatial + temporal grounding) |
| V0022 | narrative-convergence audit fields (plot-line connected ids, conflict `escalation_demonstrated`, pacing-curve `intensity_points`) |
| V0023 | circle-of-trust secrets (`canonical_fact.secret` / `concealment_note`, `knowledge_fact.secret_of_fact_id`) |
| V0024 | `canon_delta` staging queue (ADR 0001) |
| V0025 | authoring-run `mining_policy` + per-scene `mine_status`/`mine_detail` |
| V0026 | authoring-run `max_revise_attempts` + per-scene `verify_status`/`revise_attempts` |
| V0027 | `authoring_run_event` append-only journal (ADR 0002) |
| V0028 | authoring-run `checkpoint_policy` + per-checkpoint auto outcome |
| V0029 | `plan_amendment` staging queue + `chapter_plan.plan_revision` (ADR 0003) |
| V0030 | authoring-run `replan_policy` + per-chapter `replan_status`/`replan_detail` |
| V0031 | style-learning-from-edits (`project.style_learning` + edit candidates) |

## Where to make changes

Use this map when you need to patch behavior quickly.

- Add or modify tool DTOs: `crates/spindle-core/src/models.rs`.
- Add or modify continuity and validator-facing DTOs:
  `crates/spindle-core/src/models.rs`.
- Add or modify MCP session/defaulting/schema behavior:
  `crates/spindle-mcp/src/tools.rs`.
- Change repository persistence/query behavior:
  `crates/spindle-adapters/src/sqlite/repository.rs`.
- Change business logic or tool orchestration:
  `crates/spindle-adapters/src/sqlite/service.rs`.
- Change MCP tool/resource exposure:
  `crates/spindle-mcp/src/tools.rs` and
  `crates/spindle-mcp/src/resources.rs`.

## Contributor guardrails

When you change architecture-sensitive code, keep these guardrails.

1. Do not move transport logic into `spindle-core`.
2. Do not bypass service-layer invariants from MCP handlers.
3. Put public tool/resource DTOs in `spindle-core`, not in `spindle-mcp`.
4. Keep validator findings structured and cache-aware.
5. Update docs and skill prompts when public behavior changes.
6. Run `cargo test -p spindle-core --test architecture_boundaries` after
   crate-boundary or DTO-placement changes.
7. Prefer branch-aware queries and avoid hidden project-wide scans unless
   explicitly required.

## First-hour onboarding checklist

Run this sequence when you start a new implementation task.

1. Read `README.md` for runtime and test commands.
2. Read this file end to end.
3. Locate affected DTOs in `spindle-core` and service paths in
   `spindle-adapters`.
4. Confirm any tool-surface impact in `spindle-mcp`.
5. Run targeted tests before and after edits.

## Related docs

- `docs/spindle-implementation-brief.md`: broader implementation snapshot.
- `docs/authoring-supervisor.md`: the current authoring-run flow, tool-by-tool
  (policies, staging hand-offs, journal, reader-sim).
- `docs/spindle-evolution-design.md`: the historical design record and rationale
  for the editorial loop (P0–P6, all landed) — this architecture doc supersedes
  it for current behavior.
- `docs/secret-knowledge-gating-design.md`: the circle-of-trust design (NOW +
  NEXT-5/6 landed; NEXT-7 reveal-aware briefing and LATER items still open).
- `docs/adr/0001-canon-delta-classes.md`, `0002-authoring-run-event-journal.md`,
  `0003-plan-amendment-classes.md`: the one-way-door contracts (delta/amendment
  class vocabularies, journal kinds/payloads) referenced above.
- `docs/continuity-quantity-design.md`, `docs/continuity-timing-design.md`: the
  quantity and temporal continuity spine the editorial loop builds on.

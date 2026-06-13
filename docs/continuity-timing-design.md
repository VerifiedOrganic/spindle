# Continuity & timing — design & roadmap

**Status:** design draft (no implementation yet). Reconciled against a deep code audit;
every code reference below was verified on branch `feat/automated`.
**Mandate:** keep the story world in sync and protect against drift across **hundreds of
chapters and multiple books**, with in-world **timing/chronology** coherence as the top
priority.

This document specifies the additive in-world **time layer**, the **chronology** and
**write-time** drift defenses, the **scale-retention** work, and the **promise/pacing timing**
fixes — sequenced NOW / NEXT / LATER with the verified bugs that make ordering non-negotiable.

> Design rule throughout: **additive and optional**. No existing project changes behavior; every
> new column is nullable; a book that never declares story-time behaves exactly as today.

---

## 0. Design goals — three story shapes, one model

| Shape | What it needs |
| --- | --- |
| **Long single series** (100s of chapters, multiple books, one continuous timeline) | a monotonic in-world clock that spans books; cross-book elapsed-time and character ages |
| **Heavy non-linear time** (flashbacks, time-skips, parallel POV) | story-time *decoupled* from manuscript order; declared flashback/parallel markers so legitimate jumps don't false-positive |
| **Invented calendar** (custom months/days/epochs, seasons, ages) | a project calendar (incl. non-24h days) that renders an abstract instant to/from display for any scheme |

Unifying abstraction: a **total-order in-world index** derived from a per-scene
`day_index` + `time_of_day`, governed by a per-project **calendar**. Every shape reduces to
"place this scene/event at a day+time, give it a duration, mark its relationship to the
manuscript flow."

---

## 1. Current state — verified

### 1.1 What already exists (genuine strengths — build on these)

- **Deterministic 5-validator Phase-4 suite**: `canonical_fact_prose_drift`,
  `world_rule_semantic_drift`, `voice_drift`, `retcon_reachability`, `style_compliance`
  (`service.rs:17674-17680`, `validators.rs:21-25`), with content-hash + context-hash caching
  and entity-edit invalidation.
- **Point-in-time canonical facts** via `fact_at_or_before` honoring
  `valid_from` / `valid_until` / `superseded_by` — real temporal-fact versioning.
- **Append-only, position-stamped `character_state`**.
- **Knowledge-timing primitives**: `knowledge_fact.learned_at`, `knows`, and a
  `future_knowledge` `OutOfBandsKnowledge` prose scan.
- **Non-truncatable hard-constraint prefix** with ~4000-token headroom in scene context.

This is solid drift coverage for **facts / rules / voice / style**. The gap is **time**, plus
the fact that detected drift isn't *enforced* at write time.

### 1.2 The time axis is structurally absent

There is **no in-world clock / calendar / duration / age / elapsed-time / season** primitive
anywhere — zero hits across both crates and all 16 migrations (the only `elapsed` is wall-clock
latency at `style_service.rs:419`). Every temporal comparison collapses to:

```
story_index = book*10_000 + chapter*100 + scene_order      (format.rs:928)
```

which (a) conflates manuscript order with story time (flashbacks/time-travel indistinguishable),
(b) can't answer "how much time elapsed by ch 120 / how old is she now / has the wound healed",
and (c) **collides** (see 1.3).

### 1.3 Verified bugs that bite at scale (must fix first)

1. **`story_index` collision / transposition.** No bounds on `chapter_number` or `scene_order`
   (`create_chapter` accepts unbounded n at `service.rs:1150`). Verified:
   `story_index(1,100,0) = 20000 = story_index(2,0,0)`. Past 99 chapters (or 99 scenes/chapter),
   every one of ~28 positional checks (promise aging, timeline ordering, future-knowledge expiry,
   retcon reachability, knowledge gating, recency windows) silently mis-orders across the book
   boundary — invisible, growing exactly as the work scales. The doc comment claiming collisions
   are impossible is false (`format.rs:1424-1426`).
2. **~100× promise-aging unit bug.** `chapters_since_plant = end_scope_index −
   story_index_from_placement(planted_at)` with **no `/100`** (`service.rs:6703-6707`), while
   `story_index` packs chapter at `*100`. A promise planted b1/ch1 checked at b1/ch5 computes
   ~400, not 4 → `planted >= 3` trips after 3 *scene-steps*; any cross-chapter delta is ≥100. At
   200 chapters **every** open promise is a permanent warning (alert fatigue masks the real ones).
   The correct logic already exists in the parallel path `format.rs:1828-1846`, so the two
   surfaces contradict each other for the same promise.
3. **Drift detected ≠ drift blocked.** `commit_scene_changes` blocks **only** on "Likely"
   world-rule regex hits (`service.rs:8835-8859`); `register_canonical_fact` is a blind INSERT
   (`service.rs:2849`); and **both** automated paths hard-code `accept_world_rule_risks=true`
   (`execution.rs:288`, `tools.rs:3115`). In unattended runs nothing the validators detect can
   block, so canon contradictions / premature-knowledge / chronology errors bake in permanently
   between manual `check_consistency` runs.
4. **Gate-2 is not scope-aware.** `detect_fact_contradictions` (`service.rs:7498-7549`) flags
   *every* evolving fact with >1 value as an error; `scope` ("evolving") is written
   (`service.rs:2879`) but read nowhere, and `valid_from`/`valid_until` are written but never
   consulted in grouping. Any write-time gate built on it would spuriously block legitimate
   timeline evolution. **This must be fixed before the gate goes live (non-negotiable ordering).**
5. **`future_knowledge_briefing` leaks to the drafting model.** It maps **all**
   `raw_future_knowledge` with no cursor filter (`service.rs:6017-6024`), and `knowledge_briefing`
   also appends future knowledge un-gated (`service.rs:6079+`) — right next to the
   `knowledge_fact` path that *is* gated by `story_index <= cursor` (`service.rs:6072-6076`). The
   guardrail is primed to *cause* premature reveals.
6. **Retention is recency-only.** ≤5 chapter summaries + a label-only `BookSummary`
   (`models.rs:969-974`); `semantic_references` is a hard-coded `Vec::new()` (`service.rs:6114`);
   nothing surfaces book 1–2 events when drafting book 3.

---

## 2. Workstreams at a glance

| Tier | Item | Effort | Migration |
| --- | --- | --- | --- |
| **NOW** | 2.1 Bound + widen `story_index` (radix) + write-time guards | M | — |
| **NOW** | 2.2 Promise timing: fix unit bug, honor `planned_payoff`, shared verdict | S | — |
| **NOW** | 2.3 Gate the `future_knowledge` briefing leak | S | — |
| **NOW** | 2.4 Write-time continuity gate (after Gate-2 scope fix) + wire overrides | M | — |
| **NOW** | 2.5 Story-time data model (`StoryClock` + `project_calendar`) | L | **V0017** |
| **NEXT** | 3.1 `ChronologyDrift` validator (6th Phase-4) | L | — |
| **NEXT** | 3.2 Clock write/read MCP tools + per-book elapsed rollup | L | — |
| **NEXT** | 3.3 "In-world time" hard-constraint render | M | — |
| **NEXT** | 3.4 `BookDigest` story-so-far retention | L | **V0018** |
| **NEXT** | 3.5 Knowledge-timing gate (ordinary `knowledge_fact`) | M | — |
| **LATER** | 4.1 Advance the inert `pacing_tracker` | M | — |
| **LATER** | 4.2 Realized scene intensity + `pacing_drift` check | L | **V0019** |
| **LATER** | 4.3 Wire `semantic_references` (placement-filtered recall) | L | + migration |
| **LATER** | 4.4 Knowledge/relationships into `ValidatorContext` + `move_scene` invalidation | M | — |
| **LATER** | 4.5 Branch merge/diff canon coverage + item-level budget thinning | L | — |

The four NOW non-clock fixes (2.1–2.4) close active drift today and need no migration; 2.5 is the
foundation the NEXT tier consumes.

---

## 3. NOW tier — close active drift, lay the foundation

### 2.1 Bound and widen `story_index` (prerequisite for 2.2 and all ordering)

**Highest-value, lowest-churn half first:** add a creation/persist-time **guard** returning a
typed error (not `debug_assert!`, which is a release no-op) when `chapter_number` or
`scene_order >= radix`, in `create_chapter` (`service.rs:1150`) **and** repository
`persist_scene` (`repository.rs:~1375`). This converts silent transposition into a rejected write.

**Then widen + centralize:** `const SCENE_RADIX = 1_000`, `CHAPTER_RADIX = 1_000`
(`book*1_000_000 + chapter*1_000 + scene_order`); change
`story_index` / `story_index_from_placement` / `story_index_from_scene` / `end_scope_index` /
`chapter_story_index` to return `i64` (the type change forces every call site to be revisited),
reconcile `chapter_story_index` (`format.rs:1432`) to the same scheme, update the `/100` divisor
(`format.rs:1829`) to `/SCENE_RADIX` and the `<=100` "soon" threshold to `<=SCENE_RADIX`, and
delete the false doc comment. Frame this as a **multiplier collision/transposition** fix (it is
*not* i32 overflow — i32 never overflows here).

> Widening **without** the guard just moves the cliff from chapter 100 to chapter 1000. Ship the
> guard regardless; widen in the same PR if churn is acceptable.

### 2.2 Promise timing — fix the unit bug + honor `planned_payoff` (ship together)

Extract a shared `promise_timing_verdict(promise, current_index) -> (urgency, overdue_by, message)`
in `format.rs` adjacent to `narrative_promise_due_summary` (`format.rs:1821`) and route **both**
it and the `check_consistency` arm (`service.rs:6708-6733`) through it, eliminating the contradiction
between the two surfaces.

Rules:
- **(a)** Return non-flagging first for resolved/`paid_off` (the stored value is `"paid_off"`, not
  `"paid"` — verified `service.rs:6050`). `check_consistency` does **not** pre-filter (it drops via
  `_ => continue` at `6733`), so a naive extraction would start flagging resolved promises — guard
  this explicitly.
- **(b)** `planned_payoff = Some` → **overdue/error** when `current_index >= payoff_index` and
  unpaid; **soon** when `payoff_index − current_index <= SCENE_RADIX`; otherwise silent (no more
  false "going cold" on deliberate long arcs). Use the scene-level index (`*SCENE_RADIX`), not the
  divergent `chapter_story_index`.
- **(c)** `planned_payoff = None` → status-aware heuristic dividing by `SCENE_RADIX` (preserving
  the planted-vs-reinforced distinction), with a **configurable, chapter-scaled** cool threshold
  replacing the hard-coded 3/5.

Tests: `format.rs` units (planted→watch→due; `paid_off` stays resolved — regression; reinforced
fallback) + a `check_consistency` integration test for the payoff path (zero coverage today).
**Drafting-model surface:** `get_writer_state.open_promises_due_now` and scene-context
`narrative_promises_due` now agree; the ranked `overdue > due > soon > watch` list lets token
trimming drop `watch` first.

> Depends on 2.1 for the `SCENE_RADIX` constant and correct cross-chapter math.

### 2.3 Gate the `future_knowledge` briefing leak (standalone, no migration)

Apply the `story_index_from_placement(learned_at) <= cursor` filter to `raw_future_knowledge`
before it reaches **both** `future_knowledge_briefing` (`service.rs:6017-6024`) and the
`knowledge_briefing` append (`service.rs:6079+`); also drop/flag items whose `expires_at < cursor`.
Test: future knowledge with `learned_at` after the cursor does not appear; at/before does. Once
the clock lands this can additionally gate on `learned_day_index`, but the position gate is the
immediate correctness fix. **Drafting-model surface:** directly stops priming the generator with
knowledge a POV character shouldn't have yet.

### 2.4 Write-time continuity gate on `commit_scene_changes`

**Precondition (must land first):** make Gate 2 scope/window-aware — group active facts by
`subject_table:subject_id:predicate` and flag a contradiction only when >1 **distinct value with
overlapping or absent validity windows**, consulting the `scope`/`valid_from`/`valid_until` that
are currently written-but-ignored. Without this, the gate spuriously blocks legitimate evolution
(a character's changing location/rank/age) — the dominant regression risk.

Then add a private `commit_continuity_precheck` **before** the mutation loops
(`service.rs:8861`) collecting:
- `detect_fact_contradictions(active + prospective input.canonical_facts)` (the scope-aware helper), and
- `scan_retcon_findings(&project_id, &scene)` (`service.rs:8019`, today only called from
  `revise_scene:9268`), mapped via a new `retcon_finding_to_consistency_issue`.

DTOs (`spindle-core/src/models.rs`):
- `CommitSceneChangesInput` (`models.rs:1549`) `+= accept_continuity_risks: bool` (`#[serde(default)]`)
  `+ continuity_gate: Option<CommitContinuityGate{ Off, WarnOnly, BlockErrors }>` (default `BlockErrors`).
- `CommitSceneChangesOutput` (`models.rs:1808`) `+= blocking_continuity_findings: Vec<ConsistencyIssue>`
  `+ retcon_findings: Vec<RetconFinding>`; add `RetconFinding::PrematureKnowledge{ character_id, fact, learned_at, message }`.

Bail mirroring the world-rule bail at `8853`. **Wire `accept_continuity_risks` into both override
sites** (`execution.rs:288`, `tools.rs:3115`) with `block_on_error = true` default for automated
runs — this is the fix that stops unattended drafting from baking in detected drift.
**Drafting-model surface:** `blocking_continuity_findings`/`retcon_findings` are public MCP DTOs
the authoring agent reads on every commit — a write-time ratchet.

### 2.5 Story-time data model — `StoryClock` + `project_calendar` (migration V0017)

**Core DTOs** (`spindle-core/src/models.rs`; MCP stays thin per `CLAUDE.md`):

```text
StoryClock {
  day_index:       Option<i64>,    // in-world day from project epoch (monotonic, spans books)
  time_of_day:     Option<i32>,    // minutes from midnight, 0..(hours_per_day*60)
  duration_days:   Option<f64>,    // in-world span of the scene/event
  precision:       Option<String>, // minute | hour | day | week | month | year (display + tolerance)
}
CalendarDef {
  days_per_week:   i32,
  hours_per_day:   i32,            // DEFAULT 24 — REQUIRED so time_of_day validates and a total
                                   //   order computes on non-24h calendars
  week_day_names:  Vec<String>,
  months:          Vec<CalendarMonth>,   // { name, days } — invented calendars
  days_per_year:   i32,
  epoch_label:     Option<String>,
}
```

Mirror `StoredStoryClock` in `json_records.rs` (parallel to `StoredStoryPlacement` at
`json_records.rs:139`).

**Non-linear support (all-three-shapes):** also add nullable `temporal_mode`
(`linear | flashback | flashforward | concurrent`, default `linear`) and `thread_key`
(parallel-POV key, default `main`) on `scene`, so the chronology validator (3.1) can distinguish a
deliberate flashback / parallel thread from an accidental backward jump.

**Migration `V0017__story_time.sql`** (V0016 is the verified latest):
- new `project_calendar` table: `project_id` PK/FK `ON DELETE CASCADE`, `days_per_week`,
  `hours_per_day` DEFAULT 24, JSON columns with `CHECK(json_valid(...))`, `days_per_year`,
  `epoch_label`, timestamps.
- `ALTER TABLE scene ADD COLUMN story_day_index INTEGER, story_time_of_day INTEGER,
  story_duration_days REAL, story_time_precision TEXT, temporal_mode TEXT, thread_key TEXT;`
  (all nullable, **appended at the table end** — see §6 positional caveat).
- matching nullable `*_day_index` columns on `timeline_event`, `future_knowledge`,
  `temporal_intervention`.
- indices: `idx_scene_thread_time(project_id, branch_id, thread_key, story_day_index)`,
  `idx_timeline_event_occurs(project_id, branch_id, occurs_day_index)`.

**Record plumbing:** extend `SCENE_COLUMNS` and `Scene::try_from` (`records.rs:206`) via
`row::opt_int` / `row::opt_real` (append indices last). Add
`format::story_time_index(clock, calendar) -> Option<i64>` =
`day_index * (hours_per_day*60) + time_of_day.unwrap_or(0)`, `None` when `day_index` is `None` —
it **must** take the calendar.

**Write-time validation only:** reject `day_index < 0`, `duration_days < 0`, `time_of_day` outside
`0..(hours_per_day*60)`, and `months` summing `!= days_per_year`. The calendar is **not** required
to draft — fully opt-in.

**Out of scope for this migration:** the chronology validator, the promise rewrite, and any
hard-constraint render (those are separate items). This PR ships the primitive only.

---

## 4. NEXT tier — where chronology and long-range value materialize

### 3.1 `ChronologyDrift` validator (6th `PhaseFourCacheId`)

Add `ChronologyDrift` to `PhaseFourCacheId` (`service.rs:17674`) with `as_str` / `all` /
`PHASE_FOUR_CHECK_TYPES` entries; register `ChronologyDriftValidator` in `validators.rs:19`.

**MVP sub-checks only:**
- **Monotonicity:** flag a scene whose `story_day_index` < the prior in-manuscript scene's
  `day_index` on the same `thread_key`, **unless** covered by a `temporal_intervention` or a
  `flashback`/`concurrent` `temporal_mode` marker (severity Warning).
- **Intervention orientation done right:** replace the manuscript-order comparison at
  `service.rs:6989-6991` with `source_day_index` vs `target_day_index` **when both events carry
  `day_index`**, keeping the legacy manuscript check as a **fallback only** when `day_index` is
  absent (do not delete it — it's the only timeline check for non-time projects).

**Defer** duration / age / season checks — they need NL time-phrase parsing the fixed-marker
scanner doesn't generalize to, and would flood at scale.

**Plumbing:** snapshot structs in `registry.rs` (`SceneClockSnapshot`, `EventClockSnapshot`,
`CharacterBirthSnapshot`); add `CalendarDef` to `ValidatorContext` (`registry.rs:55`) built in
`build_phase_four_validator_context` (`service.rs:8286`); hash the calendar + all `day_index`/
`duration` into `phase_four_context_hashes` (`service.rs:17742`); extend
`phase_four_cache_target_for_entity_update` (`service.rs:299`); the new
`set_scene_clock`/`set_project_calendar` paths call `resolve_phase_four_caches([ChronologyDrift])`.

**Correctness guard:** strict no-op unless a calendar **and** ≥1 `day_index` exist (mirror the
`StyleCompliance` early-return at `validators.rs:281`) → zero new findings for non-time projects.

> Semantic caveat (see Risks): `source_event_id` vs `target_event_id` direction is **undefined** in
> code/schema/docs, and `intervention_type` is free text. Do **not** claim the legacy check is
> "provably backwards." The defensible improvement is to make in-world `day_index` the authority
> when present and fall back to (clearly-labeled) manuscript order when absent.

### 3.2 Clock write/read tools + per-book elapsed rollup

Clock columns are inert without a write surface — and the `continuity-editor` skill already orders
the model to verify in-world dates and ages (`skills/continuity-editor/SKILL.md:78-80`) with
nothing able to record them.

DTOs (`spindle-core/src/models.rs`): `SetProjectCalendarInput/Output(CalendarDef)`,
`SetSceneClockInput/Output(scene_id + StoryClock)`, `SetTimelineEventClockInput/Output`,
`SetCharacterBirthInput/Output(character_id + birth_day_index)` — all carrying `project_id`,
operating on the active `branch_id` like every `Set*` tool. Register in `tools.rs` after the
timeline tools (`tools.rs:608`) via the `self.invoke` dispatch pattern.

Each setter invalidates `ChronologyDrift` once it ships — **note:** the claim that
`create_timeline_event` invalidates `ChronologyDrift` is false; it invalidates
`RetconReachability` (`service.rs:1322`); until ChronologyDrift exists, invalidate
`RetconReachability`.

**Per-book elapsed rollup** as a pure read (no new table) but **project- and branch-scoped**:
`SELECT book_number, MIN(story_day_index), MAX(story_day_index) FROM scene WHERE project_id=? AND
branch_id=? AND story_day_index IS NOT NULL GROUP BY book_number` (scene is per-branch; an unscoped
GROUP BY mixes branches). **Drafting-model surface:** an "in-world time" line in
`get_chapter_briefing` (`service.rs:5341`) and `get_writer_state` (`service.rs:4745`), emitted only
when dated scenes exist. Update `docs/spindle-architecture.md`, `docs/spindle-implementation-brief.md`,
and the `continuity-editor`/`scene-writer` skills per `CLAUDE.md`.

### 3.3 "In-world time" hard-constraint render (the prevention mechanism)

Even after storing `day_index`, the model never sees it: `timeline_briefing` is
`SectionKind::Supplementary(2)` (`format.rs:2447`) — the first thing dropped under budget pressure
(`enforce_budget`, `context_bundle.rs:184-196`). Hard constraints today are only world rules +
canonical facts (`service.rs:6358-6371`).

Add a builder `temporal_hard_constraint(scene_clock, calendar) -> Option<HardConstraint>` in
`format.rs`, returning `None` when the project has no calendar (zero change for non-time projects —
unit-test on an existing fixture). In `get_scene_context` (`~service.rs:6358`) **prepend** it to
`hard_constraints` before `build_scene_context_bundle` (`service.rs:6381`) so it rides the
non-truncatable section and the existing ~4000-token headroom. MVP string limited to data that
exists: `"IN-WORLD TIME: Day N (calendar-rendered); elapsed since chapter 1: N days. Do not
contradict this clock."` Defer the POV-ages line (needs character births + day→year conversion).
This block must also be injected into the `caller_should_send_brief` path so Grok / a future
Claude-CLI adapter receive the clock (it's derived, not a single fetchable record).

### 3.4 `BookDigest` — auto-maintained "story so far" (migration V0018)

The only durable rollup today is per-chapter `ChapterSummary` surfaced as ≤5 recent chapters;
`BookSummary` is a bare `{id, number, title}` pointer (`models.rs:969-974`); `BookOutline` isn't
injected into scene context at all. Drafting book 3 ch 10 surfaces nothing from books 1–2.

**Migration `V0018__book_digest.sql`:** `book_digest(id PK CHECK LIKE 'book_digest:%',
project_id/branch_id FK ON DELETE CASCADE, book_number, synopsis TEXT NOT NULL, open_threads TEXT
json, last_chapter_covered INTEGER, token_estimate INTEGER, updated_at,
UNIQUE(project_id, branch_id, book_number))`. Record + columns const + `TryFrom` in `records.rs`
mirroring `ChapterSummary` (`records.rs:1248`); core `BookDigestSummary` in `models.rs`.

**Maintenance folded inside `repository.save_summary`'s existing writer transaction**
(`repository.rs:6840-6858`) so digest and chapter summary are atomic: a deterministic
concatenate-of-(summary + key_events + arc_advances + promise_events)-and-cap, gating an optional
model-router compaction pass on `token_estimate > cap` (computed **before** the transaction — keep
model calls out of the DB closure).

**Injection:** (a) `build_chapter_briefing_bundle` (`format.rs:2579`) at `Supplementary(150)` —
above `book_outline(100)`/`recent_chapter_summaries(50)`, below `chapter_plan(175)`; (b)
`SceneContextNovelLayer` (`models.rs:478`) `+= book_digest: Option<BookDigestSummary>` injected in
`build_scene_context_bundle` (`format.rs:2414`) — **this path loads no chapter summaries, so the
digest is its only cross-book memory.** `check_consistency` info-note when
`max(ChapterSummary.chapter_number) > last_chapter_covered`. Scope strictly as memory-rollup — do
**not** overload it with a timing model.

### 3.5 Knowledge-timing gate for ordinary `knowledge_fact`

Who-knew-what-when is enforced only for `future_knowledge`, only in `revise_scene`. The ordinary
`knowledge_fact`/`knows` graph carries `learned_at` but is never compared to the scenes a character
appears in, and `ValidatorContext` carries no knowledge — so a character referencing a secret/death
in ch5 established in ch40 produces zero findings.

Extend the NOW-tier commit gate: in `scan_retcon_findings` (`service.rs:8019`), after the
`DeadCharacterAct` loop, load `list_knowledge_facts_by_project_and_branch` (`repository.rs:9062`,
main-branch fallback); for each character whose name appears in `scene.full_text` (reuse
`contains_case_insensitive_word_boundary`, `service.rs:17945`), for each of that character's facts
where `learned_at IS Some` AND `position_gt(learned_at, scene_position)` (`service.rs:17926`) AND
`contains_case_insensitive_phrase(scene.full_text, fact.normalized_fact)` (`service.rs:17932`) →
emit `RetconFinding::PrematureKnowledge`.

**Precision guardrail:** `normalized_fact` is the whole fact sentence lowercased
(`normalize_name`, `models.rs:5436`), so verbatim matches are rare — keep severity **Info/advisory**
(high-precision, low-recall tripwire), not a hard gate. Promotable to blocking behind the override
flag.

---

## 5. LATER tier — round out coverage

- **4.1 Advance the inert `pacing_tracker`.** `current_progress/budget_remaining/velocity/status`
  are written once at seed (`repository.rs:6575`) and never recomputed, so `pacing_budget_audit`
  (`service.rs:6795`) evaluates frozen seed values and essentially never fires. Add
  `update_pacing_tracker_progress` and recompute in `save_summary` (`service.rs:1014`) from the full
  set of that book's chapter summaries (re-derive each call — `save_summary` is a replace-by-(book,
  chapter) upsert, so it must be idempotent), gated on a `per_book_budget` entry.
- **4.2 Realized intensity + `pacing_drift`** (migration `V0019__scene_intensity.sql`): add
  `scene_beat_annotation.intensity REAL`; **COALESCE on re-annotate** (`annotate_scene_beats` is
  DELETE-then-INSERT, `repository.rs:6391-6411`, so an omitted value erases it). Add
  `expected_chapter_intensity(tension_model, act_breakpoints, chapter_index, chapter_count)` (there
  is no planned scalar to compare against until this exists), and a `pacing_drift` branch in
  `check_consistency` that fires only on a **sustained** realized-vs-expected gap across N
  consecutive chapters (a single spike is intentional). Not a `PhaseFourCacheId` — pacing is
  book/chapter-level, not per-scene.
- **4.3 Wire `semantic_references`** (the `Vec::new()` at `service.rs:6114`). **Not zero-schema:**
  canonical facts and scenes aren't embedded (`register_canonical_fact:2849` and the rebuild loop
  never embed them), and `search_embedding` carries no placement (`V0001:1099`), so
  `fact_at_or_before` can't be reused. Requires (A) embedding facts/scenes, (B) nullable
  book/chapter/scene_order columns on `search_embedding` (+ reindex) populated from each entity's
  establishment placement with an at-or-before-cursor SQL filter in `knn_search_embeddings`
  (`repository.rs:7721`), (C) replacing `service.rs:6114` to embed the seed and map placement-filtered
  kNN hits to `SearchBibleResultItem`. Resurfaces distant-but-relevant canon by relevance.
- **4.4 Knowledge/relationships into `ValidatorContext` + `move_scene` invalidation.**
  `ValidatorContext` (`registry.rs:55`) carries no knowledge/relationships, so the suite can't see
  them; and `move_scene` (`service.rs:1986`) changes placement without touching `full_text` (so
  `scene_text_hash` is unchanged) and calls no `resolve_phase_four_caches`, leaving placement-
  dependent findings (`canonical_fact_prose_drift`, `retcon_reachability`) stale after restructuring.
- **4.5 Branch merge/diff canon coverage + item-level budget thinning.** `merge_branch`
  (`service.rs:11495`) merges only scenes/character_states/relationships/pacing_trackers —
  world_rules/canonical_facts/promises/timeline/knowledge/arcs are silently dropped, and
  `diff_branches` never surfaces canon divergence. Add canon tables to the merge/diff snapshot with
  conflict detection. Separately, add `Section::trim_to(target_tokens)` with per-item importance
  (timeline by placement proximity, promises by `urgency` at `models.rs:563`, knowledge by character
  relevance) so budget pressure drops the lowest-value *items* rather than an entire canon category.

---

## 6. Sequencing & dependencies (non-negotiable order)

1. **Gate-2 scope-aware refactor → before 2.4** (else the write-gate false-positives on legitimate
   evolution — the dominant regression risk, and it hits the user's exact priority).
2. **2.1 radix fix → before 2.2** (the promise math and every ordering site depend on a correct,
   non-colliding index).
3. **2.5 (V0017) → before all NEXT-tier temporal work** (no clock columns, no chronology).
4. **Migrations serialize:** `V0017` story_time, `V0018` book_digest, `V0019` scene_intensity.
   Multiple proposals independently claimed V0017; the runner fails on duplicate numbers.
5. **Skills/docs adoption is part of the work, not after it:** the clock produces **zero**
   protection until authors/the loop actually populate `day_index`. Budget `scene-writer` /
   `continuity-editor` skill + `docs/` updates alongside 2.5/3.2/3.3.

---

## 7. Risks

- **Evolving-fact false positives** (dominant): Gate 2 ignores `scope`/`valid_from`/`valid_until`;
  a write-gate shipped before the scope-aware refactor will block legitimate timeline evolution and
  erase the historical record needed for chronology. Sequence per §6.1.
- **Migration numbering collisions** (§6.4).
- **Opt-in dormancy:** the additive clock is correct for back-compat but yields no protection until
  populated — adoption in skills + the authoring loop is required to realize value.
- **Low-recall knowledge matching:** `normalized_fact` is a whole lowercased sentence, so
  `contains_case_insensitive_phrase` has near-zero recall; the `future_knowledge` scan shares this.
  These are high-precision/low-recall tripwires, **not** guarantees — keep advisory by default;
  overselling them creates false confidence.
- **Cache cost at scale:** adding ChronologyDrift/knowledge/relationships to the Phase-4 context
  hash means clock/knowledge/relationship edits invalidate caches; combined with the existing
  branch-wide blow-away (`register_canonical_fact` resolves project-wide at `service.rs:2913`;
  `resolve_validator_findings_for_validator` clears all open rows at `repository.rs:3554`), each edit
  can trigger whole-branch re-validation. Validate invalidation granularity before adding more cached
  validators.
- **Radix widening blast radius:** the `i32 → i64` change touches ~28 sites; it's compiler-guided but
  broad. The **guard alone** delivers most of the safety with far less churn if widening must be
  deferred.
- **Intervention orientation is semantically undefined:** don't invert the existing comparison;
  introduce `day_index` as the authority when present and fall back to labeled manuscript order.

---

## 8. Test plan

- **Record round-trip:** scene/timeline/character with and without story-time survive save→load;
  positional column indices intact.
- **Calendar:** Gregorian and a custom invented calendar (incl. non-24h `hours_per_day`) render and
  validate consistently; weekday/season derivation correct across a year boundary.
- **Radix:** `story_index(1, CHAPTER_RADIX, 0) != story_index(2, 0, 0)`; `create_chapter` /
  `persist_scene` reject at-radix values.
- **Promise timing:** distant `planned_payoff` stays silent until the soon window; overdue fires past
  payoff; `paid_off` stays resolved (regression); unscheduled uses the scaled threshold; book-1
  promise with book-3 payoff silent during book 1; the two surfaces agree.
- **`future_knowledge` gate:** `learned_at` after cursor is absent from briefings; at/before present.
- **Write-gate:** contradiction over canon blocks (and is overridable); legitimate evolving-fact
  change (distinct validity windows) does **not** block.
- **`ChronologyDrift`:** unflagged backward jump (warn); declared flashback clean; parallel-thread
  simultaneity clean; non-time project → zero findings.
- **Scene context:** the temporal hard constraint lands in the non-truncatable prefix and in both the
  pre-packed and brief paths; survives a tight budget.
- **No-time project:** full suite green with zero story-time set (proves additivity).

---

## 9. Open questions

1. **Clock decomposition** — confirmed: `day_index` + `time_of_day` (minutes from midnight) +
   `duration_days` + `precision`, with the derived total-order index from §2.5. (Author-friendly;
   avoids huge minute counters.)
2. **Calendar storage** — dedicated `project_calendar` table (recommended) vs a column on `project`.
3. **Block vs advise in NOW tier** — the write-gate defaults to `BlockErrors` but is overridable;
   `ChronologyDrift` is advisory until the gate consumes it. Confirm the default for automated runs.
4. **Auto-stamping `day_index`** from prose during drafting — deferred; Tier work is author/loop-declared.
5. **Thread model depth** — start with a `thread_key` string for parallel POV; promote to first-class
   thread records only if needed.

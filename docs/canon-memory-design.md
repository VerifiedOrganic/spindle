# Canon extraction, typed events, and layered memory

Status: proposed Wave 2 / P2 design. **No implementation is included.**

## Scope and evidence

This design separates prose generation from canon acquisition, replaces competing
state authorities with typed event projections, and makes memory retrieval
position-aware. P1 owns drafting packets and critique/revision policy; P3 owns
plans; P4 owns tool consolidation; S owns behavior-preserving module extraction.
P2 supplies claims, projections, findings, and retrieval contracts to those seams.
The locations below describe the pre-split code, not implementation placement
requirements for S.

All descriptions of shipped behavior below refer to `origin/main` at
`419f40be7378d8919ac89d28139496d06b7db935` (fetched 2026-11-08).
Evidence references link to that immutable commit; labels give `file:line`.
Everything after the audit is a **proposal**, including thresholds and budgets,
not a claim that the feature exists or its performance has been measured.

### Evidence ledger

- [E1: crates/spindle-harness/src/artifacts.rs:134](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-harness/src/artifacts.rs#L134): generated scene package contains prose, summary, character states, canonical facts, relationship updates, beats, and continuity notes.
- [E2: crates/spindle-harness/src/execution.rs:378](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-harness/src/execution.rs#L378): package fields feed commit; line 398 selects `WarnOnly`. Package validation at [execution.rs:2476](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-harness/src/execution.rs#L2476) checks prose/summary and character membership, not extraction completeness.
- [E3: crates/spindle-adapters/src/sqlite/service.rs:13280](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L13280): character lists are replaced when supplied; omitted lists inherit. Line 13343 onward creates canonical facts; line 13433 reconciles scene relationship contributions.
- [E4: crates/spindle-core/src/models.rs:7150](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-core/src/models.rs#L7150): mining defaults disabled and accepts only `propose_all`; `auto_accept` rejection is tested at line 7910. [V0024__canon_delta.sql:1](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0024__canon_delta.sql#L1) defines staged proposals, evidence, confidence, and decision audit.
- [E5: crates/spindle-core/src/models.rs:141](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-core/src/models.rs#L141): character patch has free-text goals/status/notes and an emotional-state map, no typed injury or possession operations.
- [E6: crates/spindle-adapters/src/sqlite/validators.rs:30](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/validators.rs#L30): prose drift matches fact key and missing value, skipping the establishing scene. [service.rs:9911](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L9911) requires character-name presence plus normalized fact phrase for knowledge timing. [world_rules/scanner.rs:107](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-core/src/world_rules/scanner.rs#L107) skips absent scan patterns.
- [E7: crates/spindle-core/src/validators/registry.rs:66](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-core/src/validators/registry.rs#L66): ValidatorContext has scenes, facts, rules, voice, timeline/interventions and style, but no knowledge, relationships, or character state. [service.rs:11683](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L11683) defines the separate retcon scanner; its dead-state path is at line 11836. Call sites at lines 13216 and 13770 cover commit/revision, whereas consistency's registry fan-out is at line 10472.
- [E8: crates/spindle-adapters/src/sqlite/repository.rs:14968](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/repository.rs#L14968): parent-chain resolution is child-first; effective scene branch resolution follows at line 15021. [repository.rs:16151](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/repository.rs#L16151) documents natural-key child overrides; [repository.rs:12363](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/repository.rs#L12363) assembles subject snapshots from records.
- [E9: crates/spindle-adapters/migrations/V0001__initial_schema.sql:183](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0001__initial_schema.sql#L183): character_state; world_state/world_rule at 240/256; timeline_event/system_overlay at 572/602; future_knowledge/knowledge_fact at 624/641; progression_event at 742; canonical_fact at 812; knows at 1077. [V0020__quantity_state.sql:27](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0020__quantity_state.sql#L27) adds append-only quantities; [V0017__story_time.sql:22](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0017__story_time.sql#L22) adds scene clocks.
- [E10: crates/spindle-adapters/src/format.rs:3878](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/format.rs#L3878): STORY SO FAR budget is exactly 2,000 bytes. [service.rs:6858](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L6858) selects configurable recent-summary limit; [format.rs:2801](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/format.rs#L2801) orders/truncates it; the default is three at [format.rs:1881](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/format.rs#L1881). [service.rs:7914](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L7914) runs semantic recall through embedding/kNN.
- [E11: crates/spindle-adapters/src/ai.rs:21](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/ai.rs#L21): token-hash-v1 uses 64 dimensions; algorithm at line 213. HTTP embedding backend already exists at [ai.rs:1633](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/ai.rs#L1633). [V0002__vec0_search_embedding.sql:18](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0002__vec0_search_embedding.sql#L18) fixes the vector index to FLOAT[64]. [service.rs:18709](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/src/sqlite/service.rs#L18709) sorts miner promises by id before capping; cap is 30 at line 28145.
- [E12: evals/results/continuity_recall_baseline.json:11](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/evals/results/continuity_recall_baseline.json#L11): 16/28 overall, five control false positives, 0/4 dead-character detections. [evals/continuity_recall.py:1203](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/evals/continuity_recall.py#L1203) supplies dead-character fixtures and accepted check types.

### Audit interpretation

The drafter supplies both prose and its continuity report (E1–E2). A report
with no state/fact/relationship changes can pass package validation; notes are
not proof of complete coverage. The harness commits through WarnOnly, writing
facts directly, while independent mining is optional and stages proposals for
ratification (E2–E4). This is an authority asymmetry, not merely a missing prompt.
The package has no dedicated scene-clock, character-location, possession,
injury, quantity-change or promise plant/payoff fields (E1, E5). Generic facts
can encode some information, but do not enforce those domain transitions.
Supplying a new status list replaces the prior list: an omitted injury *inside
a supplied list* can disappear; omitting the entire list inherits it (E3).

Lexical validators miss paraphrases and can mistake mentions for contradictions
(E6). The dead-character scanner exists but is not called by check_consistency;
the registry path is a different path (E7), consistent with the measured zero
recall (E12). Stores overlap without one shared transition model (E9).
Subject snapshots are already assembled read models, not another physical
canon table (E8). Memory has bounded prose summaries and word-hash default
recall; real HTTP embeddings already have a route, but arbitrary dimensions
cannot fit the fixed 64-dimensional index (E10–E11). These distinctions constrain
the migration below.

## 1. Independent extraction contract

Proposed pipeline: save immutable scene revision → independent extraction →
normalize/diff → claim-vs-canon validation → policy decisions → project events →
refresh memory. P1 may request this before revision and again after final prose;
only the final revision's eligible claims can affect canon. The drafter package
becomes a competing assertion set, never a privileged write authority.

Use a separate `extract` pass through the existing rating-cleared routing gate,
with an analysis prompt, fixed schema version, and explicit coverage checklist.
Separate means a fresh invocation without the drafter's reasoning or package;
it need not require a different vendor/model. Input is exact scene text/revision,
entity ids/aliases, preceding ratified state, and a bounded provisional conflict
list. No future plot, future summary, or proposed drafter answers. Route absence,
uncleared rating, parse failure, or timeout records `extraction_pending/failed`;
it must never mean “no changes.” Retry asynchronously and expose host extraction
fallback with the same schema/evidence checks.

A Claim v1 has: claim_id, schema_version, project/branch, scene/revision id,
text hash, extraction run/model/prompt version, subject entity id, predicate
(discriminated enum), operation, typed value, story position, optional in-world
time, modality, speaker/experiencer, confidence score, and evidence array.
Each evidence span carries exact quote plus UTF-8 byte offsets into the hashed
revision. Validate offsets and quote equality locally. Unknown entity references
remain unresolved proposals; never silently invent an existing id. Unsupported
predicates are rejected with a schema finding, not stored as arbitrary JSON.
Evidence is necessary, not sufficient: dialogue, belief, speculation, dreams,
negation and flashbacks must have explicit modality and temporal scope.

| Predicate family | Typed values and operations |
| --- | --- |
| Life / action | life_status set alive/dead/unknown; action occurrence with actor and mode |
| Location / scene clock | located_at entity; move from/to; scene interval with calendar/time precision |
| Possession | acquire/transfer/lose item id with owner/custodian distinction |
| Injury | add/heal injury id, body site, severity; no whole-list replacement |
| Quantity | set/delta measure, decimal amount/unit or band; transaction id for paired transfers |
| Knowledge / disclosure | learn/forget/use/disclose proposition id, knower, audience, source and time |
| Attribute / world state | typed scalar set or explicit set-member add/remove; invariant versus evolving |
| Relationship | directed pair, trust/tension delta or absolute observation, never both implicitly |
| Promise | plant/reinforce/payoff/abandon thread id with evidence; inferred payoff stays reviewable |
| Rule / temporal anchor | rule constraint definition or rule-use occurrence; intervention and anchor references |

Coverage result reports each family as inspected-none, claims-present, or
unknown, with reasons; an all-none result is allowed but sampled for missed
claims. Stable proposition/entity resolution is deterministic first, with bounded
model-assisted equivalence only for unresolved cases. Do not collapse a belief
about a proposition into its truth.

Normalize the drafter package into the same claim schema, preserving `source =
drafter` and absent evidence. Compare semantic keys (subject, predicate, position,
operation, normalized value), not strings. Record agreed, extractor-only,
drafter-only, and incompatible claims. Every disagreement becomes a structured
finding with both sources, evidence and recommended resolution; high-impact
uncertainty prevents auto-ratification. Agreement alone cannot override old canon.
Extraction and package coverage metrics remain visible even if no contradiction
is found. Neither source wins by confidence alone.

## 2. Append-only typed events and projections

Add `canon_claim`, `canon_event`, `canon_decision`, `canon_source_map`, and
rebuildable projection/checkpoint tables. Core owns DTOs, enums and pure reducers;
adapters own SQL, extraction orchestration and embedding providers; MCP/harness
consume service contracts. This follows the current crate boundaries documented
in the required architecture references, without prescribing S's file split.

A canon_event records immutable event id, project/branch, subject, predicate,
operation/typed payload, effective story position, source revision/claim ids,
evidence hash, optional supersedes/withdraws references, and append sequence.
Index `(project, branch, subject, predicate, book, chapter, scene_order,
within_scene_order, append_seq)`. Event identity is not that index: multiple
injuries/actions/transfers can occur at one position. Unique idempotency key is
branch + source revision + semantic claim fingerprint + policy action; retries
cannot add quantity/relationship deltas twice. Fixed enums validate JSON payloads
if SQL stores tagged JSON. Validate references, units, transfer conservation and
explicit set preconditions before append, in one transaction with the decision.

Keep narrative position separate from in-world time and append time. Query
`state_at(branch, position, tier, decision_watermark)` folds visible events through
the cursor; within-scene event ordering permits knowledge learned halfway through
a scene to precede use. Unknown ordering returns uncertain, not a guessed error.
A later-arriving correction effective in chapter 2 changes today's chapter-2
projection; a historical decision watermark reproduces what was known before
that correction. Missing values stay unknown, never zero/alive/uninjured.

Projections include entity state, relationships, quantities, knowledge/circle of
trust, rule constraints, clocks/timeline, and open threads. Each stores source
event ids and reduction version. Scalar set changes only its predicate; injury
and possession sets retain independent member ids. Delta reducers require an
opening balance; observations without one do not invent a balance. Definitions
of units, calendars and rule schemas remain versioned configuration. Summaries
are derived memory, never canon authority.

### Branch, revision, and rollback semantics

Preserve live parent-chain inheritance (E8), rather than introducing an implicit
fork-time snapshot. Resolve child → parent → main, never siblings. Apply
child-over-parent visibility by semantic stream key, not by copied row id.
A child override must explicitly declare a coverage interval: before its start,
ancestor state remains visible; during coverage it shadows the ancestor stream;
after its end inheritance resumes. For collections, the stream key includes
member/item id. Relationship/quantity delta streams default to inherited history
plus child events; replacing an ancestor contribution requires an explicit
branch-local mask targeting the event, not a second compensating debit inferred
from prose. A child correction must never mutate the parent event or decision.

Compatibility projections must first reproduce legacy natural-key masking,
including whole-record overrides, via explicit legacy coverage masks. New
predicate-level behavior is an explicit upgrade, not a silent reinterpretation
of old branch reads. Test main → child → grandchild, ancestor edits after fork,
inherited scenes committed on active descendants, siblings, and tombstones.

Revision/remine appends source-invalidated and replacement decisions; it does
not delete history or accumulate old contributions. Move/delete/restore changes
a versioned scene-placement/visibility mapping and invalidates affected suffixes.
Save points capture projection/reducer version, lineage and decision watermarks;
restore appends branch-local visibility decisions to reproduce that view.
Merge compares common ancestor, target and source semantic streams; explicit
conflicts cover divergent scalar sets, removals, and duplicate transfers.
Accepted merge events retain source provenance and new target ids; target-local
supersession never rewrites source history. Copy relationship contribution ledgers
and predecessor links logically, preserving the existing guarantees (E3, E8).

### Migration and retirement map

No big-bang rewrite. Backfill with stable `(table, legacy primary key, migration
version)` mappings and transaction checkpoints; reruns are idempotent. Retain raw
legacy values and links, with `legacy_unverified` provenance. An unknown placement
is a baseline of unknown effective time, not scene zero. Existing explicitly
accepted/manual canon imports as ratified legacy canon; staged proposals remain
provisional. No fabricated prose quotes or calibrated confidence for old rows.

| Existing store (E8–E9) | Backfill and final role |
| --- | --- |
| canonical_fact | Attribute/rule assertion events preserving scope, validity, secret and predecessor links; retire writes, then replace with compatibility projection |
| character_state | Ordered observed snapshots → explicit per-field/member differences; preserve raw ambiguous statuses as legacy annotations pending extraction; retire authority |
| world_state | Location/world predicate observations; retire authority after parity |
| world_rule | Retain rule identity/configuration and legacy scan pattern; version rule semantics through events, remove duplicate mutable truth fields after parity |
| knowledge_fact + knows | Deduplicate proposition ids; retain each knower edge, learned_at, confidence, reader visibility, secret link and source; project both legacy read shapes from knowledge events |
| future_knowledge | Knowledge-acquisition events with displaced-time/source-intervention qualifier and expiry; do not fold into ordinary knowledge without that qualifier |
| quantity_state | Absolute observations with units/bands, not invented deltas; compatibility projection after parity; keep quantity-scheme configuration |
| progression_event | Preserve audit references and import only uniquely attributable typed changes; mark duplicates of state rows through source_map, avoid double application; quarantine opaque delta JSON |
| system_overlay | Keep overlay definition/configuration; migrate dynamic overlay state to typed subject predicates |
| timeline_event | Keep event identity/metadata; typed occurrence/anchor events project temporal truth; preserve intervention references |
| scene_clock (+ timeline_event_clock) | Typed interval/clock observations including precision/calendar and provenance; retire independent state writes |
| subject_snapshot | Retain public assembled DTO, switch inputs to projections; there is no table to backfill |
| narrative_promise / relationship_scene_delta | Keep thread/entity metadata; migrate realized thread transitions and per-scene relationship contributions with deduplication, then project legacy views |
| canon_delta | Preserve staged/applied/rejected/superseded history and decisions; map proposals to claims and applied effects to events, never reapply an already-applied delta |

Stage A adds tables and backfills without changing reads. Stage B routes writes
through a single service transaction that appends events and maintains old tables;
compare normalized old/new reads in shadow mode. Never run two independent
writers. Stage C flips per-project reads after parity, preserving old projections
for one release and a reversible feature flag. Stage D removes legacy write
paths, then drops obsolete tables only in a later backed-up migration. Rollback
before D switches reads to transactionally maintained old projections; after D
requires restoring backup/replaying events with the supported reducer. Audit and
source mappings outlive table retirement. Differences and quarantined rows block
cutover for that project, not other projects.

## 3. Canon tiers and unattended ratification

Two query tiers: **ratified** is authoritative; **provisional** is evidence-backed
but unresolved. Rejected/withdrawn records remain audit history, excluded from
both. A ratified claim can later be withdrawn only by a new decision. Ratification
never edits evidence. Default validation compares claims against ratified canon;
provisional conflicts yield labeled warnings, never silently override truth.
Drafting context includes relevant provisional constraints and unresolved findings
so unattended work does not simply forget unreviewed changes.

Proposed policy v1, configurable by project and versioned per decision:

| Class | Auto-accept conditions | Otherwise |
| --- | --- | --- |
| Explicit movement, scene clock, item custody, nonfatal injury add/heal, explicit quantity transaction | Calibrated confidence ≥0.98; exact evidence, resolved ids/time/units, legal reducer preconditions, no canon/package conflict | Provisional + finding |
| Ordinary knowledge acquisition, relationship observation, evolving attribute | ≥0.99 and all above; knowledge must be evidenced acquisition, no secret/reveal ambiguity | Provisional + finding |
| Death/resurrection, invariant retcon, new world law, secret disclosure, temporal intervention, promise payoff/abandonment, ambiguous entity | Never auto-accept in v1 regardless of score | Author decision; recommended candidate shown |
| Promise plant/reinforcement | ≥0.98 only for an explicitly referenced existing thread and explicit occurrence; new/inferred threads remain provisional | Provisional + finding |

Confidence is calibrated against held-out labeled claims per class/model/prompt;
raw model self-confidence is not a probability. Until calibration demonstrates
≥99% accepted-claim precision with a reported confidence interval and adequate
class sample size (minimum 100 accepted held-out claims), that class stays shadow
only. Package agreement is useful evidence, not independence or calibration.
Human decisions can always ratify/reject with a reason and audit actor.

Each decision records actor (human/policy id), policy version, confidence inputs,
precondition results, source revision and projection watermark, previous decision,
and reason. Compare-and-append fails on stale canon and revalidates. Withdrawal
of a bad batch appends revocations, rebuilds affected projections, invalidates
summaries/indexes and rechecks downstream scenes. Dependent claims lose automatic
ratification until revalidated; audit records remain. Event/decision append and
outbox notification are atomic so a crash cannot leave an applied effect unaudited.

Normal unattended processing runs extraction before the next scene context is
assembled, auto-ratifies eligible claims and schedules a durable retry queue for
failures. Track pending scene age/count and disputed high-impact claims. Defaults:
retry twice within a 60-second scene budget, then continue with pending warnings;
checkpoint cannot report continuity-clean while extraction is missing. At more
than three pending scenes or one disputed high-impact prerequisite for the next
scene, pause at the next checkpoint with an explicit author task. Never silently
promote a backlog to ratified canon. P1 owns how this status affects editorial
progress; P2 exposes it without treating a skip as success.

## 4. Claim-vs-canon validation

Retain lexical scans as inexpensive retrieval/prefilter hints, not contradiction
proof. Validation receives `CanonValidationContext`: branch lineage, cursor and
within-scene order, claims, ratified projections, separately marked provisional
claims, knowledge/relationships/state, clocks/rules/interventions, and provenance.
This replaces the missing dimensions of E7. Cache keys include scene revision,
claim schema/extractor version, event/decision watermark, lineage, reducer/rule
version and relevant provisional dependencies; no text-only cache hit survives a
canon change.

Deterministic reducers compare normalized claims: blue versus brown attribute;
acting in present narrative after dead status; using knowledge before an
acquisition; spending more than available; impossible movement/clock order;
rule use without required cost; disclosure to an unauthorized audience. Unknown
state produces uncertainty, not contradiction. Rules become typed preconditions
and effects where expressible; unsupported prose rules use bounded semantic
adjudication with quoted evidence, explicit uncertainty and no automatic invariant
rewrite. Not every magic system can be reduced to a numeric cost.

First wire the existing dead-character scanner into scene verify and broad
check_consistency, deduplicated under the existing retcon check family for eval
compatibility. Then replace its character-presence shortcut with an acting claim
plus as-of life state. Mentioning a dead person, quoting a recording, a flashback,
ghost action permitted by a rule, or a valid resurrection must not count as an
unqualified present-life violation. Reuse one implementation across save/revise,
verify, commit and deep scans, avoiding the E7 call-path gap.

A finding contains check type/severity, candidate claim/evidence span, conflicting
event ids and quotes, effective positions, tier, confidence/uncertainty,
recommended correction, and stable dedup key. Do not call absence of a canonical
value in prose a contradiction. Keep model-required checks separate from the
fully deterministic benchmark lane: route failure means incomplete scan, not
zero findings. P1 consumes findings for revisions without P2 changing its loop.

## 5. Layered memory and retrieval

Replace fixed recency-only memory with four derived levels: scene → chapter → arc
→ book. Scene summaries derive from final prose plus event ids; parents summarize
children and carry covered position ranges, unresolved threads, entity ids,
source revision hashes, tier, and summary model/prompt version. P3 supplies arc
membership when available; absent it, use an explicit unassigned chapter group,
not an invented narrative arc. Never use a summary to establish canon.

Edit, move, delete, rollback, merge or ratification invalidates dependent scene
summaries and ancestors via a dependency DAG, plus affected entity/thread index
entries. Recompute lazily with durable jobs and show stale status. Cursor filtering
must precede selection: never retrieve a chapter/book summary spanning later
positions; choose eligible children instead. Reader/POV secret filtering applies
to summaries and quotations as well as event retrieval. Book memory cannot leak
a future reveal just because an older chapter is in the same book.

Build context in this order: ratified hard constraints and scene temporal anchor;
relevant provisional disagreements; cast/location entity state; open threads;
recent scene/chapter detail; relevant arc/book summaries; semantic supplemental
recall. Budget each layer in tokens with explicit truncation metadata, preserving
constraints or returning budget-exceeded. Keep the current recent-summary option
as a compatibility preference, not a hard three-chapter ceiling (E10).

Entity retrieval uses stable ids and aliases, graph neighbors and current
location/possessions/injuries/knowledge. Open-thread retrieval uses realized events
at the cursor, planned due positions separately labeled, and dependencies.
Order digests by overdue first, due soon, unresolved blocking contradiction,
scene/cast relevance, then age and stable id as tie-breaker. Reserve capacity per
class and page overflow with omitted counts; an old low-id thread cannot crowd
out an urgent high-id thread as in the current miner digest (E11). Share this
selection contract with P1 and the miner; P3 owns planned deadlines.

Embedding configuration adds provider, model, dimension, distance metric,
normalization and version to an index-generation identity. Retain token-hash-v1
as a cheap explicit fallback and extend the existing HTTP provider seam (E11).
Validate returned dimension and finite values before insert or query. Never pad,
truncate, or compare vectors from different models/dimensions. Build separate
vec0 tables per generation/dimension (validated identifiers), backfill resumably,
then atomically switch active generation after completeness and retrieval tests.
Queries pin one generation. Keep old generation until rollback window closes;
failed rebuilds leave it active. No provider availability means entity/FTS and
existing hash fallback with honest degraded status, not a fabricated embedding.
Local storage remains default; remote embeddings require configured routing and
rating clearance for prose-bearing inputs, with secrets excluded from logs.

## 6. PR-sized delivery and acceptance

These are future implementation PRs; this PR changes only this document. Preserve
existing defaults until each cutover passes its gate. Coordinate public DTO seams
with S/P1/P3/P4; do not combine their refactors with migrations.

| PR | Deliverable | Acceptance gate |
| --- | --- | --- |
| A | Dead-character call-path wiring and fixtures | Same finding in verify/consistency/commit; dead-character recall ≥3/4; mention/flashback controls pass; overall ≥19/28 and control false positives ≤5 |
| B | Claim schema, independent extraction and package diff in shadow mode | Every family has coverage status; malformed quote/id rejected; notes-only package yields missing-claim finding on seeded prose; conflicts carry both sources; no canon mutation on extractor failure |
| C | Append-only schema and pure reducers | Replay deterministic at every cursor/watermark; injury persists through unrelated changes; transfers conserve quantities; duplicate retries/recommit do not accumulate; rollback restores prior view |
| D | Backfill/source maps and legacy compatibility shadow reads | Every row mapped or explicitly quarantined; rerun idempotent; no duplicated progression effects; all listed stores covered; nested branch, live ancestor edit, merge and restore parity; project cutover blocked on unexplained difference |
| E | Tier decisions, calibrated policy and cutover | Policy thresholds/class sample gates met; stale decision race fails safely; crash/retry audit atomic; revoked batch invalidates dependencies; no unattended eligible-claim backlog beyond next scene; pending work never reported clean |
| F | Shared claim validators | Targets below on fixed baseline plus held-out paraphrase/negation/POV/time controls; lexical-only lane retained; degraded model lane explicit; no FP increase |
| G | Layered summaries and entity/thread retrieval | No cursor/secret leaks; source edit invalidates ancestor summaries; urgent high-id thread beats low-id filler; overflow visible; state retrievable without embedding provider |
| H | Dimension/provider index generations | Exercise 64, 384 and 1536 dimensions with fixtures; wrong dimension/NaN rejected; mixed-generation query impossible; interrupted rebuild/restart/rollback preserve serving index; retrieval quality and scene budgets pass |

### Measurement and release gates

Use the checked-in continuity_recall baseline and unchanged scoring as the
comparison (E12). Final minimums: **≥24/28 (85.7%) overall**, dead_character_acting
≥3/4 (75%), character_attribute ≥3/4, knowledge_leak_before_learned ≥3/4,
world_rule_cost_violation ≥3/4, secret_leak ≥3/4; preserve chronology and quantity
at 4/4 each. That per-type floor totals only 23: overall 24 is an independent
requirement, not implied by the floors. Control false-positive count must stay
≤5 on the same controls, with no new severe false positives. Report counts and
rates per type, not rounded percentages alone.

Run a deterministic lane with frozen typed claim fixtures to isolate comparator
correctness, and an end-to-end extractor lane on the same prose plus a held-out
set. Fixture-lane success alone cannot satisfy the end-to-end release gate.
Keep benchmark labels separate from extractor prompts. Record model/prompt/policy,
seed if supported, extraction failures and variance across three runs. Add held-out
injury persistence, location, possession, promise, transfer, paraphrase, unreliable
speaker, and nested-branch cases; report separately so extending the corpus cannot
hide regression on the original 28. Measure claim coverage/precision separately
from contradiction recall and retrieval relevance.

Proposed initial incremental P2 budget per 1,000–2,000-word scene: one extraction
call plus at most one bounded adjudication call; ≤8,000 input and ≤2,000 output
tokens total; p50 ≤15 seconds, p95 ≤45 seconds; hard 60-second foreground timeout.
Target provider cost ≤US$0.03/scene at the configured model's recorded rates;
local providers report zero billed cost plus tokens and wall time, not “free.”
Warm deterministic validation + projection/retrieval target p95 ≤250 ms on a
100,000-event local fixture; report hardware and cold-start separately. Amortize
chapter/arc/book summarization and embeddings per scene and include them in cost
reports, while reporting their asynchronous latency separately. These budgets
are provisional product targets, not existing measurements.

Extend existing model-call attribution ([V0047__model_call_attribution.sql:1](https://github.com/VerifiedOrganic/spindle/blob/419f40be7378d8919ac89d28139496d06b7db935/crates/spindle-adapters/migrations/V0047__model_call_attribution.sql#L1)) rather than logging prose: record scene,
run, route, prompt/version, packet hash, token counts, billed/estimated cost,
cache hits, retry count and latency. Track extraction, adjudication, summary,
embedding and deterministic stages separately. Evaluate at least 100 scenes for
latency/cost distributions; rate-limit/timeout failures count in completion rate.
Budget overruns produce pending/degraded status and block default rollout until
measured tradeoffs are reviewed. No schema/retrieval improvement may claim recall
or cost gains without a stored reproducible evaluation result in its future PR.

## Open questions for the author

1. Should death, secret disclosure and promise payoff ever auto-ratify? Recommended
   default: no in v1; unattended context carries provisional evidence and pauses
   only when the next checkpoint depends on a disputed high-impact claim.
2. Is the proposed US$0.03 / p95 45-second incremental scene budget acceptable?
   Recommended default: use it as the initial rollout gate, measure locally and
   remotely, and change model/coverage explicitly if it cannot be met.
3. Should branches freeze parent canon at fork? Recommended default: retain live
   parent-chain inheritance, matching current reads (E8); offer frozen branches
   only as a separately designed future feature.

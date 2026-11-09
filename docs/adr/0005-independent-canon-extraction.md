# ADR 0005: independent evidence-bound canon claims

Status: proposed implementation, shadow contracts + append-only event store/reducers + calibrated policy/cutover + shared claim validators + layered memory (canon-memory PRs B–G).

## Decision

The separate `extract` model invocation receives exact accepted UTF-8 scene text,
its hash, stable entity IDs/display terms, and preceding ratified evidence. It
never receives the drafter's package or reasoning. P1 owns the accepted-text
consumer port; P2 implements that port and owns extraction semantics. Frozen
accepted task identity, placement, rating, packet hash and receipt must survive
retries. `IndependentCanonExtractor` implements the exact P1 trait. Its compatibility
result adds an optional typed `canon_audit`; legacy callers omit it, preserving
legacy serialization. A local derived cache atomically records the frozen task
fingerprint and successful report. Successful retries/restarts reuse it; changed
inputs, corrupt reports and unresolved crash locks fail explicitly. Host fallback
completes pending tasks through the same validation and cache. Compatibility
write arrays remain empty. This does not replace the existing canon miner or
activate an authoring cutover.

Core `canon` contracts use version 1 discriminated assertions across ten coverage
families: life/action, location/clock, possession, injury, quantity,
knowledge/disclosure, attribute/world state, relationship, promise, rule/temporal
anchor. Every family reports inspected-none, claims-present, or unknown with a
reason. Unknown is not a successful clean scan. Operations, action mode, modality,
temporal scope and scalar kinds are explicit. Collection members retain their
own identity. Decimal values use checked decimal strings rather than binary
floating-point arithmetic. These are assertions, not validated event effects.

The model supplies proposed claims; the adapter attaches trusted project,
branch, accepted revision/hash, placement and extraction provenance. Every model
claim requires finite confidence in [0,1] and exact nonempty quoted evidence at
UTF-8 byte offsets into that revision. Invalid hashes, offsets/quotes, duplicate
claim IDs, unknown resolved entity IDs, unsupported predicates, inconsistent
coverage or malformed values reject the whole report. Unknown referents remain
explicit unresolved labels; they do not invent existing IDs. Host-supplied
extraction goes through the same acceptance gate. Errors expose codes/paths,
not scene text or provider output.

The prose-clearance chokepoint includes `extract`. V1's shadow engine supports
non-agentic HTTP extraction only, caps output at 2,000 model tokens and bounds
foreground work to 60 seconds. Unsupported backends, missing/uncleared routes,
timeouts, malformed/truncated output and budget failures are explicit pending or
failed work, never empty successful extraction. The input byte cap is a
conservative guard, not a verified tokenizer or a measured cost guarantee.
Provider telemetry reuses the existing model-call attribution path. Raw model
confidence is uncalibrated and never grants authority.

Drafter packages are competing assertions. Normalization preserves their source,
absent evidence and absent extraction confidence, and lists legacy fields it
cannot safely represent. Notes do not establish claim coverage. Comparison keeps
agreed, extractor-only, drafter-only and incompatible groups, including each
source's evidence/provenance. It preserves position, modality, temporal scope and
perspective rather than collapsing belief or flashback into current truth. Case-
sensitive text and units are preserved; decimal spelling can normalize without
loss. Neither agreement nor confidence overrides prior canon.

## Boundaries and consequences

No canonical facts, character state, knowledge, relationships, promises or events
are written by extraction or comparison. Shadow reports must not be converted
into unattended legacy write authority. PR C adds the append-only canon event
store (`V0051`) and pure deterministic reducers (`spindle_core::canon::events`,
`spindle_core::canon::reducer`), while legacy backfill, compatibility shadow
reads, author ratification cutover and policy calibration remain later PRs.
Deaths, secret disclosures and promise payoffs never auto-ratify in v1.

PR E adds the pure, versioned `spindle_core::canon::policy` evaluator and its
SQLite audit/revocation/cutover plumbing. Auto-ratification requires verified
UTF-8 evidence against the accepted revision, resolved entities, no error-level
ratified contradiction, direct present narration, package agreement (or an
explicit operator write) and a class/model/prompt calibration gate. The hard v1
safety list always stages provisional for an author decision. Decision rows
record policy version, confidence inputs, precondition summary, idempotency key
and projection watermark; stale compare-and-append fails safely; revocation
appends withdrawals and invalidates dependents instead of deleting history.
Migration V0054 gates per-project `legacy -> dual_write -> shadow_read ->
canon_read` transitions on compatibility parity/quarantine readiness while
keeping rollback reversible. Legacy-unverified rows participate in compatibility
reads but are warning/info-only until corroborated. Extraction still makes no
default authoring cutover and no MCP tool is added by this policy layer.

PR F adds `spindle_core::canon::claim_validation`: the lexical prefilter is
retained, deterministic lanes cover unkeyed attribute prose drift, paraphrased
knowledge references before the learned position and secret narration by the
nearest out-of-circle actor, and a `SemanticClaimAdjudicator` port reports an
explicit degraded/incomplete status when a route is absent or fails. Negation,
irrealis/POV, dialogue, thin-overlap and time controls are regression fixtures.
The lanes reuse the existing `canonical_fact_prose_drift`, `knowledge_timing`,
`secret_leak` and retcon check types so no public tool schema changes. World-rule
scanner semantics v4 adds plain-verb inflections and explicit cost-omission
promotion while retaining the existing lexical behavior. The continuity recall
corpus/scorer is unchanged; measured results are reported separately.

PR G adds `spindle_core::memory` and migration V0055: scene/chapter/arc/book
derived summaries with coverage, freshness, secret/reader visibility, dependency
DAG invalidation, durable lazy-recompute jobs, budget metadata, and ordered thread
and direct-state entity retrieval. Arc membership defaults to an explicit
unassigned group rather than an invented arc; summaries never establish canon.
`story_so_far` prefers fresh layered chapters before the cursor and retains the
legacy recency path as a fallback. Cursor and secret filtering precede selection,
and stale summaries are not silently treated as fresh.

The versioned meanings above are a protocol boundary. Unknown predicates are
rejected, not preserved as arbitrary JSON; changing a predicate's meaning needs
an explicit schema/version transition. This ADR reserves no migration number.
Any future migration uses the next contiguous version above main at merge time.

Coverage tests and fixture-provider end-to-end tests are not evidence of live
extractor quality. Keep the original continuity corpus/scorer unchanged and
report its per-type before/after counts separately. No default rollout, recall,
quality, cost or latency improvement is claimed by a shadow contract alone.

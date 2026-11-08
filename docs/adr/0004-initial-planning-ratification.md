# ADR 0004: initial planning is a separate ratification family

Status: accepted for Phases A–B (contracts, staging and human-ratified apply).

Initial planning may begin before a project exists. Store premise and intent
hash in `planning_session`, generated operations in `planning_proposal`, and
reserve `planning_operation_receipt` for recoverable application (V0049).
A session binds either no project/branch or an explicit matching pair. No
provisional project is created to display a contract.

The v1 operation vocabulary is the closed `PlanningOperation` enum in core.
Inputs are scoped by their session rather than model-supplied project IDs.
Reference slots distinguish existing IDs from proposal-local output references;
staging requires visible existing records of the correct type or an earlier
operation/exact-hash parent dependency. Unknown operations and input fields fail
closed. No generic tool dispatcher or arbitrary `update_entity` is available.
Initial planning cannot assert reached milestones, delivered consequences,
scene beat realization, or voice evidence from a scene.

Every session starts with human gates. The recommended future profile keeps
contract, cast/world, series/arc and book shape human-approved, with chapter
and scene acceptance delegated only by a separate explicit author decision.
Delegated application must be a second audited transition after staging, using
`policy:<id>:<version>` and a persisted policy snapshot. Merely receiving model
output, passing validation, or selecting this profile in a prompt grants no
permission. Holds and unresolved author decisions must block automation.

Phase B enables human acceptance through the service-level apply path. There
are still no MCP planning entry tools or creative model dispatches. All gates
remain human; policy activation/delegation is not shipped by this foundation.
The staging repository's decide method remains reject/supersede-only, so callers
cannot bypass service preflight or the atomic operation boundary.

The selected dependency closure first executes existing service paths in a
rollback-only SQLite transaction. Valid input then applies sequentially, one
operation per transaction. The domain writes, returned IDs, actor/note/time,
prior state and operation receipt commit together. Root attachment (including
initial book/chapter ID aliases) shares that commit. Successful receipts are
immutable: retries read them instead of repeating writes, even after process
interruption before proposal finalization. A failure reports applied, failed
and not-reached operations; it never rolls back previously accepted operations.
Partially applied proposals cannot be rejected or superseded as unwritten.

Application transactions pin async service readbacks to the writer, nest the
planning-used repository write paths through savepoints, and exclude unrelated
writers with an async gate. Cancellation retains the gate until rollback. A
lost transaction must fail closed rather than allowing autocommit writes.
Semantic-index refreshes are queued in V0050 and processed after domain commit;
pending refreshes are visible separately from domain application success.

Project-wide contract changes and book allocation respect active runs on any
branch; branch-local roots respect the selected branch.

Base snapshots include reader contract, referenced records and mutation targets.
Unknown refs acquire snapshots when approved parent IDs become available. Known
snapshots are never silently refreshed from arbitrary bible changes. Verified
book allocations can advance later allocations in the same approved proposal or
an exact-hash dependent proposal; an accepted contract change can advance its
dependent context snapshot. Other external changes require a new proposal.

Decisions require exact proposal hashes and session scope. Batch preflight and
updates share a serialized writer transaction. Rejection or supersession
supersedes all staged descendants atomically; decided history is preserved.
Edits use replacement proposals after explicit supersession rather than
mutating an artifact under an existing hash. Listings return persisted artifacts
and audit actor, note and time. Readiness is not inferred from these records.

This permission applies only to initial unwritten intent. ADR 0001 canon deltas
and ADR 0003 amendments retain their existing classes, human decisions and
policies. In-run amendments never auto-accept. Domain apply reuses existing
service paths, revalidates hashes/branch/prose guards, and records sequential
progress without claiming one atomic transaction across the selected batch.

The additions are opt-in and additive. Existing runs and clients remain
unchanged. Remaining phases add generation receipts, schedule
validation, living-outline reconciliation and entry tools before release.

# ADR 0004: initial planning is a separate ratification family

Status: accepted for Phase A (contracts and staging).

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

Phase A exposes service/repository staging, reads, rejection and supersession
only; no MCP tools or model dispatch are added. Acceptance deliberately returns
an unavailable error and leaves proposals staged until Phase B can commit domain
writes and durable operation receipts together. No row is called applied just
because an author approved it. Policy actors are rejected at this boundary.
Thus Phase A's decision round-trip tests exercise rejection/supersession;
successful acceptance and crash-recovery tests belong to Phase B.

Decisions require exact proposal hashes and session scope. Batch preflight and
updates share a serialized writer transaction. Rejection or supersession
supersedes all staged descendants atomically; decided history is preserved.
Edits use replacement proposals after explicit supersession rather than
mutating an artifact under an existing hash. Listings return persisted artifacts
and audit actor, note and time. Readiness is not inferred from these records.

This permission applies only to initial unwritten intent. ADR 0001 canon deltas
and ADR 0003 amendments retain their existing classes, human decisions and
policies. In-run amendments never auto-accept. Domain apply will reuse existing
service paths, revalidate hashes/branch/prose guards, and record sequential
progress without claiming a transaction spanning independent service calls.

The additions are opt-in and additive. Existing runs and clients remain
unchanged. Remaining phases add atomic apply, generation receipts, schedule
validation, living-outline reconciliation and entry tools before release.

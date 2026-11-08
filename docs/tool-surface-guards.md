# Schema and discovery guards

The guard infrastructure inspects the finalized public `Tool` values after the
input and output sanitizers, with no runtime schema mutation. It preserves the
existing strict-client tests and all `tools/list` snapshots.

`cargo test -p spindle-mcp quality` covers:

- `tool_schema_lint`: validates every emitted input/output schema against its
  JSON-Schema meta-schema, checks profile schema equality, and records current
  compatibility-catalog descriptive debt with tool names and JSON pointers.
- Description checks on every nested/optional property, including local `$ref`
  targets. The walker follows schema keywords, visits distinct pointer nodes,
  escapes property names and terminates on cycles. Unresolved/external refs are
  diagnostic findings; the validator has HTTP/file retrieval features disabled.
- Empty and field-name-only descriptions, emitted implementation jargon and the
  known provider-specific research promise. String fields named `mode`, `status`,
  `action`, or ending in `_policy` without enums are review candidates, not proof
  that those fields should have closed vocabularies.
- `quality/closed-fields.json` explicitly identifies the currently closed
  authoring mode and policy fields; missing schema fields/enums are findings.
  Rust enum conversion and serialized-value parser round trips belong with the
  later contract cleanup, not guessed from these heuristics.
- `profile_token_budgets`: checks explicit current profile ceilings. Synthetic
  tests independently reject oversized inputs, descriptions and outputs. The
  strict canonical helper has no exemption path and rejects per-tool discovery
  over 4,096 bytes as well as descriptive lint.
- `http_profile_discovery_matches_registry_budgets`: initializes a real loopback
  HTTP MCP session for each current profile using the shared server handler and
  asserts identical ordered tool values and byte measurements. Requests have
  timeouts; session cancellation and a task guard clean up on every exit.

`tools/profile-budgets.json` is shared by Rust and
`tools/profile_snapshot_probe.py`. It separates the **current compatibility
ceilings** from the **future canonical acceptance ceilings** in the design.
Compact UTF-8 tool-array bytes are authoritative; token estimates are
`ceil(bytes / 4)`. Input-only measurement includes name, description and input
schema. Full measurement includes advertised outputs. The live isolated stdio
probe checks the same ceilings plus exact registry snapshots and per-tool growth
on mismatch. The live probe also asserts unknown startup profiles are rejected before data
access; direct unknown-profile routers fail closed.

The current long-form catalogs are not lint-clean and do not meet canonical size
targets. `quality/legacy-schema-inventory.json` is a generated diagnostic
inventory of their findings, not a waiver accepted by the strict canonical
helper. Normal tests compare it exactly, so new, moved, or removed findings need
review. No active canonical profiles or clean-catalog claim are introduced here.
The inventory is owned by P4, must be reviewed during R rollout and retired by
R+1 with the compatibility catalogs. Do not copy its findings into canonical
exemptions. Canonical profile registration must run the strict helper and target
budget checks when those profiles ship.

After an intentional compatibility schema change, regenerate with
`SPINDLE_UPDATE_SCHEMA_INVENTORY=1 cargo test -p spindle-mcp tool_schema_lint`,
inspect the pointer/rule diff, and rerun without the update setting. This does
not update discovery snapshots or authorize a budget increase. Intentional
public surface changes still require same-PR snapshot updates and a changelog.
The target caps are not claims of implemented reduction. Generated transitional
profiles now have measured compatibility ceilings; canonical descriptions/enums,
narrowed skill cores and compact discovery remain upcoming P4 work.

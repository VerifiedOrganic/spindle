# Optional full response contracts

Generated profiles omit `outputSchema` for 19 explicitly selected large context,
analysis, alternatives and style-card results. Actual JSON results, input types,
required/defaulting rules and service validation are unchanged. Scene save,
revision, commit, authoring-save and import-start receipt schemas stay inline.
Unset/default, the four original profiles and `legacy-all` retain their full
original tool descriptors byte-for-byte.

Each omitted schema has a `spindle/responseContract` tool metadata entry and a
short description link. Load it only when a client needs the full response shape:

- Resource index: `bible://contracts/tools/v1`.
- Full JSON Schema: `bible://contracts/tools/v1/get_scene_context`.
- Tool-only index: `get_reference(name="contracts/tools/v1")`.
- Tool-only full schema: `get_reference(name="contracts/tools/v1/get_scene_context")`.

`list_skills` includes the reference index, and resource discovery includes the
index plus a named template. It does not preload every schema. Resources return
JSON (individual schemas use `application/schema+json`); the existing reference
output shape returns one Markdown document containing the full schema. Contract
references are schema documentation, not skill workflow units or access grants.
Unknown versions, names and traversal-shaped names fail without project reads.

V1 describes the current result envelopes. The full schemas come directly from
the cached typed legacy descriptors after numeric-format/null-enum sanitation;
there is no hand-maintained stub. Existing schema limitations remain visible in
the legacy diagnostic inventory. Backwards-compatible documentation/additions
may refine v1; a breaking result-envelope change requires a new contract major
version and the planned compatibility adapter. Merely omitting discovery schema
metadata never changes a result envelope.

The projection is cached for direct routers and idempotent for secondary proxies.
Receipts retain their schemas, all non-description input keywords remain exact,
and other tool metadata stays intact. Generated `research_query` describes the
configured research route; compatibility discovery keeps its frozen copy. The
get-reference input description advertises the optional contract namespace.

Input discovery grows slightly to explain the optional loading routes; full
UTF-8 discovery shrinks substantially. Measured generated-profile caps are
updated in `tools/profile-budgets.json`; compatibility and future canonical
acceptance ceilings are unchanged. These profiles are still transitional:
large inputs, inline receipts, broad entry dependencies and missing descriptions
remain. This PR does not claim canonical byte ceilings or lint cleanliness.

Tests validate all 19 full schemas, resource/tool-only loading and invalid names,
projection idempotency and input preservation, an actual unchanged structured
location read against its schema, proxy descriptor parity, HTTP/stdio snapshots,
and receipt preservation. Optional contract reads expose schema metadata only;
they perform no model calls and contain no project data.

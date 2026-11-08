# Drafting packet rollout

### Packet rollout boundary

The typed `DraftingPacketV1` foundation is currently an internal preparation API,
not an active authoring-run mode or a new MCP tool. Keep using the shipped
save/commit continuity-package workflow. Future packet prose tasks will contain
their own compact rubric; do not append this skill, standards manuals or tool
instructions to a stateless packet completion. Do not claim `critique_v1` is
available until the authoring integration and evaluation gates ship.

PR 2 adds internal opt-in Rust packet-draft APIs and a P2 extractor consumer
contract. It does not add an authoring tool or enable a run policy. A verified
model tokenizer is required; unknown framing stops before dispatch. Plain-text
artifacts preserve exact bytes and receipt provenance. Truncated drafts cannot
be accepted. Accepted-text extraction failures keep prose staged for a retry;
they never request another draft. Continue using the public legacy package
workflow until host/agent integration ships.

# Offloaded Generation and Explicit-Rating Receipts

Load this reference when drafting `explicit` sexual prose, checking `bible://config/routing` for `caller_should_send_brief` / `adapter_kind`, or offloading generation to a configured server-side model route.

## 1. Explicit-Rating Origin Gate and Receipts

`save_scene_draft` validates the declared `content_rating` (`general`, `teen`, `mature`, `explicit`) and enforces a strict origin gate on `explicit` sexual prose:

1. **Client-side restriction**: Never generate `explicit` sexual prose directly in the host assistant response. `save_scene_draft` rejects client-authored explicit sexual prose unless accompanied by a valid server-held `generation_id`. Write only non-explicit framing client-side.
2. **Check route configuration**: Read `bible://config/routing` to confirm a `draft` rule with `rating: "explicit"` exists, and inspect `list_agents` to verify an agent declares `ratings` covering `"explicit"`. If no explicit route is configured when the author expects offload, surface that configuration gap instead of silently falling back to a default model.
3. **Generate via server route**: Call `continue_generation` with `route: "draft"` and `rating: "explicit"`. Keep the returned `generation_id` receipt; for continuations, the receipt tracks accumulated text (`prior_output + output`).
4. **Revise explicit prose**: For continuity, voice, or style edits inside explicit prose, call `revise_generation` with the source `generation_id` and edit instructions; it routes through the explicit-cleared draft agent and returns a new `generation_id`.
5. **Save with receipt**: Pass the `generation_id` to `save_scene_draft`; Spindle persists the server-held generated text as the scene's `full_text`.

## 2. Prompt Strategy: Brief vs. Packed (`caller_should_send_brief`)

Every routing rule in `bible://config/routing` exposes `caller_should_send_brief: bool` and `adapter_kind`. Check this flag for the exact `(route, rating)` pair before calling `continue_generation` or `revise_generation`:

- **`caller_should_send_brief: true`** (MCP-capable agent adapter, e.g. `adapter_kind: "grok"` or CLI-with-MCP):
  - The spawned agent pulls Story Bible canon on demand over MCP and receives a Spindle Context block with project/book/chapter/scene IDs.
  - Send a **short brief** (under ~2k tokens): scene intent, beats to hit, POV/voice notes, and record IDs for canon the agent should fetch (character voice IDs, prior-scene IDs, world rules).
  - Do **not** inline chapter briefings, character snapshots, or prior-scene full text.
- **`caller_should_send_brief: false`** (stateless HTTP/local/raw-CLI endpoint without MCP access):
  - The model cannot fetch Bible state itself.
  - **Pre-pack** all required context in the prompt: chapter briefing, character snapshots and voice profiles, prior scene text or summaries, applicable world rules, and scene intent.

<!-- tool: continue_generation -->
<!-- tool: list_agents -->
<!-- tool: revise_generation -->
<!-- tool: save_scene_draft -->

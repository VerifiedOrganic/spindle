# Spindle agent configuration

This document describes the shipped model-agent configuration system. It covers
how Spindle loads `spindle.toml`, how configured agents map onto the existing
model routes, and which MCP tools and resources let you inspect or reload that
state at runtime.

<!-- prettier-ignore -->
> [!IMPORTANT]
> This implementation configures the existing route-based model runtime. It
> does not implement the older handoff pipeline concepts such as
> `create_handoff`, `dispatch_handoff`, `submit_handoff_result`,
> `merge_handoff`, or `compute_content_boundaries`.

## What this system does

Spindle already uses logical model routes such as `draft`, `review`,
`import_extract`, `import_synthesize`, and `import_validate`. The new config
system lets you replace the default local route bindings with named external or
local HTTP agents without changing the rest of the service layer.

The current implementation lets you:

- define named agents in `spindle.toml`
- assign shipped route names to those agents
- reload the config at runtime with MCP
- inspect sanitized agent and route state through MCP resources
- test a configured agent through the same route machinery the server uses

When no config file exists, Spindle keeps the existing built-in local route
defaults.

For `provider = "cli"`, `endpoint` is the executable name or path for that
specific agent. Completion passes the route and prompt as two arguments.
Configured endpoints take precedence over `SPINDLE_MODEL_CLI_COMMAND`; that
environment variable remains a fallback for legacy CLI routes without an
endpoint. Different rated agents therefore dispatch different executables.

## Config file locations

Spindle resolves agent config in this order:

1. `SPINDLE_CONFIG`, if set
2. the nearest project-local `.spindle/config.toml` found by walking up from
   the current working directory
3. `./spindle.toml` in the current working directory
4. `~/.spindle/config.toml`

If none of those files exist, Spindle starts with the default built-in routes.

## Config shape

The shipped parser currently supports `[[agents]]` and `[[routing]]`.

```toml
[[agents]]
id = "local-http"
name = "Local HTTP model"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"
api_key_env = "OPENAI_API_KEY"
max_context = 32000
ratings = ["safe", "mature", "explicit"]
quality_tier = "primary"
capabilities = ["system_prompt"]
notes = "Local or proxy OpenAI-compatible endpoint"

[[routing]]
route = "draft"
agent = "local-http"
purpose = "scene drafting and alternative synthesis"
system_prompt = "You are a fiction drafting agent."

[[routing]]
route = "review"
agent = "local-http"

[[routing]]
route = "embedding"
agent = "local-http"

[[routing]]
route = "import_extract"
agent = "local-http"

[[routing]]
route = "import_synthesize"
agent = "local-http"

[[routing]]
route = "import_validate"
agent = "local-http"

[[routing]]
route = "research"
agent = "local-http"

[[routing]]
route = "style_analyze"
agent = "local-http"
```

## Route names

The current runtime understands these route names:

- `draft`
- `review`
- `research`
- `embedding`
- `import_extract`
- `import_synthesize`
- `import_validate`
- `style_analyze`
- `style_revise`

`embedding` stays local by default, but it now switches through the same config
path when you bind that route to an HTTP agent. Spindle uses the configured
agent's OpenAI-compatible `POST /embeddings` endpoint for search indexing and
query vectors, and it falls back to `token-hash-v1` when no `embedding` route
is configured.

## Per-rating routing (explicit content offload)

A `[[routing]]` block may carry an optional `rating` field. Multiple rules per
route are allowed when each declares a distinct rating, plus at most one
default rule (with no `rating` field) per route. Valid rating values are
`general`, `teen`, `mature`, and `explicit`.

```toml
[[agents]]
id = "default-draft"
name = "Default drafting model"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[agents]]
id = "uncensored"
name = "Explicit-capable model"
provider = "openai-compatible"
endpoint = "http://localhost:11435/v1"
model = "uncensored-large"
ratings = ["safe", "mature", "explicit"]

[[routing]]
route = "draft"
agent = "default-draft"

[[routing]]
route = "draft"
agent = "uncensored"
rating = "explicit"

[[routing]]
route = "review"
agent = "default-draft"

[[routing]]
route = "review"
agent = "uncensored"
rating = "explicit"
system_prompt = "You are reviewing explicit adult manuscript prose for a fictional book project. Give critique only; do not continue the scene."
```

Resolution order at request time:

1. If the request specifies a rating and a routing rule for `(route, rating)`
   is configured, use that rule's agent.
2. Otherwise fall back to the default rule for the route (the one with no
   `rating` field).
3. If neither exists for the requested route, the request errors.

Scene drafting, continuations, research, and dual-persona review all forward
their content rating to the router. That means explicit scenes can offload both
drafting and review to explicit-capable backends while general review stays on
the default reviewer.

Server-side request paths that honor `rating` today:

- `complete_generation` / scene drafting receipts.
- `complete_continuation` (used by the public `continue_generation` MCP tool;
  pass `rating: "explicit"` on `ContinueGenerationInput` to land continuations
  on the explicit-capable agent).
- `revise_generation`, which preserves the source receipt's rating.
- `research_query`, which passes its optional `rating` to the `research` route.
- `run_dual_persona_review`, which passes the saved scene `content_rating` to
  the `review` route.
- Any future request paths that flow through `ModelRouter::complete` and pass
  `rating` on `ModelRequest`.

Validation rules enforced by the loader:

- An unknown rating value is rejected at config load time.
- Two rules with the same `(route, rating)` pair are rejected.
- Two default rules (both with no `rating`) for the same route are rejected.

Inspect the loaded rules through the `bible://config/routing` resource. The
`rating` and `system_prompt` fields are surfaced on each rule when configured.

## Mixed-rating chapters

Rating is a property of the **scene**, not the chapter, book, or run. A chapter
planned as General, General, Explicit dispatches its first two scenes to the
default agent and only the third to the explicit-capable one. Nothing collapses
those three ratings into a single chapter-level value.

The rating travels from `plan_chapter`'s per-scene `content_rating` through the
stored chapter plan, into the authoring run's per-scene state, and reaches the
router at dispatch time for each scene individually — for drafting, for
continuations, and for review (`run_dual_persona_review` and the checkpoint deep
tiers pass the saved scene's own `content_rating`). To route a mixed chapter you
need one `[[routing]]` default rule plus one `rating = "explicit"` override, as
in the example above; nothing per-chapter is required.

### Preflight over the planned rating set

`authoring_prepare_run` / `authoring_start_run` compute the set of **distinct**
ratings across every planned scene in the chapter range, then check route
coverage once per rating. So:

- Adding one Explicit scene to an otherwise-General chapter **does** require an
  explicit-capable `draft` route; the run blocks without one, and the reported
  `missing_requirements` entry names `explicit`.
- It does **not** require the General scenes to be servable by that explicit
  agent, and it does not report the covered ratings as gaps.

The same per-rating pass runs for the `mine` route (under
`mining_policy: "propose_all"`) and the `review` route (under the auto checkpoint
policies).

### Context does not cross the rating boundary

Per-scene routing would be hollow if the prompt smuggled the prose across
anyway. Drafting a General scene that follows an Explicit one assembles context —
including the previous scene's closing prose — and dispatches it to the *General*
route, i.e. an agent the operator never cleared for explicit content.

So `get_scene_context` elides across that boundary: when the preceding scene is
Explicit and the scene being drafted is not, `previous_scene_tail.excerpt`
carries the neighbour's stored **summary** instead of its prose, and
`elided_reason` explains the substitution. The markdown rendering labels it
rather than presenting it as closing lines:

```
## PREVIOUS SCENE (closing)
Ch 1.2 [prose withheld — previous scene is explicit; this scene is rated
general and drafts on a route that may not be cleared for explicit content,
so its closing prose is replaced by the scene summary]
Summary: Mara and the smith finally give in.
```

Elision is a rating boundary, not a blanket filter — an Explicit scene following
an Explicit one still receives the real closing prose, because it routes to the
cleared agent. The policy **fails closed**: if the target scene has no planned
rating and has not been drafted (so its clearance cannot be established), the
explicit neighbour is elided rather than gambled. Planning your scenes' ratings
is what restores the full hand-off.

## Import routes and content

The manuscript-import routes (`import_extract`, `import_synthesize`) carry your
full manuscript prose, but they are **not rating-gated**. The rating gate cannot
protect you here: content ratings are assigned by *analysis*, which runs during
and after import, so at import time there is no rating to gate on. Import is also
a direct operator action on your own manuscript — you point Spindle at your text
and choose which agents run the import.

Because the gate does not apply, the obligation is **informed configuration**: if
your manuscript contains explicit content, configure `import_extract` and
`import_synthesize` agents you trust with it end to end — for example, the same
explicit-cleared local agents you use for drafting. An import agent that is not
cleared for explicit content will still see your full explicit manuscript; the
gate will not stop it.

If you offload explicit work to a separate agent and want import to follow that
offload without wiring each import route by hand, set the top-level flag:

```toml
route_import_to_explicit = true
```

With it enabled, `import_extract` and `import_synthesize` resolve to your
explicit-cleared offload agent — the agent that serves `draft` at the `explicit`
rating (or an `import_*`-specific `rating = "explicit"` override if you defined
one) — instead of their default chair. This keeps un-rated manuscript prose off a
default agent whose provider safety classifier would refuse it (e.g. Qwen). It is
opt-in: a deliberately configured import chair is never overridden unless you set
the flag. If no explicit-cleared agent is configured, import falls back to its
default chair.

As a nudge, `configure_agents` emits an advisory warning when an import route's
agent does **not** declare the `explicit` rating while another configured agent
does — a heuristic sign that you work with explicit content somewhere but your
import chair is not cleared for it. It is advisory, not an error: import routes
are not rating-gated, so ensure the agent may see your full manuscript (or set
`route_import_to_explicit = true`). The advisory is suppressed when the flag is
enabled.

## Research routing

`route = "research"` is the first-class route for agent-assisted factual
research. Research agents should return structured source/note/claim JSON and
must not draft story prose. Use a default research route for ordinary setting,
technical, historical, and social research. Add a rating-specific override for
adult/explicit factual research so those queries do not fall through to the
general research model.

```toml
[[agents]]
id = "general-research"
name = "General research model"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "research-model"
api_key_env = "RESEARCH_API_KEY"

[[agents]]
id = "explicit-research"
name = "Explicit/adult factual research model"
provider = "grok-cli"
endpoint = "grok"
model = "grok-build"
ratings = ["safe", "mature", "explicit"]
agent_profile = "spindle-researcher"

[[routing]]
route = "research"
agent = "general-research"

[[routing]]
route = "research"
agent = "explicit-research"
rating = "explicit"
```

When `research_query` is called with `rating = "explicit"`, Spindle requires
the explicit research override. This is stricter than generic model routing:
explicit research should never silently use the unrated research route.

## Startup behavior

At startup, `spindle-mcp` now:

1. resolves the config path
2. loads `spindle.toml` if it exists
3. validates agent IDs and route assignments
4. resolves API keys from `api_key_env`
5. updates the in-memory `ModelRouter`

If config loading fails, startup fails with a configuration error. If the file
is absent, Spindle falls back to the shipped local routes.

## Runtime behavior

Configured generation routes use an OpenAI-compatible `POST /chat/completions`
request. Spindle sends:

- the configured model name
- `system_prompt` as the system message when the routing rule sets it
- otherwise, a short system message derived from the route `purpose`
- the user prompt generated by the existing service logic

`capabilities = ["system_prompt"]` is descriptive agent metadata. It does not
change request construction by itself; the actual field that controls the chat
system message is `[[routing]].system_prompt`.

For `route = "draft"` requests with `rating = "explicit"`, Spindle also appends
a built-in explicit-rating drafting directive to the system message. The
directive tells the external drafting model to keep requested adult sexual
material on page, avoid fade-to-black/euphemized handling, and preserve consent,
adult age, continuity, character voice, and story tone. Use
`[[routing]].system_prompt` for project-specific taste and voice; the built-in
directive is the default floor.

## Grok CLI provider (`provider = "grok-cli"`)

The `grok-cli` provider routes a logical route — typically the
explicit-rated `draft` rule — to the locally-installed `grok` CLI running in
single-turn headless mode. Unlike OpenAI-compatible HTTP agents, grok spawns
as a child process with full agentic loop, its own MCP session, and access to
spindle's own MCP server. That last part is the win: grok pulls the canon it
needs on demand (`search_bible`, `get_scene_context`, `get_character_snapshot`,
`get_entity`, `get_chapter_briefing`, …) instead of every request hauling a
pre-packed mega-prompt over the wire.

### Prerequisites

1. The `grok` CLI must be installed and on the spindle-mcp process's `PATH`
   (or addressable by absolute path). The binary lives at
   `~/.grok/bin/grok` after a standard install.
2. Spindle's MCP server must be reachable from grok. Grok loads MCP servers
   from `~/.claude.json`, `~/.grok/config.toml`, and project-scoped
   `.grok/config.toml` / `.mcp.json` files walking up from the working
   directory. The easiest one-time setup is to add a `spindle` entry to
   `~/.claude.json`:

   ```jsonc
   {
     "mcpServers": {
       "spindle": { "command": "/absolute/path/to/spindle-mcp" }
     }
   }
   ```

3. Recommended: run `init_grok_skills` so the
   `bible://skills/scene-writer` (and friends) are installed as `~/.grok/skills/`
   adapters — grok's autoloader picks them up when the prompt looks
   spindle-flavored.

### TOML shape

```toml
[[agents]]
id = "grok-local"
name = "Local Grok CLI"
provider = "grok-cli"
# `endpoint` is the grok binary name or absolute path, not an HTTP URL.
endpoint = "grok"
# `model` is forwarded to grok via --model; leave it as your preferred grok
# variant. The string is opaque to spindle.
model = "grok-4"
ratings = ["safe", "mature", "explicit"]
# Grok-specific knobs (all optional; defaults shown in brackets):
effort = "high"                           # → --effort  [unset]
max_turns = 250                           # → --max-turns  [200]
agent_profile = "spindle-scene-writer"    # → --agent  [unset]
working_directory = "/path/to/project"    # → --cwd  [spindle-mcp's cwd]
allow_tools = ["mcp__spindle__search_bible"]   # additive --allow rules
deny_tools  = ["mcp__spindle__delete_scene"]   # additive --deny rules
extra_args  = ["--check"]                 # escape hatch, appended verbatim

[[routing]]
route = "draft"
agent = "grok-local"
rating = "explicit"
```

### What spindle sends grok

For each request, spindle spawns:

```text
grok \
  --prompt-file <tempfile>            # the rendered prompt (verbatim)
  --output-format json                # parseable envelope
  --always-approve                    # required for non-interactive runs
  --max-turns <N>                     # default 200, overridable per-agent
  --system-prompt-override <...>      # composed system prompt (see below)
  --model <model>                     # forwarded from agent.model
  [--agent <agent_profile>]
  [--effort <effort>]
  [--cwd <working_directory>]
  [--allow <rule>] ... (repeated)
  [--deny  <rule>] ... (repeated)
  [extra_args...]
```

The system prompt grok receives is layered:

1. The route's base `system_prompt`.
2. The explicit-rating drafting directive (for `route = "draft"` +
   `rating = "explicit"`), same one the HTTP path receives.
3. A grok-specific MCP usage hint instructing it to call
   `set_active_project` **first**, then use the read tools as needed.
4. A structured **Spindle Context** block listing the
   `project_id` / `book_id` / `chapter_id` / `scene_id` for this request.

### Bootstrap: `set_active_project` is required

The spawned grok process runs in its **own** MCP session and does not
inherit the caller's `active_project_id`. The bootstrap is what makes any
subsequent spindle MCP call resolve to the right project's bible.

To make this work, `continue_generation` now accepts optional
`project_id` / `book_id` / `chapter_id` / `scene_id` on
`ContinueGenerationInput`. When omitted, the MCP layer auto-fills
`project_id` from the calling session's active project (same fallback the
read tools already use). Pass the IDs explicitly when you're drafting
for a project other than the active one. If no project_id can be resolved,
the spawned grok session will error on its first bible call with the
standard "this MCP session has no active project" message.

### Long-scene drafting and `max_turns`

Default `--max-turns` is **200**. Grok consumes ~6 messages even for a
trivial reply (system reminders, reasoning, response), and a real explicit
scene that pulls bible canon and spans multiple output continuations can
legitimately use 50–150 messages. The 200 default leaves headroom; bump
`max_turns` on the agent if you see drafts truncated by the max-turns
guard (grok exits non-zero with `Internal error: "max_turns exceeded: …"`
in the JSON envelope and spindle surfaces that error verbatim).

### Continuations

Grok's headless `-p` mode has no native concept of resuming a prior
assistant message, so spindle handles continuations by embedding the
prior partial output into the prompt body with an explicit "continue
where you left off" instruction. The same `--max-turns`, system prompt,
and MCP wiring apply.

### Output envelope

Grok writes one JSON object to stdout under `--output-format json`:

```json
{
  "text": "<final assistant message>",
  "stopReason": "EndTurn",
  "sessionId": "...",
  "requestId": "...",
  "thought": "<chain-of-thought, may be very long>"
}
```

Spindle uses `.text` as the output, treats any `.stopReason != "EndTurn"`
as `truncated = true`, and ignores `.thought` on the happy path. On
failure grok emits `{"type":"error","message":"…"}` (and exits non-zero);
spindle surfaces the `message` verbatim. Grok's tracing logs go straight
to spindle-mcp's stderr in real time so long agentic loops don't feel
hung.

### Security and tool scoping

Use `allow_tools` / `deny_tools` to scope which MCP tools grok may call.
A reasonable explicit-draft default is to allow only the bible-reading
tools and deny every write tool; spindle persists grok's final prose
through its own draft-receipt pipeline (`continue_generation` → server
output verification → `save_scene_draft` with the receipt's
`generation_id`). The system prompt also explicitly tells grok not to
invoke any write tools.

## Explicit draft origin enforcement

`continue_generation` records a short-lived server-side generation receipt for
the completed output (`prior_output + output`), including the resolved route,
rating, agent id, and output hash. When `save_scene_draft` receives explicit
sexual prose (`content_rating: "explicit"` plus explicit sexual language), it
rejects the save unless the caller passes that receipt's `generation_id`. The
receipt must be from:

- `route: "draft"`
- `rating: "explicit"`
- an agent whose `ratings` includes `"explicit"`

The receipt is **provenance only**. The caller's `full_text` is authoritative
and is what Spindle persists; the receipt proves the save was authorized by a
cleared, explicit-capable draft generation and supplies the `agent:{id}` stamp.
Spindle does not substitute the receipt's output for your prose — in practice a
receipt carries agent narration and truncated turn fragments alongside the
prose, so treating it as the source of truth silently destroyed real drafts.

**One receipt authorizes one scene.** The first explicit save presenting a
receipt binds it to that scene's `(book, chapter, scene_order)` placement.
Re-saving the *same* scene with the same `generation_id` stays legal, so retries
and re-saves work. Presenting it for a *different* scene is rejected:

```
generation_id "model_generation:7:a1b2c3d4e5f6" already authorized a different
scene (book 1, chapter 1, scene 3); each explicit save needs its own receipt
```

This matters most in mixed-rating chapters (below): without it, a single trip
through the explicit-capable agent would blanket-authorize explicit saves across
every neighbouring scene, each stamped `agent:{id}` as though that agent had
written it.

This is a hard server-side gate. Explicit sexual scene text must be produced
through the explicit draft route and saved with the returned `generation_id`.
Minor cleanup of *any* generated draft — explicit, mature, teen, or general —
should use `revise_generation`, which sends the server-held generation text
plus edit instructions back through the **same draft route the source
receipt used** (preserving its rating routing) and returns a new receipt.
Save the revised receipt id. This lets you make surgical edits without a
full `continue_generation` re-roll regardless of rating.

Saved scenes record `draft_origin` as `client` or `agent:{agent_id}` so later
inspection can distinguish client-authored prose from server-side explicit
route output.

When an explicit draft route is configured, `get_scene_context` also injects a
hard constraint reminding the caller to use `continue_generation` with
`route: "draft"` and `rating: "explicit"`.

Provider-specific headers are handled for:

- `openrouter`
- `anthropic`
- all other providers through standard bearer auth when `api_key_env` resolves

If no API key is required, such as some local servers, the request is sent
without auth headers.

When `embedding` is configured, Spindle instead uses OpenAI-compatible
`POST /embeddings` requests with the configured model name and normalizes the
returned vector for cosine scoring.

When `[health_check].enabled = true`, Spindle also keeps agent reachability
fresh after startup with a background heartbeat. The refresh interval is
controlled by `SPINDLE_HEALTH_CHECK_INTERVAL_MS`; if unset it defaults to 10
minutes, and `0` disables the periodic refresh. Each heartbeat emits tracing
logs summarizing checked/unhealthy agents and logs reachability transitions
when a configured agent changes status.

## MCP tools

The following tools are now implemented.

### `configure_agents`

Reload the config file into the runtime router.

Input:

```json
{
  "config_path": "/absolute/path/to/spindle.toml"
}
```

If `config_path` is omitted, Spindle uses the normal config resolution order.

### `list_agents`

Return the sanitized configured agent list, including route assignments and
status such as `active` or `missing_api_key`.

### `test_agent`

Run a short prompt through a configured agent using one of its assigned routes.

Input:

```json
{
  "agent_id": "local-http",
  "test_prompt": "Write two short lines that confirm the model is reachable."
}
```

## MCP resources

The following resources are now implemented.

- `bible://config/agents`
- `bible://config/routing`
- `bible://system/model-routes`

`bible://config/agents` exposes sanitized agent metadata only. It never returns
resolved API keys.

Each routing rule (`bible://config/routing`) and resolved route
(`bible://system/model-routes`) carries two fields callers use to pick a
prompt strategy:

- `adapter_kind` — the resolved adapter (`http`, `grok`, `cli`, `local`).
- `caller_should_send_brief` — `true` when the adapter runs a child process
  that has its own MCP access back to spindle (currently `adapter_kind ==
  "grok"`). Callers building prompts should send a short brief instead of
  pre-packing canon, because the spawned agent pulls bible context on
  demand. `false` for stateless adapters where the caller must pre-pack
  everything. The scene-writer skill checks this flag before building its
  `continue_generation` prompt.

When a future CLI-with-MCP adapter is added (e.g. Codex-CLI), it should
also report `caller_should_send_brief = true` so every existing caller
automatically uses the brief shape without further code changes.

## Validation rules

The loader currently enforces these rules:

- every agent must have a unique `id`
- every agent must define `endpoint` and `model`
- every routing rule must define a unique `route`
- every routing rule must reference a known `agent`
- every `fallback`, if present, must reference a known `agent`
- `effort`, when set, must be one of `low | medium | high | xhigh | max`
  (matches grok CLI's accepted values)
- `max_turns`, when set, must be `>= 1`
- grok-cli-only fields (`effort`, `max_turns`, `agent_profile`,
  `working_directory`, `allow_tools`, `deny_tools`, `extra_args`) may only
  appear on agents whose `provider = "grok-cli"`. They are rejected at
  load time on other providers so misconfigs surface immediately rather
  than silently being ignored at request time.

## Minimal example

This is the smallest useful config for replacing the default `draft` route.

```toml
[[agents]]
id = "draft-http"
name = "Draft model"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[routing]]
route = "draft"
agent = "draft-http"
```

## Operational notes

This implementation keeps the current architecture boundaries intact.

- `spindle-core` owns the public DTOs for config tools and resources
- `spindle-adapters` owns TOML loading, validation, and the runtime router
- `spindle-mcp` owns the config tools, config resources, and startup reload

The active source of truth is the resolved `spindle.toml` file plus environment variables used for API key resolution.

## Privacy & Routing Implications for style_analyze and style_revise

The `style_analyze` and `style_revise` routes handle processing of user-provided drafting corpora and target prose. Because these may contain sensitive or copyrighted material, Spindle provides specific privacy controls:

1. **`metrics_only` Mode**: When creating a style profile, if `metrics_only` is set to `true`, Spindle completely strips all raw prose chunks from the synthesis prompt. The prompt only includes deterministic statistics (average sentence length, dialogue ratios, punctuation rates) and high-level structure metadata. This prevents any source text from leaking to the model.
2. **Local vs. External Routing**:
   - **External routing** (e.g., routing `style_analyze` to an external API provider like OpenAI or Anthropic): Chunks of the source corpus (up to `source_sample_word_budget`) will be sent over the network.
   - **Local routing**: To keep the corpus completely offline, route `style_analyze` to a locally running model provider (e.g., llama.cpp or Ollama on localhost) or use the local fallback.

## Next steps

Natural follow-up work, if you want to continue from here, is:

1. add provider-specific diagnostics for embedding routes
2. add MCP or SSE notifications for health-state transitions
3. add per-route timeout, temperature, and token controls
4. add persisted audit or status snapshots if runtime history becomes useful

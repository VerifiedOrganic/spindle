# Writing with GPT-6 Astra in Codex

Use GPT-6 Astra as the active model in the Codex app or CLI. Codex writes and
revises the prose; Spindle supplies the story bible, context, validation, and
persistent drafts. This host-writing path needs no OpenAI API key in Spindle.
Spindle's optional internal model routes are separate from your Codex session.

## Connect your existing book

Build the updated binary from the Spindle source checkout:

```sh
cargo build --release -p spindle-mcp
```

Use the **same book folder and Spindle data/config paths you used with Claude**.
Switching clients does not require importing the manuscript or creating a new
project. If Claude used `SPINDLE_DATA_DIR` or `SPINDLE_CONFIG`, carry those exact
values into the Codex MCP configuration. Otherwise, use the same working
directory so Spindle resolves the same `.spindle/` workspace or platform-global
store. Do not run `spindle-mcp init` in an existing global-store book folder:
creating a new `.spindle/` there would select a new, empty workspace.

Install the discoverable global skill (from any folder):

```sh
/absolute/path/to/spindle-mcp init-codex
```

Both `spindle-mcp init` and `spindle-mcp init-codex` install one `spindle` entry
skill in `~/.agents/skills/spindle/`, available across book folders. It loads
all twelve specialist workflows from the running MCP server as needed,
including scene writing, authoring supervision, revision, canon stewardship,
and research. One discovery entry keeps Spindle's startup footprint small.

`init-codex` only installs the skill: it does not initialize a database or change
model/client configuration. It is repeatable and refuses to replace differing
content, preserving customized skills. When upgrading an installed skill,
preserve any custom edits before replacing the file. The previous
`init-codex-skills` command is an alias.

For project scope instead of global installation, pass
`init-codex --target-dir "/path/to/my-book"`. This installs the same entry under
that folder's `.agents/skills/`; avoid installing both scopes for the same book
unless you intentionally want duplicate entries in the skill picker.

For a **new** book only, initialize its workspace first:

```sh
mkdir my-book
cd my-book
git init
/absolute/path/to/spindle-mcp init
```

`init` creates the local workspace and installs the global skill in one step.

If Codex warns that skill descriptions were shortened, its discovery list has
exceeded its context budget. Selected skills still load their full instructions;
this warning does not truncate Spindle's writing workflows. Use `$spindle`
explicitly and disable unused skills/plugins if the warning persists. Avoid
installing separate local copies of the specialist workflows. See OpenAI's
[skill discovery guidance](https://learn.chatgpt.com/docs/build-skills).

Add the following to your book's `.codex/config.toml`, merging with any existing
settings. Replace the example paths with absolute paths. Project configuration
is loaded for trusted projects; open/trust the book folder in Codex.

```toml
model = "gpt-6-astra"
model_reasoning_effort = "high"

[mcp_servers.spindle]
command = "/absolute/path/to/spindle-mcp"
cwd = "/absolute/path/to/my-book"
startup_timeout_sec = 30
tool_timeout_sec = 180

[mcp_servers.spindle.env]
SPINDLE_TOOL_PROFILE = "authoring"
# If your previous client set these, use the SAME paths:
# SPINDLE_DATA_DIR = "/absolute/path/to/existing/spindle-data"
# SPINDLE_CONFIG = "/absolute/path/to/existing/config.toml"
```

The `authoring` profile covers the ordinary writing loop, including character
snapshots, knowledge updates, scene clocks, and the writer's saved position.
Remove `SPINDLE_TOOL_PROFILE` when doing specialist imports or other work that
requires the full toolbox.

The native `codex mcp add` command is an alternative for registering a server in
your user configuration. Bind its data path explicitly when it should always
open one book, and avoid duplicating the project-scoped entry above:

```sh
codex mcp add spindle \
  --env SPINDLE_TOOL_PROFILE=authoring \
  --env SPINDLE_DATA_DIR=/absolute/path/to/existing/spindle-data \
  -- /absolute/path/to/spindle-mcp
```

Include `--env SPINDLE_CONFIG=...` as well if your existing setup uses it.

Open the book folder in the Codex app, select **GPT-6 Astra**, and start a fresh
thread after changing MCP configuration. For the CLI:

```sh
cd /absolute/path/to/my-book
codex -m gpt-6-astra -c model_reasoning_effort=high
```

Use `/mcp` in the CLI to inspect the connection. If the skill has not appeared,
restart Codex. Start with:

```text
$spindle Resume my existing book. Read the writer state and the last saved scene,
then draft the next planned scene in the established voice. Save it, check
continuity, fix local prose issues, and update the story state. Ask only about
unresolved creative decisions.
```

Spindle should list your existing projects and recover the expected branch and
cursor. If the book appears empty, check the MCP working directory and data
overrides before creating anything.

## Get the best writing from the workflow

Give the book a concrete reader contract and a few accepted passages that show
the voice you want. Keep positive targets specific: narrative distance, humor,
interiority, rhythm, dialogue, and what the reader should feel. The writing skill
reads prior prose and available voice samples, then uses the scene's bounded
context packet instead of loading an entire manuscript into every turn.

Ask for a complete scene at a useful length and with a clear purpose. For example:
“About 1,200 words, close third, past tense; she agrees to the bargain but hides
the missing key. Keep the restrained humor and end on the cost of her choice.”
The updated craft guidance allows purposeful summary and quiet beats, keeps
soft anti-slop findings advisory, and avoids forcing every paragraph through a
stock bodily reaction.

For multi-chapter runs, use `authoring-supervisor` in default **hybrid** mode.
Codex responds to a host-draft step by writing the scene and saving it with
`authoring_save_scene_draft`, including its continuity package. For standalone
scenes it uses `save_scene_draft`. AI-written or AI-revised prose is always
marked `authorship: "assistant"`.

Selecting Astra in Codex does **not** redirect `continue_generation`, internal
reviews, reader simulation, or canon mining to Astra. Those use the agents in
Spindle's configuration. Existing rating/receipt gates still apply. Inspect
route availability before requesting those passes; empty local-stub results
are not an independent model review. Ordinary host drafting can use
deterministic consistency checks plus Codex's own editorial pass. Authoring
runs retain their checkpoint requirements and must report unavailable coverage.

`high` reasoning is a starting configuration, not a measured prose-quality
guarantee. Judge the result against your own accepted scenes. The existing
[serial-fiction comparison kit](../evals/README.md) can compare context and
workflow conditions without claiming unmeasured wins; host-model usage must
come from Codex because Spindle cannot see it.

Setup follows the official [Codex MCP documentation](https://developers.openai.com/codex/mcp)
and [skill discovery documentation](https://developers.openai.com/codex/skills).
The writing guidance addresses the instruction-following, follow-through, and
formatting behaviors described in [OpenAI's GPT-6 Astra guide](https://developers.openai.com/api/docs/guides/latest-model/gpt-6-astra.md#prompting-best-practices).

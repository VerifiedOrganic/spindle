# Spindle

**A story bible that keeps your novel consistent across hundreds of pages.**

Spindle is a local-first writing companion for long-form fiction. It gives your
AI writing assistant a persistent memory of your characters, world, plot
threads, and prose — so the scene you draft in chapter 32 still knows what
happened in chapter 3, who's holding the cursed sword, and which promises to
the reader haven't paid off yet.

It runs entirely on your machine. Your manuscript, canon, and revisions never
leave your laptop.

## The problem Spindle solves

Long manuscripts fall apart. A character's eye color drifts. A subplot you
seeded in chapter 4 quietly evaporates. The magic system contradicts itself
in act three. You revise a scene and lose the version you actually preferred.
Your AI co-writer, however capable, only sees the window of text you paste in
— it has no idea your protagonist's mother died eighty pages ago.

Spindle is the layer underneath your AI writing tool that fixes this. It
holds the story bible, hands the model exactly the context it needs for the
scene you're drafting, tracks what's already canon, and lets you branch and
revise without losing earlier work.

## What you get

- **A story bible that remembers.** Characters, locations, factions, world
  rules, relationships, timelines, themes, motifs, narrative promises — all
  stored, queryable, and automatically surfaced when relevant.
- **A drafting loop that hands your AI the right context.** Before each
  scene, Spindle assembles a writing packet: the characters present, the
  locations involved, the world rules that apply, the pacing directives, the
  promises due to pay off, and recent chapter summaries.
- **Branching and save points.** Try an alternate version of a chapter
  without losing the original. Compare drafts. Restore earlier scene
  versions. Branch the whole manuscript to explore "what if" rewrites.
- **Continuity and consistency checks.** Catch contradictions before they
  become rewrites. Run dual-persona editorial reviews. Search the bible for
  every scene that references a given character, item, or rule.
- **Import an existing manuscript.** Already have a draft? Spindle can
  ingest it, extract characters and world canon, and pick up where you left
  off.
- **EPUB export.** Ship the finished book.

## Who it's for

Fiction writers — especially novelists and serial authors — who use AI
writing tools (Claude Code, Claude Desktop, or any MCP-aware client) and have
hit the wall where the AI can't hold the whole story in its head. Spindle is
the missing memory.

LitRPG, progression fantasy, and other long-running serial fiction benefit
the most, but Spindle is genre-agnostic.

## Quickstart

You'll need [Rust](https://rustup.rs/) installed.

1. **Clone and build Spindle:**
   ```bash
   git clone https://github.com/VerifiedOrganic/spindle
   cd spindle
   cargo build --release -p spindle-mcp
   ```

2. **Initialize a local book workspace:**
   Create a new folder for your book project, initialize a git repository, and run Spindle workspace initialization:
   ```bash
   mkdir my-book && cd my-book
   git init
   /path/to/spindle/target/release/spindle-mcp init
   ```
   This creates a project-local `.spindle/` folder containing your `config.toml`, SQLite database, and `artifacts/` subdirectory.

3. **Configure your MCP Client:**
   Point your MCP-capable AI client at the built binary. For **Claude Code**, you can configure it to start from your book directory:
   ```json
   {
     "mcpServers": {
       "spindle": {
         "command": "/path/to/spindle/target/release/spindle-mcp",
         "cwd": "/path/to/my-book"
       }
     }
   }
   ```

4. **Start writing:**
   Connect/use Spindle MCP from that folder and start designing or writing using the `authoring-supervisor` skill!

By default, Spindle resolves to the project-local `.spindle/` directory (climbing up parents to find it). A platform-global data directory is used as a fallback only when no `.spindle/` workspace exists. You can still set `SPINDLE_DATA_DIR` and `SPINDLE_CONFIG` to override this behavior.


## A first session

A typical first session against a fresh Spindle install looks like this.
Each step is a single instruction to your AI client — it makes the
underlying tool calls for you.

1. **Create the project.** Spindle sets up the story bible and returns a
   project id that becomes the default for the rest of the session.
2. **Create a book and a chapter** you want to draft into.
3. **Seed any canon you already know** — characters, locations, factions,
   world rules. Spindle ships with `worldbuilder` and `character-creator`
   skills that walk your AI through this conversationally.
4. **Plan the chapter.** Sketch the scenes. Spindle returns scene slots you
   can fill in any order.
5. **Get scene context.** For the scene you want to draft, Spindle assembles
   the writing packet: characters present, locations, relevant world rules,
   pacing directives, narrative promises due, recent summaries.
6. **Draft the prose** in your AI client using that context.
7. **Save the draft.** Spindle stores the scene and returns an id you can
   reference later.
8. **Commit what the scene establishes** — character state updates,
   canonical facts, relationship changes. This is what keeps chapter 32
   consistent with chapter 3.
9. **Annotate beats and save a chapter summary** so the next scene's
   context stays tight.

Drive that loop across chapters and Spindle keeps the story bible
consistent.

## Embedded skills

Spindle ships writing skills your AI client can load directly:

- `bible-librarian` — search and lookup across the story bible
- `authoring-supervisor` — coordinate interactive drafting and checkpoint runs
- `scene-writer` — draft prose with the right context
- `character-creator` — build out a character
- `worldbuilder` — develop locations, factions, rules, lore
- `plot-architect` — structure, pacing, conflicts, narrative promises
- `continuity-editor` — catch contradictions
- `revision-manager` — branch, compare, and revise
- `editor` — developmental and line edits
- `manuscript-importer` — ingest an existing draft

For Grok users, `init_grok_skills` installs the full skill set into
`~/.grok/skills/`.

## Features at a glance

Spindle's MCP surface gives your AI client tools for:

- **Project structure** — projects, books, chapters, scenes
- **World and canon** — characters, locations, factions, religions,
  economies, terms, relationships, world rules, voice profiles
- **Plot tracking** — plot lines, conflicts, themes, motifs, narrative
  promises, character arcs, timelines, temporal interventions
- **Pacing and planning** — pacing configs and curves, arc constraints,
  chapter and book outlines
- **Drafting loop** — scene context assembly, draft persistence, scene
  beats, summaries, commits
- **Revision and branching** — branches, save points, scene versions,
  alternatives, diffs, merges
- **Analysis** — consistency checks, bible search, dual-persona editorial
  review, canonical fact extraction
- **Import** — full manuscript ingestion with entity extraction and bible
  hydration
- **Export** — EPUB output, bible export, preflight checks

See [`docs/spindle-architecture.md`](docs/spindle-architecture.md) for the
full tool reference.

### LitRPG system blocks

EPUB export recognizes system UI blocks in scene prose. Use either a
pandoc-style fenced div or a backtick code fence whose info string names a
system class (`system-box`, `system-notification`, `system-pull`,
`system-quest`, or plain `system` as an alias for `system-box`):

```text
::: system-box                ```system-box
STAGE CRED EARNED: +2.        STAGE CRED EARNED: +2.
:::                           ```
```

Both render as styled XHTML `div` elements in exported EPUB files.

## Going further

- **Interactive Drafting Loop.** The `authoring-supervisor` skill drives an interactive, chat-native drafting run using SQLite-persisted states, automated checkpoints, and user feedback reviews. See [`docs/authoring-supervisor.md`](docs/authoring-supervisor.md).
- **Batch drafting.** `spindle-harness` is an operator-driven tool for
  unattended batch drafting with checkpointed editorial review and resumable
  artifacts. See [`docs/spindle-harness-usage.md`](docs/spindle-harness-usage.md).
- **Custom model routing.** Bind specific tasks (drafting, editing,
  embeddings) to different models through `spindle.toml`. The embedding
  route can use an OpenAI-compatible embedding model for higher-quality
  Bible search. See [`docs/spindle-agent-config.md`](docs/spindle-agent-config.md).
- **HTTP mode.** For multi-client or networked setups, run with
  `SPINDLE_HTTP_ADDR=127.0.0.1:8787` to expose the streamable HTTP MCP
  transport at `/mcp`. Currently experimental.

## Under the hood

Spindle is a single Rust binary that speaks the
[Model Context Protocol](https://modelcontextprotocol.io/) over stdio (or
optionally HTTP) to your AI client. All state lives in a local SQLite
database. The workspace is split into:

- `spindle-core` — shared models, contracts, public DTOs
- `spindle-adapters` — SQLite persistence, repositories, services, model
  routing, search and embeddings
- `spindle-skills` — build-time embedding of repo-local skills
- `spindle-mcp` — the MCP server, tools, and resources
- `spindle-harness` — operator-driven batch drafting

The architectural brief lives at
[`docs/spindle-implementation-brief.md`](docs/spindle-implementation-brief.md).
See [`docs/README.md`](docs/README.md) for the full docs map.

## Build and contribute

```bash
cargo fmt --all
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Wall-clock performance regressions are gated out of the default suite. Run
them explicitly when needed:

```bash
cargo test -p spindle-core --features perf
cargo test -p spindle-adapters --features perf
```

Run the MCP server directly during development:

```bash
cargo run -p spindle-mcp
```

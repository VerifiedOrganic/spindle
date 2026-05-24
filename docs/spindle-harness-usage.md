# Spindle Harness — Usage Guide

This guide walks through every step of using `spindle-harness` to automate
multi-chapter writing with editorial checkpoints. All examples use real command
syntax and copy-pasteable JSON.

## Prerequisites

1. The Spindle MCP server must be buildable (`cargo build -p spindle-mcp`)
2. A Spindle project must exist with characters, locations, and chapter plans
3. A model agent must be configured for the `draft` route in your `spindle.toml`

Build the harness:

```bash
cargo build -p spindle-harness
```

Or run it directly with `cargo run -p spindle-harness --`.

## Concepts

### Harness State File

A JSON file you own. It tracks everything the harness needs that the Spindle DB
does not store: which scenes to write, what phase each scene is in, checkpoint
history, editorial directives, and artifact paths.

### Seed File

A JSON file you create before starting a batch. It defines the chapter range,
per-scene manifests (characters, locations, ratings, tones), and standing
editorial directives. The harness initializes its state file from the seed.

### Artifacts

Generated content is saved as JSON files alongside the state file in a
`spindle-harness-artifacts/` directory. This includes the full model prompt,
all completion fragments, parsed scene packages, and checkpoint reports. These
survive crashes and enable resume without regeneration.

### Scene Phases

Each scene progresses through four phases:

```
pending → draft_saved → changes_committed → beats_annotated
```

After all scenes in a chapter reach `beats_annotated`, the chapter summary is
saved and the chapter is marked complete.

### Checkpoints

After every N completed chapters (configurable), the harness runs a full
editorial review: consistency audit, dual-persona craft review on sampled
scenes, pacing check, and creates a save point. It then pauses for human
review.

---

## Step 1: Find Your Entity IDs

Before writing a seed file, you need your project's real entity IDs. You can
get these from the Spindle MCP tools or from the Claude Code session where you
created them.

The IDs look like:

```
project:ftxjajglsj23zufjmovr
character:abc123
location:def456
```

If you are in a Claude Code session with Spindle connected, you can use tools
like `list_projects`, `search_bible`, or read resources like
`bible://projects/{project_id}/characters` to find your IDs.

---

## Step 2: Write a Seed File

Create a JSON file that describes exactly what chapters and scenes to write.
Every scene must include `character_ids`, `location_id`, and `content_rating`
because the Spindle DB does not store these on chapter plans.

### Minimal Seed — 2 Chapters, 3 Scenes Total

Save this as `~/ledger-batch-1.seed.json`:

```json
{
  "project_id": "project:gf2fnhdoxeea8obvd3xr",
  "book_number": 1,
  "range": {
    "start_chapter": 11,
    "end_chapter": 12
  },
  "checkpoint_interval": 2,
  "editorial_directives": [
    "Maintain continuity with chapters 1-10",
    "Keep content rating at teen",
    "Marcus should sound clipped and formal in dialogue"
  ],
  "chapters": [
    {
      "chapter_number": 11,
      "synopsis": "Marcus confronts Elena about the ledger discrepancies.",
      "pov_character_id": "character:marcus",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:marcus", "character:elena"],
          "location_id": "location:archive",
          "content_rating": "teen",
          "tone": "tense"
        },
        {
          "scene_order": 2,
          "character_ids": ["character:marcus"],
          "location_id": "location:roof",
          "content_rating": "teen",
          "tone": "grim"
        }
      ]
    },
    {
      "chapter_number": 12,
      "synopsis": "Elena tests whether Marcus will keep her secret.",
      "pov_character_id": "character:elena",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:elena", "character:marcus"],
          "location_id": "location:safehouse",
          "content_rating": "teen",
          "tone": "suspicious"
        }
      ]
    }
  ]
}
```

### Larger Seed — 6 Chapters With Checkpoint

Save as `~/ledger-batch-2.seed.json`:

```json
{
  "project_id": "project:gf2fnhdoxeea8obvd3xr",
  "book_number": 1,
  "range": {
    "start_chapter": 13,
    "end_chapter": 18
  },
  "checkpoint_interval": 6,
  "editorial_directives": [
    "Accelerate the trust arc — Marcus must begin doubting Elena by chapter 16",
    "Content rating: teen throughout",
    "Reduce interior monologue; prefer action and dialogue"
  ],
  "chapters": [
    {
      "chapter_number": 13,
      "synopsis": "A third party reveals information that contradicts Elena's story.",
      "pov_character_id": "character:marcus",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:marcus", "character:vasquez"],
          "location_id": "location:precinct",
          "content_rating": "teen",
          "tone": "procedural"
        }
      ]
    },
    {
      "chapter_number": 14,
      "synopsis": "Marcus verifies the contradicting information on his own.",
      "pov_character_id": "character:marcus",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:marcus"],
          "location_id": "location:archive",
          "content_rating": "teen",
          "tone": "methodical"
        },
        {
          "scene_order": 2,
          "character_ids": ["character:marcus"],
          "location_id": "location:apartment",
          "content_rating": "teen",
          "tone": "anxious"
        }
      ]
    },
    {
      "chapter_number": 15,
      "synopsis": "Elena realizes Marcus is pulling away and makes a preemptive move.",
      "pov_character_id": "character:elena",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:elena"],
          "location_id": "location:safehouse",
          "content_rating": "teen",
          "tone": "calculating"
        },
        {
          "scene_order": 2,
          "character_ids": ["character:elena", "character:marcus"],
          "location_id": "location:restaurant",
          "content_rating": "teen",
          "tone": "tense"
        }
      ]
    },
    {
      "chapter_number": 16,
      "synopsis": "The confrontation. Marcus lays out what he found.",
      "pov_character_id": "character:marcus",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:marcus", "character:elena"],
          "location_id": "location:rooftop",
          "content_rating": "teen",
          "tone": "volatile"
        }
      ]
    },
    {
      "chapter_number": 17,
      "synopsis": "Aftermath. Marcus decides whether to report Elena or protect her.",
      "pov_character_id": "character:marcus",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:marcus"],
          "location_id": "location:apartment",
          "content_rating": "teen",
          "tone": "conflicted"
        },
        {
          "scene_order": 2,
          "character_ids": ["character:marcus", "character:vasquez"],
          "location_id": "location:precinct",
          "content_rating": "teen",
          "tone": "guarded"
        }
      ]
    },
    {
      "chapter_number": 18,
      "synopsis": "Elena discovers Marcus's decision through a third party.",
      "pov_character_id": "character:elena",
      "scenes": [
        {
          "scene_order": 1,
          "character_ids": ["character:elena", "character:vasquez"],
          "location_id": "location:precinct",
          "content_rating": "teen",
          "tone": "cold"
        }
      ]
    }
  ]
}
```

### Seed Field Reference

| Field | Required | Description |
|---|---|---|
| `project_id` | Yes | Spindle project ID (e.g. `project:abc123`) |
| `book_number` | Yes | Which book in the project |
| `range.start_chapter` | Yes | First chapter to write |
| `range.end_chapter` | Yes | Last chapter to write |
| `checkpoint_interval` | Yes | Run editorial review every N completed chapters |
| `editorial_directives` | No | Standing instructions applied to every scene prompt |
| `chapters[].chapter_number` | Yes | Must be contiguous within range, no gaps |
| `chapters[].synopsis` | Yes | Must match the persisted `plan_chapter` synopsis if one exists |
| `chapters[].pov_character_id` | No | POV character for the chapter |
| `chapters[].scenes[].scene_order` | Yes | Must be contiguous starting from 1 |
| `chapters[].scenes[].character_ids` | Yes | All characters present in the scene |
| `chapters[].scenes[].location_id` | Yes | Where the scene takes place |
| `chapters[].scenes[].content_rating` | Yes | `general`, `teen`, `mature`, or `explicit` |
| `chapters[].scenes[].tone` | No | Target tone (e.g. `tense`, `grim`, `hopeful`) |
| `chapters[].scenes[].source_path` | No | Optional reference to source manuscript file |

---

## Step 3: Initialize the Harness

The `init` command reads your seed, connects to the Spindle MCP server,
validates everything against live data, and writes the harness state file.

### Spawning a Fresh Spindle MCP Child

The simplest mode — the harness starts its own `spindle-mcp` process:

```bash
cargo run -p spindle-harness -- init \
  --state ~/ledger-batch-1.state.json \
  --seed ~/ledger-batch-1.seed.json
```

### With a Custom Data Directory

If your Spindle data lives somewhere other than `~/.local/share/spindle`:

```bash
cargo run -p spindle-harness -- init \
  --state ~/ledger-batch-1.state.json \
  --seed ~/ledger-batch-1.seed.json \
  --server-data-dir /path/to/your/spindle/data
```

### Connecting to an Already-Running Spindle Server

If Spindle is already running with HTTP enabled (`SPINDLE_HTTP_ADDR`):

```bash
cargo run -p spindle-harness -- init \
  --state ~/ledger-batch-1.state.json \
  --seed ~/ledger-batch-1.seed.json \
  --server-url http://127.0.0.1:4321/mcp
```

### Expected Output

```
No findings.
Initialized harness state at /Users/you/ledger-batch-1.state.json
Active branch: bible_branch:main
Next action: draft book scene 11.1
```

If there are problems (missing chapter plans, branch mismatches, missing
characters), you will see findings:

```
[error] chapter_plan_synopsis_mismatch: chapter 11 synopsis differs from persisted chapter plan
[error] missing_scene_characters: chapter 12 scene 1 has no character_ids
refusing to write harness state because initialization is not continuity-safe
```

Fix the seed file and re-run init.

---

## Step 4: Check Status

See where things stand at any time:

```bash
cargo run -p spindle-harness -- status \
  --state ~/ledger-batch-1.state.json
```

Output:

```
Project: project:gf2fnhdoxeea8obvd3xr
Active branch: bible_branch:main
Range: book 1 chapters 11-12
Checkpoint interval: 2
Completed chapters: 0
Last checkpoint end: 10
Editorial directives: 3
Checkpoint history: 0
Chapter 11 [pending] summary_saved=false scenes=[1:pending, 2:pending]
Chapter 12 [pending] summary_saved=false scenes=[1:pending]
```

### Verbose Status

Add `--verbose` for full detail including artifact paths, diagnostics, and
checkpoint records:

```bash
cargo run -p spindle-harness -- status \
  --state ~/ledger-batch-1.state.json \
  --verbose
```

Output:

```
Project: project:gf2fnhdoxeea8obvd3xr
Active branch: bible_branch:main
Range: book 1 chapters 11-12
Checkpoint interval: 2
Completed chapters: 0
Last checkpoint end: 10
Editorial directives: 3
Checkpoint history: 0
Artifacts root: /Users/you/spindle-harness-artifacts
Directives:
  - Maintain continuity with chapters 1-10
  - Keep content rating at teen
  - Marcus should sound clipped and formal in dialogue
Chapter 11 [pending] summary_saved=false scenes=[1:pending, 2:pending]
  Scene 1 [pending] scene_id=- artifact=-
  Scene 2 [pending] scene_id=- artifact=-
Chapter 12 [pending] summary_saved=false scenes=[1:pending]
  Scene 1 [pending] scene_id=- artifact=-
```

---

## Step 5: Execute One Step

The harness advances exactly one continuity-safe action per `--execute-one`
call. This is the core loop.

```bash
cargo run -p spindle-harness -- resume \
  --state ~/ledger-batch-1.state.json \
  --writeback \
  --execute-one
```

### What Happens Per Step

The harness determines the next action automatically based on state:

| State | Next Action | What It Does |
|---|---|---|
| Scene is `pending` | `DraftScene` | Calls `get_chapter_briefing` + `get_scene_context`, sends prompt to draft model, parses JSON output, calls `save_scene_draft` |
| Scene is `draft_saved` | `CommitSceneChanges` | Calls `commit_scene_changes` with character states, facts, relationships from the artifact |
| Scene is `changes_committed` | `AnnotateSceneBeats` | Calls `annotate_scene_beats` with beats from the artifact |
| All scenes `beats_annotated` | `SaveChapterSummary` | Generates a summary prompt, sends to model, calls `save_summary` |
| N chapters completed | `RunCheckpoint` | Runs `check_consistency`, samples scenes for `run_dual_persona_review`, reads pacing/promises, creates save point |
| Checkpoint pending review | `AwaitCheckpointReview` | Refuses to continue until you review the checkpoint |

### Expected Output

First call:

```
No findings.
Next action: draft book scene 11.1
Saved draft for chapter 11 scene 1 as scene:abc123
No findings.
Next action: commit scene changes for chapter 11 scene 1 (scene:abc123)
Updated harness state at /Users/you/ledger-batch-1.state.json
```

Second call:

```
No findings.
Next action: commit scene changes for chapter 11 scene 1 (scene:abc123)
Committed scene changes for chapter 11 scene 1
No findings.
Next action: annotate beats for chapter 11 scene 1 (scene:abc123)
Updated harness state at /Users/you/ledger-batch-1.state.json
```

### Dry Run (Without --execute-one)

To see what would happen next without doing it:

```bash
cargo run -p spindle-harness -- resume \
  --state ~/ledger-batch-1.state.json
```

Output:

```
No findings.
Next action: draft book scene 11.1
Execution is not implemented yet; this is a continuity-safe dry run.
```

---

## Step 6: Run Multiple Steps in a Loop

To execute continuously until the harness pauses (checkpoint or error), wrap
`resume` in a shell loop:

```bash
while cargo run -p spindle-harness -- resume \
  --state ~/ledger-batch-1.state.json \
  --writeback \
  --execute-one; do
  echo "--- Step completed, continuing... ---"
done
echo "--- Harness paused (checkpoint, error, or complete) ---"
```

The loop exits when:
- A checkpoint requires human review (exit code non-zero)
- An error occurs (model failure, partial commit, etc.)
- All chapters are complete

### For a 2-Chapter Batch With 3 Scenes

The full sequence of steps would be:

```
draft book scene 11.1              → save_scene_draft
commit scene changes 11.1          → commit_scene_changes
annotate beats 11.1                → annotate_scene_beats
draft book scene 11.2              → save_scene_draft
commit scene changes 11.2          → commit_scene_changes
annotate beats 11.2                → annotate_scene_beats
save summary for chapter 11        → save_summary
draft book scene 12.1              → save_scene_draft
commit scene changes 12.1          → commit_scene_changes
annotate beats 12.1                → annotate_scene_beats
save summary for chapter 12        → save_summary
run checkpoint for chapters 11-12  → check_consistency + dual_persona_review + create_save_point
(pauses — awaiting human review)
```

That is 13 steps total for this batch.

---

## Step 7: Review a Checkpoint

When the harness runs a checkpoint, it creates a save point in Spindle and
writes a detailed report artifact. It then refuses to continue until you
review.

### Read the Report

The report is saved as a JSON file:

```bash
cat ~/spindle-harness-artifacts/checkpoints/chapter-0011-0012.json | python3 -m json.tool
```

The report contains:
- `consistency` — full output of `check_consistency`
- `sampled_reviews` — `run_dual_persona_review` results for sampled scenes
- `pacing_overview` — current pacing state
- `chapter_summaries` — summaries for the chapters in range
- `narrative_promises` — promise tracking status
- `save_point` — the save point ID for rollback
- `sampled_scene_ids` — which scenes were reviewed

### Mark the Checkpoint as Reviewed

After reviewing, approve the checkpoint and optionally add new directives:

```bash
cargo run -p spindle-harness -- review-checkpoint \
  --state ~/ledger-batch-1.state.json \
  --start-chapter 11 \
  --end-chapter 12 \
  --directive "Good pacing — maintain this rhythm" \
  --directive "Elena's voice needs more edge in chapter 13+"
```

Output:

```
Marked checkpoint 11-12 as reviewed; added 2 new directive(s).
```

### Mark Reviewed Without New Directives

```bash
cargo run -p spindle-harness -- review-checkpoint \
  --state ~/ledger-batch-1.state.json \
  --start-chapter 11 \
  --end-chapter 12
```

After reviewing, `resume` will continue to the next chapter.

---

## Step 8: Verify State Against Live DB

If you suspect the harness state has drifted from what is actually in Spindle
(e.g. after manual edits via Claude Code), verify:

```bash
cargo run -p spindle-harness -- verify \
  --state ~/ledger-batch-1.state.json
```

This connects to Spindle, reads the actual scenes, summaries, and branch state,
and reports any mismatches.

### Auto-Fix Minor Drift

Add `--writeback` to let the harness update its state file with corrections
(e.g. capturing scene_ids for scenes that exist in Spindle but were missing
from state):

```bash
cargo run -p spindle-harness -- verify \
  --state ~/ledger-batch-1.state.json \
  --writeback
```

Verification will refuse to write back if there are blocking errors.

---

## Step 9: Handle Blocked Scenes

If `commit_scene_changes` returns partial errors (some items applied, some
failed), the harness marks the scene as blocked and stops.

Check what is blocked:

```bash
cargo run -p spindle-harness -- status \
  --state ~/ledger-batch-1.state.json \
  --verbose
```

```
Chapter 11 [in_progress] summary_saved=false scenes=[1:draft_saved, 2:pending]
  Scene 1 [draft_saved] scene_id=scene:abc123 artifact=/.../scene-001.json
    blocked: commit_scene_changes applied partial results; inspect artifact ...
```

### Inspect the Artifact

```bash
cat ~/spindle-harness-artifacts/scenes/chapter-0011/scene-001.json | python3 -m json.tool
```

Look at the `commit_output` field to see which items failed and why.

### Resolve the Block

After fixing the issue (or deciding to skip the failed items), manually advance
the scene to the next phase:

```bash
cargo run -p spindle-harness -- resolve-scene-block \
  --state ~/ledger-batch-1.state.json \
  --chapter-number 11 \
  --scene-order 1 \
  --target-phase changes-committed
```

Output:

```
Advanced scene 11.1 to changes_committed after operator review.
Previous block: commit_scene_changes applied partial results; inspect artifact ...
```

The harness will then continue from `annotate_scene_beats` on the next
`resume --execute-one`.

### Allowed Phase Advances

You can only advance one phase at a time:

| Current Phase | Allowed Target |
|---|---|
| `pending` | `draft-saved` |
| `draft-saved` | `changes-committed` |
| `changes-committed` | `beats-annotated` |

---

## Step 10: Start a New Batch

After completing a batch, create a new seed file for the next chapter range
and initialize a new state file:

```bash
cargo run -p spindle-harness -- init \
  --state ~/ledger-batch-2.state.json \
  --seed ~/ledger-batch-2.seed.json
```

Each batch is independent. You can have multiple state files for different
projects or different chapter ranges.

---

## Transport Options Reference

Every command that talks to Spindle accepts these transport flags:

| Flag | Description |
|---|---|
| (none) | Spawns a child `spindle-mcp` process automatically |
| `--server-data-dir PATH` | Sets `SPINDLE_DATA_DIR` for the spawned child |
| `--server-config PATH` | Sets `SPINDLE_CONFIG` for the spawned child |
| `--server-url URL` | Connects to an already-running HTTP endpoint |

### Examples

Spawn child with default data dir:
```bash
cargo run -p spindle-harness -- resume \
  --state ~/batch.state.json --writeback --execute-one
```

Spawn child with custom data dir:
```bash
cargo run -p spindle-harness -- resume \
  --state ~/batch.state.json --writeback --execute-one \
  --server-data-dir ~/my-spindle-data
```

Connect to running server:
```bash
cargo run -p spindle-harness -- resume \
  --state ~/batch.state.json --writeback --execute-one \
  --server-url http://127.0.0.1:4321/mcp
```

---

## Command Reference

### `init`

Initialize harness state from a seed file.

```bash
spindle-harness init --state STATE_PATH --seed SEED_PATH [transport flags]
```

- Connects to Spindle and validates the seed against live data
- Refuses to write state if there are continuity errors
- Creates the state file at `STATE_PATH`

### `status`

Display current harness progress.

```bash
spindle-harness status --state STATE_PATH [--verbose]
```

- Offline — does not connect to Spindle
- `--verbose` shows artifact paths, diagnostics, directives, checkpoint details

### `verify`

Reconcile harness state against live Spindle data.

```bash
spindle-harness verify --state STATE_PATH [--writeback] [transport flags]
```

- Connects to Spindle and compares state
- `--writeback` updates state file with corrections (refuses on errors)

### `resume`

Determine next action and optionally execute it.

```bash
spindle-harness resume --state STATE_PATH [--writeback] [--execute-one] [transport flags]
```

- Without `--execute-one`: dry run showing the next action
- With `--execute-one`: executes exactly one step, re-verifies, saves state
- `--writeback`: allows state updates from reconciliation
- Exits non-zero at checkpoints, errors, or completion

### `review-checkpoint`

Mark a checkpoint as reviewed and optionally add editorial directives.

```bash
spindle-harness review-checkpoint --state STATE_PATH \
  --start-chapter N --end-chapter M \
  [--directive "..."] [--directive "..."]
```

- Offline — does not connect to Spindle
- Duplicate directives are ignored
- Unblocks the harness to continue past the checkpoint

### `resolve-scene-block`

Manually advance a blocked scene to the next phase.

```bash
spindle-harness resolve-scene-block --state STATE_PATH \
  --chapter-number N --scene-order M \
  --target-phase {draft-saved|changes-committed|beats-annotated}
```

- Offline — does not connect to Spindle
- Only allowed when the scene has a `blocked_reason`
- Can only advance one phase at a time

---

## File Layout After a Batch Run

```
~/
├── ledger-batch-1.seed.json           # Your input — keep for reference
├── ledger-batch-1.state.json          # Harness state — do not hand-edit
└── spindle-harness-artifacts/         # Generated content
    ├── scenes/
    │   ├── chapter-0011/
    │   │   ├── scene-001.json         # Full artifact: prompt, output, parsed package
    │   │   └── scene-002.json
    │   └── chapter-0012/
    │       └── scene-001.json
    ├── summaries/
    │   ├── chapter-0011.json          # Summary artifact: prompt, output, parsed summary
    │   └── chapter-0012.json
    └── checkpoints/
        └── chapter-0011-0012.json     # Checkpoint report: consistency, reviews, pacing
```

---

## Quick Reference: Full Batch Workflow

```bash
# 1. Write seed file
vim ~/ledger-batch-1.seed.json

# 2. Initialize
cargo run -p spindle-harness -- init \
  --state ~/ledger-batch-1.state.json \
  --seed ~/ledger-batch-1.seed.json

# 3. Run all steps until checkpoint
while cargo run -p spindle-harness -- resume \
  --state ~/ledger-batch-1.state.json \
  --writeback --execute-one; do
  echo "--- Step done ---"
done

# 4. Read checkpoint report
cat ~/spindle-harness-artifacts/checkpoints/chapter-0011-0012.json \
  | python3 -m json.tool | less

# 5. Review checkpoint
cargo run -p spindle-harness -- review-checkpoint \
  --state ~/ledger-batch-1.state.json \
  --start-chapter 11 --end-chapter 12 \
  --directive "Tighten scene transitions"

# 6. Check status
cargo run -p spindle-harness -- status \
  --state ~/ledger-batch-1.state.json --verbose

# 7. Continue (if more chapters remain after checkpoint)
while cargo run -p spindle-harness -- resume \
  --state ~/ledger-batch-1.state.json \
  --writeback --execute-one; do
  echo "--- Step done ---"
done
```

---

## Troubleshooting

### "draft route resolves to local adapter"

The harness requires a real LLM backend for the `draft` route. Configure a
model agent in your `spindle.toml` and assign it to the `draft` route.

### "branch_mismatch" error on init

The harness state tracks the active branch. If you switch branches in Spindle
between runs, verification will block. Either switch back or create a new
harness state file for the new branch.

### "chapter_plan_synopsis_mismatch"

Your seed file synopsis does not match what is stored in Spindle's chapter
plan. Either update the seed to match or re-run `plan_chapter` with the
correct synopsis.

### "missing_scene_artifact" after a crash

The harness found that a scene has been saved to Spindle (it has a scene_id)
but there is no local artifact file. This happens if the artifact directory
was deleted. The harness cannot safely resume without the artifact because it
needs the original prompt and generation data for commit and annotation phases.

Options:
1. If the scene was fully committed and annotated in Spindle, use
   `resolve-scene-block` to advance it manually
2. Otherwise, delete the scene from Spindle and reset the scene phase in the
   state file to `pending` (requires hand-editing the state JSON)

### Scene generation produces invalid JSON

The model output must be valid JSON matching the `GeneratedScenePackage`
schema. If the model returns prose instead of JSON, or malformed JSON, the
harness logs a `last_parse_error` in the scene artifact and fails. Check the
artifact file to see the raw model output and the parse error. You may need to
adjust your draft model's system prompt or use a more capable model.

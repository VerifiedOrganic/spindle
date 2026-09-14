# Fiction anti-slop

Status: **Phase 5 optional ship-it** is live. Rolling chapter counters apply
chapter-scoped shelf quotas across scenes in a chapter. Learned
false-positive suppressions persist from V0031 style-edit captures (operator
kept a flagged excerpt) and apply on later scans. StoryScope-inspired
structural observations exist behind `[anti_slop] experimental_structural`
and are **off by default** — they are not shelves and never increment
`hard_count`. Self-hosted sampler / FTPO is out of band. Phase 4 eval / CI /
console telemetry remains: eval cases lock scanner regressions; optional
labeled preference dims are editorial observations, not a quality
superiority claim. The run journal may carry an additive `anti_slop`
summary (ids + counts). CI fails if catalog MD, the v0 pack, and the
scanner shelf IDs drift. Project `[anti_slop]` config can disable / soften /
promote shelves; pack defaults and product locks still apply. Phase 3
critic / revise remains: hard verify fail uses a rewrite-from-beats prompt
contract (adapters + harness), capped at 1–2 passes, then re-lints and
surfaces residuals. Dual-persona review injects the scan report; the Craft
Technician must cite shelf IDs; the Literary Critic gets a structure block
that is not a BLUF/tech-structure gate. The Phase 1 scanner still lives at
`style/antislop/` in `spindle-core`. Soft shelves (including `solitary_fade`
and `said_bookism` by product lock) are warnings and do not increment
`hard_count`. Soft-on-save stays advisory; hard fail-closes only when
verify/revise is on.

This is the current design contract for fiction anti-slop. The human catalog is
[`references/anti-slop.md`](../references/anti-slop.md). The versioned pack stub
is [`references/anti-slop-shelf-pack.v0.toml`](../references/anti-slop-shelf-pack.v0.toml).

## Why this exists

Spindle drafts fiction. Generic LLM prose has a small set of repeatable *moves*
(contrast hinges, emotion cocktails, outlook endings, stock bodies and eyes).
Those moves are editorial, not an authorship test.

Phase 0 locks the **IDs, limits, rewrite rule, and later packet shapes** so a
scanner and the authoring loop can share one vocabulary.

## Product locks

These are product rules, not style preferences.

1. **Fiction only.** Catalog, pack, and any later scan apply to scene/chapter
   prose. Not docs, commit messages, research notes, or tool copy.
2. **`auto_strict` requires `anti_slop.hard_count == 0`.** When the scanner
   exists, harness `checkpoint_policy = auto_strict` must treat a non-zero hard
   shelf count as a finding that blocks auto-approve — same bar as today's
   "zero findings of any severity." Soft counts do not satisfy this lock by
   themselves and do not replace it.
3. **Soft on save; fail-closed hard when verify/revise is on.**
   - `save_scene_draft` / host save: soft hits are warnings; hard hits may be
     reported but must not block a save while `max_revise_attempts` is `0`.
   - When `max_revise_attempts` is `1` or `2`, a hard over-limit is fail-closed
     on the verify/revise path (same spirit as other ≥ `warning` verify
     findings). Soft stays advisory.
4. **Genre / style profiles may disable or soften shelves.** An applied style
   profile, `ReaderContract.style_notes`, or a `style` world rule can disable a
   shelf, keep it soft, or (explicitly) promote a soft shelf. Defaults stay as
   in the pack. `said_bookism` must not be hard unless a profile promotes it.
5. **Rewrite-from-beats, at most 1–2 passes.** Skills and later rewrite helpers
   go back to the scene beat. They do not synonym-swap the flagged sentence
   ("humanize"). Budget one or two rewrites per flagged stretch.

## Soft vs hard (defaults)

Hard (counted; `hard_count` later):

| ID | Default limit |
| --- | --- |
| `contrast_not_x_but_y` | ≤1 / chapter |
| `emotion_cocktail` | 0 |
| `fishing_ending` | 0 outlook-family closers |
| `said_bookism` | ≤2 non-`said` tags / chapter — **soft-only by default** |

Soft (advisory clusters): `body_reactions`, `eye_department`, `gesture_rack`,
`atmosphere_prefabs`, `naming_watchlist`, `rhythm_cadence`, `triadic_listing`,
`solitary_fade` (lived-in solitude; not a BLUF gate).

Twelve shelves. Skills must not claim a "100+" pattern list.

## StoryScope questions

Five editorial questions live in the catalog. They are not shelves and must not
become detector scores or fail gates. Genre may "fail" them on purpose.

## Non-ports

Do not implement or imply these as fiction gates: BLUF, tech buzzlists, em-dash
quotas, Flesch/grade-level, detector percentages, FAQ endings, delve-as-tech-gate.
Rationale is in the catalog.

## Shelf pack schema (v0)

`references/anti-slop-shelf-pack.v0.toml` is the v0 schema **and** the default
pack stub. Phase 1 should parse this file (or a successor `v1`) rather than
inventing parallel IDs.

Required top-level fields:

| Field | v0 meaning |
| --- | --- |
| `schema_version` | Integer. `0` for this stub. |
| `pack_id` | Stable pack name (`fiction.default`). |
| `pack_version` | Semver string for the pack contents. |
| `domain` | Must be `fiction`. |
| `catalog_uri` / `catalog_path` | Human catalog pointer. |
| `policy` | Product locks as data. |
| `genre_overrides` | Array of profile overlays (empty in Phase 0). |
| `shelves[]` | One object per stable ID. |
| `packet_fields` | Reserved shapes; not populated. |

Required per-shelf fields: `id`, `severity` (`hard` \| `soft`),
`default_limit` (integer or `advisory_cluster`), `limit_scope`, `enabled`,
`rewrite` (`from_beats`), `matchers` (empty array until Phase 1).

`matchers` staying empty is intentional. Phase 0 does not ship regex, lexicons,
or a scanner.

## Packet fields (Phase 2)

These fields are on `SceneContextOutput`, `SceneContextEnvelope`, and (for
the digest) `GetChapterBriefingOutput`. Do not invent parallel names.

### `voice_samples`

Short, **project-local** excerpts of on-voice prose: narrator plus named
speakers that already exist on the branch.

- Purpose: give rewrite-from-beats a positive target.
- Source (Phase 2): applied style-profile guidance (`do_rules`,
  `prompt_snippet`, narrator notes). Prior *accepted* scenes remain a later
  hook — not a copyrighted corpus dump.
- Persistence rule: match existing style-profile policy — do not persist long
  source excerpts by default; samples in the packet stay budget-capped.

### `scene_negatives`

Compact "do not repeat this" examples. Phase 2 hooks come from style-profile
`avoid_rules` (shelf id attached when the note names a shelf or cocktail
family). Prior shelf hits on the same chapter/run remain a later hook.

- Purpose: stop the next draft from regenerating the same cocktail, fishing
  closer, or eye-department stack.
- Shape: shelf id + one short rejected line or beat note. Not a shame list of
  the whole manuscript.

### `compact_shelf_digest`

A budget-capped digest of **enabled** shelves for the active genre/style
profile: id, severity, limit, limit_scope, and whether the profile disabled or
softened the shelf.

- Purpose: inject into `get_scene_context` / `get_chapter_briefing` without
  shipping the full catalog every call.
- Full catalog remains `bible://references/anti-slop`.
- Must stay small enough for the existing token-budget trimmer.

Later scan results (also reserved, not shipped):

| Field | Meaning |
| --- | --- |
| `anti_slop.hard_count` | Count of hard shelves over limit. `auto_strict` requires `0`. |
| `anti_slop.soft_count` | Advisory cluster count. Never sufficient to fail a save. |
| `anti_slop.hits[]` | Per-hit id, severity, span, rewrite hint (`from_beats`). |

## Authoring-loop hook (Phase 3)

1. **Save:** `save_scene_draft` scans and attaches `anti_slop` on the output.
   Soft + hard hits are advisory. The write always succeeds. Hard does not
   block while `max_revise_attempts` is `0`.
2. **Verify/revise on:** hard over-limit joins `SCENE_VERIFY_CHECKS` as
   `anti_slop` / `warning` and fail-closes. Soft remains advisory (not a
   verify finding).
3. **Checkpoint `auto_strict`:** `anti_slop.hard_count != 0` blocks
   auto-approve (product lock). Soft-only must not invent a hard finding.
4. **Rewrite:** rewrite-from-beats contract, at most two passes, then
   re-lint. Leftovers park like other unchanged verify findings.
5. **Dual-persona:** inject the report; Craft Technician cites shelf IDs (or
   NONE); Literary Critic uses the structure block (no BLUF / Voices / Flesch
   / delve gates).

## Skill honesty

`skills/scene-writer/SKILL.md` points at this catalog by shelf ID. It must not
claim a missing "100+" pattern list. The authoring supervisor does not repeat
that claim.

## Eval / CI / console (Phase 4)

- **Regression eval:** `evals/anti_slop/cases.json` plus
  `python3 evals/anti_slop.py self-test|drift|regress|score`. Cases lock
  expected shelf IDs, severity, and `hard_count`. `solitary_fade` stays
  soft. Preference dims are optional labels (`present` / `absent` / `n/a`),
  never a ranking of writing quality.
- **Journal / console:** `scene_drafted` (host save) may include an
  additive `anti_slop` key: `hard_count`, `soft_count`, `hard_ids`,
  `soft_ids`. The operator console timeline renders that summary. No
  excerpts (ADR 0002 D3.1).
- **CI drift:** catalog headings, `references/anti-slop-shelf-pack.v0.toml`
  shelf `id`s, and `SCANNER_SHELF_IDS` must be the same twelve IDs.
- **Project config:** `[anti_slop]` in `spindle.toml` / `.spindle/config.toml`
  (`disable`, `soften`, `promote_to_hard`). Empty means pack defaults.
  `said_bookism` stays soft unless explicitly promoted.

## Rolling chapter counters (Phase 5)

Chapter-scoped numeric quotas (`limit_scope = "chapter"`) accumulate raw
matches across earlier scenes in the same book/chapter. Scene 1 may consume
the chapter's allowed `contrast_not_x_but_y` hinge; scene 2's next hinge is
then over-limit. Scene-scoped advisory clusters (including `solitary_fade`)
do not roll. A new chapter resets the counters. Soft-on-save and
hard-on-verify locks are unchanged.

## Learned suppressions (Phase 5)

When style learning (V0031) captures an operator edit that **keeps** a
scanner-flagged excerpt, that `(shelf_id, normalized excerpt)` is stored
(`anti_slop_suppression`, migration V0045) and skipped on later scans.
`spindle-core` exposes a clean store + apply path even when learning is
off. Suppressions never invent a hard finding and never promote a soft
shelf.

## Experimental structural observations (Phase 5)

`[anti_slop] experimental_structural` (default **false**) may emit
StoryScope-inspired editorial notes (`theme_stated`, `gestured_reference`,
`tidy_close`). These are not shelves, not scores, and not verify findings.
They never increment `hard_count`. Genre may "fail" them on purpose.

## Self-hosted sampler / FTPO (out of band)

Spindle's `draft` route is an LLM chat completion (HTTP or builtin-local
stub), not a preference-pair trainer. Do not ship a half-baked in-process
sampler. Operators who want FTPO run it out of band.

## Out of scope

- Promoting `said_bookism` to hard in the default pack
- Porting Voices / tech gates
- Persisting source-corpus voice samples (hooks come from style-profile
  guidance only)
- An in-process self-hosted sampler / FTPO loop

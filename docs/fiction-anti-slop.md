# Fiction anti-slop

Status: **Phase 1 scanner** lives at `style/antislop/` in `spindle-core`. It
loads the Phase 0 pack + catalog. Soft shelves (including `solitary_fade` and
`said_bookism` by product lock) are warnings and do not increment `hard_count`.

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

## Packet fields (later — not on `SceneContextOutput` yet)

These fields are **reserved** for a later context-packet / briefing slice.
They are not implemented on `SceneContextOutput` or `ChapterBriefing` in
Phase 0. Do not invent parallel names.

### `voice_samples`

Short, **project-local** excerpts of on-voice prose: narrator plus named
speakers that already exist on the branch.

- Purpose: give rewrite-from-beats a positive target.
- Source: applied style profile guidance, prior *accepted* scenes, or
  narrator-voice notes — not a copyrighted corpus dump.
- Persistence rule: match existing style-profile policy — do not persist long
  source excerpts by default; samples in the packet stay budget-capped.

### `scene_negatives`

Compact "do not repeat this" examples tied to **this** scene's beats or to
prior shelf hits on the same chapter/run.

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

## Authoring-loop hook (Phase 1+, not implemented)

When a scanner exists, wire it as follows — do not implement this in Phase 0:

1. **Save:** run scan; attach soft + hard hits as warnings. Save always
   succeeds for soft. Hard does not block while verify is off.
2. **Verify/revise on:** hard over-limit joins the scene-scoped verify set and
   fail-closes. Soft remains advisory.
3. **Checkpoint `auto_strict`:** treat `anti_slop.hard_count != 0` as a
   finding. Soft-only must not auto-approve if hard_count is non-zero; it also
   must not *create* a hard finding by itself.
4. **Rewrite:** at most two from-beats passes, then park or carry to
   checkpoint like other unchanged verify findings.

## Skill honesty

`skills/scene-writer/SKILL.md` points at this catalog by shelf ID. It must not
claim a missing "100+" pattern list. The authoring supervisor does not repeat
that claim.

## Out of scope (Phase 1)

- Wiring the scanner into `save_scene_draft` / verify / `auto_strict`
- DTO fields on `SceneContextOutput`
- Dual-persona revise
- Promoting `said_bookism` to hard in the default pack
- Porting Voices / tech gates

# Continuity & quantity (money · progression) — design & roadmap

**Status:** the **entire roadmap (NOW + NEXT + LATER) is implemented and green** on branch
`feat/automated` (~28 new tests; `cargo clippy -D warnings` clean; full workspace suite passing).
Every tiered item below is shipped:
- **1.1–1.3** — economy-canon surfacing; the `QuantityState`/`QuantityScheme` model + **migration
  V0020**; the `set_project_quantity_scheme` / `commit_quantity_state` tools.
- **2.1–2.4** — the `quantity_drift` band-monotonicity check; the `[WEALTH/STATE]` scene-context
  hard-constraint render; the WarnOnly band-jump advisory returned from `commit_quantity_state`;
  the per-book band trajectory (a pure read inside the constraint — no V0021 needed).
- **3.1** — `derive_quantity_scheme_from_system_overlay` (LitRPG/cultivation tiers → bands).
- **3.2** — `scan_scene_prices` + the pure `extract_price_mentions` extractor.
- **3.3** — the `currency_consistency` check (cross-denomination price-fact contradictions, via
  `QuantityScheme::amount_in_base`).
- **3.4** — the `affordability` advisory check (a priced purchase above a present character's
  tracked wealth, Info severity).
- **3.5** — branch-merge carrying of schemes + stamped state.

Load-bearing references were verified directly against the code; secondary anchors are marked `~`
where not re-opened by hand. Where the code finalized names or approach differently from a section
below (e.g. 2.4 as a pure read; the commit gate at `commit_quantity_state` rather than
`commit_scene_changes`; 3.2/3.4 as deterministic structured checks rather than the NL parsing the
draft hedged on), **the code is canonical.**
**Mandate:** keep *tracked quantities* — money/wealth, named prices, and progression systems
(LitRPG levels, cultivation realms, reputation) — coherent across **hundreds of chapters and
multiple books**, the same way `continuity-timing-design.md` keeps the in-world clock coherent.

This document specifies an additive **quantity layer**: a stamped per-subject quantity state, an
optional per-project quantity scheme, the **drift** and **write-time** defenses that reuse the
existing Phase-4 / commit-gate rails, and the surfacing that puts wealth in front of the drafting
model — sequenced NOW / NEXT / LATER, with the verified asymmetry that makes the ordering
non-negotiable.

> Design rule throughout: **additive and optional**. No existing project changes behavior; every
> new column is nullable; a book that never declares a quantity scheme behaves exactly as today.
> Money is the **first vertical**, but the primitive is generic — `system_overlay` (LitRPG /
> cultivation stats) is its second consumer for free.

> Scope honesty up front: fiction does **not** track wealth to the coin ("she paid the innkeeper
> and pocketed the change"). A double-entry ledger would flood with false positives — the exact
> failure mode the timing doc avoided when it chose fixed-marker scanning over NL parsing and
> *deferred* duration/age/season checks. The model here is **tier/band + named-price**, not
> accounting; prose-level affordability arithmetic is explicitly deferred (LATER, advisory).

---

## 0. Design goals — several drifting-quantity shapes, one model

| Shape | What it needs |
| --- | --- |
| **Secondary-world economy** (coins, named prices, trade goods) | consistent *named prices* and currency/denomination across books; relative scale enforced (a destitute character can't quietly buy a castle) |
| **Long-arc character / party wealth** | a stamped wealth *tier/amount* that evolves with the story and is surfaced while drafting; "how rich is she now?" answerable without rereading |
| **Progression systems** (LitRPG levels, cultivation realms, mana, reputation) | monotonic-ish tracked quantities with named bands; "can't silently jump Bronze→Legendary"; the inert `system_overlay` finally enforced |

Unifying abstraction: a **stamped quantity-state** — `(subject, measure, amount, band)` placed at a
story position — governed by an optional per-project **quantity scheme** (currency denominations /
stat bands / tier ladder). Every shape reduces to "this subject holds this measure at this
amount/band as of this scene; here are the legal transitions and the relative scale." This is the
quantity analogue of the timing doc's "total-order in-world index … governed by a per-project
calendar": there, a stamped *instant* governed by a calendar; here, a stamped *amount* governed by a
scheme.

---

## 1. Current state — verified

### 1.1 What already exists (genuine strengths — build on these)

The *value-canon* half of money continuity is **~80% built** and unused:

- **Typed numeric canonical facts.** `RegisterCanonicalFactInput` (`models.rs:5353`) already carries
  `value_kind` / `value_number: f64` / `value_unit`, plus `subject_table` / `subject_id` /
  `predicate`, `aliases`, and `valid_from` / `valid_until` / `supersedes_fact_id`. A price —
  *"subject=economy, predicate=bread_price, value_number=5, value_unit=silver, aliases=[loaf,bread],
  valid_from=b1/ch3"* — is registerable **today**, and the `canonical_fact_prose_drift` Phase-4
  validator already scans prose for contradictions against it via `fact_at_or_before`. The
  infrastructure exists; nothing populates or surfaces it for economics.
- **Append-only, position-stamped `character_state`** (`records.rs:362`; `CommitCharacterStateInput`
  at `models.rs:1820`, patch shape `CharacterStatePatch` at `models.rs:127`). This is the **exact
  stamping precedent** a quantity-state mirrors: `(project, branch, character, scene, book, chapter,
  scene_order, payload)`, idempotent per position. Its `emotional_state: BTreeMap<String, Value>` is
  untyped and *unvalidated* — it could hold `{"gold": 50}` today, but nothing checks it.
- **Phase-4 validator harness.** `PhaseFourCacheId` (`service.rs:18396`) with content-hash +
  context-hash caching and entity-edit invalidation; `ChronologyDrift` was just added as the 6th
  member, proving the slot-in pattern (new id → register in `validators.rs` → `as_str`/`all`/
  `PHASE_FOUR_CHECK_TYPES` → context hash → `phase_four_cache_target_for_entity_update`).
- **Hard-constraint injection precedent.** The timing work added `temporal_hard_constraint`,
  prepended into `get_scene_context`'s non-truncatable prefix (~4000-token headroom). A quantity
  hard-constraint follows the identical path.
- **Write-time commit gate.** `commit_scene_changes` already gained `accept_continuity_risks` +
  `CommitContinuityGate{ Off, WarnOnly, BlockErrors }`; a price/quantity contradiction can ride the
  same ratchet without a new mechanism.

So the canon-value and enforcement *rails* are laid. The missing primitive is **subject quantity
STATE**, a **scheme**, a **drift check**, and **surfacing**.

### 1.2 The quantity axis is present-but-inert

Unlike time (which was *structurally absent*), quantity exists as **static string lore that never
reaches the loop**:

- **`Economy` is descriptive only.** `name`, `summary`, `scarce_resources: Vec<String>`,
  `trade_goods: Vec<String>`, a `currency` *name* string, `notes` (`records.rs:687`; schema
  `V0001:~309`; `CreateEconomyInput` at `models.rs:2624`). No amounts, no prices, no exchange rates,
  no time-series. **Never injected into scene context** — the drafting model never sees it.
- **`system_overlay` is static too.** `progression_currency: Option<String>`, `stats: Vec<String>`,
  `advancement_tiers: Vec<String>` — all *names* (`models.rs:3622`). A context hint with **zero
  enforcement**: a character can jump from a Bronze tier to Legendary in one scene and produce no
  finding.
- **Characters have no wealth/inventory field.** `character_state` tracks
  emotional_state/goals/status/notes only; there is no API to commit "now holds 50 coins" or
  "acquired a sword."
- **Import mines nothing quantitative.** `extract_economy_seeds` (`import/world.rs:~576`)
  keyword-spawns empty `Economy` stubs from words like "coin"/"ledger"/"tithe" at confidence ~0.72;
  it extracts **no prices, amounts, or denominations** from prose.
- **No economic check exists.** `check_consistency` runs ~18 checks (scene spine, promises, pacing,
  chronology, knowledge-timing, world-rule compliance, canonical-fact consistency, …) — **none**
  touch money, prices, wealth, or progression.

### 1.3 What actually drifts at scale (the bug class)

1. **Price drift.** The same named good costs wildly different amounts across chapters with nothing
   to catch it. *Registerable as a fact today, but with no extraction and no surfacing it never
   happens in practice* — the capability is dormant.
2. **Scale / affordability impossibility.** A character the prose itself established as destitute
   makes a purchase that contradicts their established means; wealth tier and named prices are never
   reconciled.
3. **Denomination / currency contradiction.** "Gold crowns" in book 1, "silver marks" in book 3 for
   the same realm; multiple currencies with no declared relation, so cross-currency mentions can't
   be reconciled.
4. **Progression drift.** A LitRPG/cultivation band regresses or skips levels; reputation flips
   without cause. `system_overlay` carries the band names but enforces nothing.

> **The honest asymmetry vs time (drives every scoping call below).** Time has a *closed,
> deterministic* structure — a calendar is a total order you can compute drift against to the
> minute. Money does not: most fiction tracks *relative scale and the consistency of named prices*,
> not balances. Therefore the high-precision / low-flood target is **tier-band + named-price**, and
> the things that need NL extraction of "spends N" — exact affordability arithmetic — are deferred
> exactly as the timing doc deferred prose age/duration parsing.

---

## 2. Workstreams at a glance

| Tier | Item | Effort | Migration |
| --- | --- | --- | --- |
| **NOW** | 1.1 Surface existing economic canon + activate price facts (Economy → scene context; price-fact recipe) | S | — |
| **NOW** | 1.2 Quantity-state data model (`QuantityState` + `project_quantity_scheme`) | L | **V0020** |
| **NOW** | 1.3 Quantity write/read MCP tools + per-subject "current amount" read | M | — |
| **NEXT** | 2.1 `QuantityDrift` validator (7th `PhaseFourCacheId`) — price + band + denomination sub-checks | L | — |
| **NEXT** | 2.2 Quantity hard-constraint render (the prevention mechanism) | M | — |
| **NEXT** | 2.3 Write-time quantity gate on `commit_scene_changes` (reuse `CommitContinuityGate`) | M | — |
| **NEXT** | 2.4 Per-book wealth/progression digest rollup (fold into `save_summary`) | M | **V0021** |
| **LATER** | 3.1 Activate `system_overlay` as a scheme consumer (the second vertical) | M | — |
| **LATER** | 3.2 Import extraction of prices/denominations → price facts | L | — |
| **LATER** | 3.3 Exchange-rate / multi-currency consistency | M | + migration |
| **LATER** | 3.4 Prose affordability arithmetic (advisory tripwire) | L | — |
| **LATER** | 3.5 Branch merge/diff coverage for quantity tables | S | — |

NOW item 1.1 needs no migration and delivers value before any schema change — it is the proof that
the dormant `canonical_fact` numeric surface, once *surfaced*, prevents price drift. 1.2 is the
foundation the NEXT tier consumes.

---

## 3. NOW tier — surface the dormant canon, lay the foundation

### 1.1 Surface existing economic canon + activate price facts (no migration)

> **Status: shipped.** `get_scene_context` adds the project's economies to the canonical-fact
> subject set, so economy-scoped numeric facts (named prices) render as hard constraints, and an
> `economy_briefing` Supplementary section carries the `Economy` lore. `get_writer_state` surfaces
> the same price facts in its re-anchor packet, and `get_chapter_briefing` inherits them via its
> embedded scene context (1.1c). The worldbuilder skill documents the price-fact recipe.

The cheapest, highest-leverage move: make what already exists *reach the drafting model*. Today
`Economy` and any registered price fact are invisible during scene drafting.

- **Inject economic canon into scene context.** In the scene-context assembly, render the project's
  `Economy` summary + active price facts (those with `value_unit`/currency at-or-before the cursor,
  reusing `fact_at_or_before`) as a `Supplementary` section — mirroring how `timeline_briefing`
  surfaces. This alone closes the "model never sees the price it set last chapter" gap.
- **Document the price-fact recipe.** Add a worldbuilder/continuity-editor skill step: a recurring
  price is a `register_canonical_fact` with `value_kind:"number"`, `value_unit:"<currency>"`,
  `aliases:["loaf","bread"]`, and a `valid_from` placement; a price *change* supersedes via
  `supersedes_fact_id` + `valid_until`. The `canonical_fact_prose_drift` validator then catches
  contradicting prose for free.
- **Surface in `get_writer_state` / `get_chapter_briefing`** an "economy in play" line when price
  facts or an `Economy` exist, so the re-anchor packet carries it.

> This item is intentionally schema-free. It validates the whole thesis — that named-price drift is
> preventable on existing rails — before committing to the V0020 data model.

### 1.2 Quantity-state data model — `QuantityState` + `project_quantity_scheme` (migration V0020)

> **Status: shipped.** `QuantityScheme`/`QuantityState` (with validation) live in `spindle-core`;
> migration `V0020__quantity_state.sql` adds `project_quantity_scheme` + the append-only stamped
> `quantity_state` table; `StoredQuantityScheme`/`StoredQuantityState` records and repository
> methods (`upsert/get/list_project_quantity_scheme`, `append_quantity_state`,
> `latest_quantity_state_at_or_before`) mirror the V0017 clock plumbing. Field names below were
> finalized slightly differently in code (e.g. `QuantityDenomination.per_base`); the code is canonical.

**Core DTOs** (`spindle-core/src/models.rs`; MCP stays thin per `CLAUDE.md`):

```text
QuantityState {                         // stamped, append-only — mirrors character_state
  subject_table:  String,               // "character" | "faction" | "location" | ...
  subject_id:     String,
  measure:        String,               // "wealth" | "mana" | "reputation:guild" | "<stat>"
  amount:         Option<f64>,          // optional exact value (rarely needed for prose)
  unit:           Option<String>,       // currency/stat unit; ties to the scheme
  band:           Option<String>,       // the PRIMARY signal: "destitute" | "Bronze" | "rank-3"
  change_reason:  Option<String>,       // marks a legitimate jump (the band analogue of temporal_mode)
  // position is stamped exactly like character_state: book_number, chapter_number, scene_order, scene_id
}
QuantityScheme {                        // per-project, per measure-family — mirrors CalendarDef
  measure:        String,               // "wealth" | "cultivation" | ...
  denominations:  Vec<Denomination>,    // { name, per } e.g. gold=100 silver, silver=10 copper
  bands:          Vec<Band>,            // ordered { name, lower_bound? } — destitute<comfortable<wealthy
  max_band_jump:  Option<i32>,          // default 1: how many bands one stamp may cross unmarked
}
```

Mirror a `StoredQuantityState` / `StoredQuantityScheme` in `records.rs`, parallel to the
`Stored*Clock` family the timing work added.

**Migration `V0020__quantity_state.sql`** (V0019 is the verified latest on disk):
- new `project_quantity_scheme` table: `(project_id, branch_id, measure)` key, FK `ON DELETE
  CASCADE`, JSON `denominations`/`bands` with `CHECK(json_valid(...))`, `max_band_jump` INTEGER,
  timestamps.
- new `quantity_state` table: stamped like `character_state` — `id` PK `CHECK LIKE 'quantity_state:%'`,
  `project_id`/`branch_id` FK, `subject_table`, `subject_id`, `measure`, nullable `amount REAL`,
  `unit TEXT`, `band TEXT`, `change_reason TEXT`, `book_number`/`chapter_number`/`scene_order`,
  `scene_id`, timestamps. **All quantity columns nullable; append-only.**
- indices: `idx_quantity_subject(project_id, branch_id, subject_table, subject_id, measure,
  book_number, chapter_number, scene_order)` for the "current amount at cursor" read, and
  `idx_quantity_scheme(project_id, branch_id, measure)`.

**Write-time validation only:** reject a negative `amount`, a `band` not in the declared scheme's
ordered list, and a `denominations`/`bands` set that isn't internally ordered. The scheme is **not**
required to draft — fully opt-in.

**Out of scope for this migration:** the drift validator, the hard-constraint render, the commit
gate, and any prose affordability logic. This PR ships the primitive only (same discipline as the
timing doc's V0017).

### 1.3 Quantity write/read tools

> **Status: shipped** (core tools). `set_project_quantity_scheme` (validates + upserts a scheme) and
> `commit_quantity_state` (validates a reading against the declared scheme, then appends a stamped
> row) are registered in `tools.rs` after the clock tools. The `set_subject_band` convenience and
> writer-state surfacing of *quantity* state (distinct from economy price facts, already shipped in
> 1.1c) are deferred to the NEXT tier alongside the validator.

DTOs (`spindle-core/src/models.rs`; register in `tools.rs` after the clock tools via the
`self.invoke` dispatch): `SetProjectQuantitySchemeInput/Output`, `CommitQuantityStateInput/Output`
(`subject + scene_id + QuantityState`-shaped patch, stamped like `commit_character_state`),
`SetSubjectBandInput` as a convenience for the band-only common case.

Reads (pure, no new table): **current amount/band at cursor** —
`SELECT … FROM quantity_state WHERE project_id=? AND branch_id=? AND subject_table=? AND
subject_id=? AND measure=? AND (book,chapter,scene_order) <= cursor ORDER BY … DESC LIMIT 1`
(branch-scoped, like the per-book elapsed rollup). Each setter invalidates `QuantityDrift` once that
validator ships (until then, no-op). **Drafting-model surface:** an "in-world wealth/state" line in
`get_chapter_briefing` and `get_writer_state`, emitted only when a scheme + ≥1 state exist. Update
`docs/spindle-architecture.md`, `docs/spindle-implementation-brief.md`, and the
`worldbuilder`/`scene-writer`/`continuity-editor` skills per `CLAUDE.md`.

---

## 4. NEXT tier — where enforcement materializes

### 2.1 `QuantityDrift` validator (7th `PhaseFourCacheId`)

Add `QuantityDrift` to `PhaseFourCacheId` (`service.rs:18396`) with `as_str`/`all`/
`PHASE_FOUR_CHECK_TYPES` entries; register `QuantityDriftValidator` in `validators.rs` exactly as
`ChronologyDriftValidator` was. **MVP sub-checks only:**

- **Price consistency.** A named good's value in prose contradicts the active price fact at the
  cursor. This is largely *reuse* of `canonical_fact_prose_drift` specialized to `value_unit` /
  currency — prefer extending the existing fact validator's numeric path over a parallel scanner.
- **Band monotonicity.** A subject's `quantity_state.band` crosses more than `max_band_jump`
  ordered bands between consecutive in-manuscript stamps **without** a `change_reason` — the direct
  analogue of the chronology validator's backward-jump check gated by `temporal_mode`. Severity
  Warning.
- **Denomination / currency consistency.** A realm's currency in prose or in a new `Economy.currency`
  contradicts the declared scheme denominations.

**Defer** prose affordability arithmetic ("spent 100, had 50") — it needs the NL amount-extraction
the timing doc deferred for age/duration, and would flood. (LATER 3.4, advisory.)

**Correctness guard:** strict no-op unless a `project_quantity_scheme` **and** ≥1 `quantity_state`
or price fact exist (mirror the `StyleCompliance` early-return) → **zero new findings for non-economy
projects.** Plumbing mirrors `ChronologyDrift`: snapshot structs in `registry.rs`, scheme added to
`ValidatorContext`, scheme + amounts/bands hashed into the Phase-4 context hash, and
`phase_four_cache_target_for_entity_update` extended so quantity/scheme edits invalidate only
`QuantityDrift`.

### 2.2 Quantity hard-constraint render (the prevention mechanism)

Storing a band is inert until the model sees it. Add
`quantity_hard_constraint(subject_states, scheme) -> Option<HardConstraint>` in `format.rs`,
returning `None` when the project has no scheme (zero change for non-economy projects — unit-test on
an existing fixture). **Prepend** it to `hard_constraints` in `get_scene_context` so it rides the
non-truncatable prefix and the existing ~4000-token headroom, beside `temporal_hard_constraint`.
MVP string limited to data that exists: `"WEALTH/STATE: <Character> is <band> (~<amount> <unit>);
prices in play: <loaf 5 silver, …>. Do not contradict these amounts or this scale."` Inject into the
`caller_should_send_brief` path too, so Grok / a future Claude-CLI adapter receive it (it's derived,
not a single fetchable record).

### 2.3 Write-time quantity gate on `commit_scene_changes`

Extend the existing continuity precheck (no new mechanism): in the `scan_retcon_findings` path, after
the knowledge-timing loop, compare prospective price/quantity facts against the active scheme and
state, emitting new `RetconFinding` variants `PriceContradiction{ good, prose_value, canon_value,
unit }` and `BandJumpUnexplained{ subject, from_band, to_band }`. These ride the existing
`CommitContinuityGate` (`BlockErrors` default, overridable via `accept_continuity_risks`), so an
unattended run cannot bake in a price contradiction between manual `check_consistency` runs.

### 2.4 Per-book wealth/progression digest rollup (migration V0021)

Fold a per-book quantity digest into `save_summary`'s existing writer transaction (atomic, idempotent
re-derive), exactly as `BookDigest` was: a deterministic summary of band/amount transitions per
tracked subject — *"book 1: the party went destitute→comfortable; bread 5→8 silver (famine)."*
Inject above `book_outline` in the chapter briefing and into the cross-book `SceneContextNovelLayer`,
so book 3 drafting still knows the party's book-1 wealth trajectory. Migration `V0021__quantity_digest.sql`
(serializes after V0020). Scope strictly as memory-rollup — do **not** overload it with enforcement.

---

## 5. LATER tier — round out coverage

- **3.1 Activate `system_overlay` as a scheme consumer (the second vertical, the payoff-twice
  item).** Map `progression_currency`/`stats`/`advancement_tiers` (`models.rs:3622`) onto a
  `QuantityScheme` (`measure` per stat, `bands` = `advancement_tiers`), and back each tracked stat
  with `quantity_state`. The band-monotonicity check then enforces LitRPG/cultivation progression
  with no new validator — the generic primitive pays for itself a second time.
- **3.2 Import extraction of prices/denominations.** Extend the import path to mine `"N <currency>"`
  patterns and denomination names from prose and register them as price facts with placement windows
  — the economic analogue of auto-stamping `day_index`, and deferred for the same reason (author/loop
  declaration first; NL extraction is lossy). Keep extracted facts low-confidence and review-gated.
- **3.3 Exchange-rate / multi-currency consistency.** Let `QuantityScheme.denominations` express
  cross-currency conversion; flag prose that converts at a rate contradicting the scheme. Needs a
  small schema addition for inter-currency rates.
- **3.4 Prose affordability arithmetic (advisory tripwire).** The explicitly-deferred hard part:
  detect "spent N when holding M". `normalized_fact`-style phrase matching has near-zero verbatim
  recall, so this is a **high-precision/low-recall tripwire — Info severity, never a hard gate**
  (same guardrail as the knowledge-timing scan). Promotable behind the override flag only.
- **3.5 Branch merge/diff coverage.** Add `quantity_state` + `project_quantity_scheme` to the
  `merge_branch`/`diff_branches` canon snapshot with conflict detection — the same gap the recent
  "carry all canon tables across branch merge" work closed for the timing tables; do not let the new
  tables silently drop on merge.

---

## 6. Sequencing & dependencies (non-negotiable order)

1. **1.1 surfacing → before any schema.** It proves named-price drift is preventable on existing
   rails and de-risks the V0020 investment; ship it first regardless.
2. **1.2 (V0020) → before all NEXT-tier enforcement** (no state table, no drift check, no gate).
3. **Settled — band-primary, amount optional** (see §9.1). The whole false-positive budget hinges on
   this; build the model band-first, with `amount`/`unit` as an opt-in refinement for systems that
   genuinely count.
4. **Migrations serialize:** next free number is **V0020** (`quantity_state`), then **V0021**
   (`quantity_digest`); 3.3 adds a later one. V0019 is the latest on disk — the runner fails on
   duplicate numbers.
5. **Skills/docs adoption is part of the work, not after it:** the scheme produces **zero**
   protection until authors/the loop populate bands and price facts. Budget `worldbuilder`
   (it already says to encode economy as `world_rule`s — extend that), `scene-writer`,
   `continuity-editor` skill + `docs/` updates alongside 1.1/1.2/1.3.

---

## 7. Risks

- **False-positive flood (dominant).** Exact-amount affordability tracking fires on every "pocketed
  the change." Mitigation: band-primary model, named-price facts only, affordability deferred to an
  advisory tripwire. This is the money analogue of the timing doc's dominant "evolving-fact false
  positives" risk — and the reason the whole design is tier-shaped.
- **Genre mismatch.** Most literary/contemporary fiction wants none of this. Hard requirement: fully
  additive/opt-in — a no-scheme project must run the full suite green with zero quantity findings
  (the additivity test).
- **Opt-in dormancy.** Identical to the clock: no protection until schemes/states are populated;
  skill + authoring-loop adoption is what realizes the value.
- **Low-recall price matching.** Named-good aliases in prose have the same near-zero verbatim recall
  as the knowledge scan; keep price-prose drift advisory by default — a tripwire, not a guarantee.
- **Two models for one thing.** `Economy` and `system_overlay` already exist as lore. The scheme must
  *subsume* them (be the enforcement layer they reference), not sit beside them as a third
  overlapping concept. 3.1 makes `system_overlay` a consumer rather than a competitor.
- **Cache cost at scale.** Adding `QuantityDrift` to the Phase-4 context hash means quantity/scheme
  edits invalidate caches; combined with the existing branch-wide blow-away on `register_canonical_fact`,
  each edit can trigger broad re-validation. Validate invalidation granularity before adding the
  cached validator (same caveat the timing doc raised for `ChronologyDrift`).

---

## 8. Test plan

- **Record round-trip:** `quantity_state` and `project_quantity_scheme` with and without values
  survive save→load; stamped positional columns intact.
- **Scheme:** a coin scheme (gold>silver>copper denominations) and a LitRPG band ladder validate and
  render; a `band` outside the declared ordered set is rejected at write time.
- **Surfacing (1.1):** an `Economy` + active price fact appear in scene context; a price fact whose
  `valid_until` precedes the cursor does not.
- **Price consistency:** contradicting bread price across chapters flagged; consistent price clean; a
  famine price change via `supersedes_fact_id`/`valid_until` not flagged.
- **Band monotonicity:** an unmarked 2-band jump flagged; the same jump with a `change_reason` clean;
  a within-`max_band_jump` step clean.
- **Hard constraint:** lands in the non-truncatable prefix and the brief path; survives a tight
  budget; absent when no scheme.
- **Write-gate:** a price contradiction blocks (and is overridable via `accept_continuity_risks`); a
  legitimate band advance with `change_reason` does not block.
- **system_overlay-as-scheme (3.1):** a tier skip is flagged once the overlay backs a scheme.
- **No-economy project:** full suite green with zero scheme/state set (proves additivity).

---

## 9. Decisions taken

These were flagged as open during design; they are now **decided** so the roadmap is
execution-ready. §9.1 is architectural (load-bearing — change only with strong cause); §9.4–9.5 are
tunable defaults (revisit freely once real projects exercise them).

1. **Model: band-primary, amount optional** *(architectural)*. `band` is the canonical, prose-
   assertable signal and the one the validator checks; `amount`/`unit` are an opt-in refinement for
   systems that genuinely count (LitRPG mana, a heist's take). This is what keeps the false-positive
   flood out and what lets `system_overlay` reuse the primitive (3.1) — every other choice in this
   doc assumes it.
2. **Scheme storage: a dedicated `project_quantity_scheme` table** — not columns on `project`, not
   an overload of `Economy`. Keeps schemes per-measure and branch-scoped, mirroring how
   `project_calendar` is its own table.
3. **`Economy` / `system_overlay` stay descriptive and *reference* a scheme** — no destructive
   migration of their string fields. The scheme is the enforcement layer they point at; the existing
   lore entities keep their current shape and meaning.
4. **Subject coverage starts at `character` + `faction`** *(default)* — party wealth and faction
   treasury. Add `location` (a city's prices) only if demand appears; the `subject_table` column
   makes that purely additive.
5. **Gate defaults** *(default)* — `BlockErrors` for price/denomination contradictions
   (deterministic, safe to block), `WarnOnly` for band jumps (judgment-laden), affordability always
   advisory. All overridable via `accept_continuity_risks`.

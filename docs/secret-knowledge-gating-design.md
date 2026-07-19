# Secret knowledge gating — circle-of-trust design

**Status:** design draft (no implementation yet). Every code reference verified on branch
`feat/automated` during the P0 evolution build (post-`eb3f92f` working tree).
**Mandate:** a fact held in confidence by some characters must not leak into other characters'
dialogue, behavior, or non-POV narration until the **circle of trust** expands to include them —
and expansion happens only through an explicit, recorded reveal.

**Running example (the owner's):** a reincarnated character has told no one. The fact shapes her
choices and interiority, but it is not common knowledge. Today, drafts drift into other characters
knowing it — because the model was shown the fact with no audience boundary. She may reveal it
eventually, to one person; from that scene on, exactly two people may act on it.

> Design rule (house standard): **additive and optional.** Facts are public by default; a project
> that never marks a secret behaves exactly as today. Every new column is nullable/defaulted.

---

## 1. Current state — verified

### 1.1 What exists (the machinery to build on)

- **Per-character knowledge rows.** `knowledge_fact(project_id, branch_id, character_id, fact,
  normalized_fact, learned_at, confidence, tags, reader_visible)` with a unique index on
  `(project, branch, character, normalized_fact)` (V0001 L641-659). This is already a
  circle-of-trust representation: *the set of characters holding a row for a fact IS its circle.*
- **Reveal timing.** `learned_at: Option<StoryPlacement>` on each row — circle membership is
  placement-stamped, so "B learns it in ch 12" is representable today via `record_knowledge`.
- **Holder-direction enforcement.** The `knowledge_timing` check flags a scene where a character
  references knowledge they haven't learned *yet* (cursor-gated), and `future_knowledge` is
  sealed per-character with `learned_at <= cursor` filtering in context assembly.
- **Reader-knowledge control.** `reader_visible` on knowledge rows — dramatic irony is already a
  modeled concept.

### 1.2 The leak vector (why drafts drift)

1. **Canonical facts are global.** `register_canonical_fact` rows have no audience scoping; scene
   context surfaces them in hard constraints and subject snapshots to the model for *every* scene
   the subject appears in. The model is told "Mara is reincarnated" as ambient world-truth while
   drafting a scene where only out-of-circle characters share the room — nothing marks the fact as
   privileged, so B reacting to it is a perfectly reasonable completion.
2. **All checks look in the holder direction.** `knowledge_timing` asks "does the *speaker* know
   this yet?" No check asks the audience question: "did an out-of-circle character reference or
   act on knowledge they were never given?"
3. **No draft-time instruction.** Even when the fact must be in context (the holder is present and
   her interiority depends on it), nothing tells the drafter who may and may not act on it.

The failure is structural, not a model defect: we hand the model a secret with no envelope.

---

## 2. Design

Two principles order everything:

- **P-A: What the model never sees, it cannot leak.** The strongest gate is withholding — used
  whenever no circle member is in the scene.
- **P-B: When the model must see it, it gets an envelope, and prose gets audited.** When a circle
  member is present, the secret ships as a hard constraint with an explicit audience boundary, and
  the audience-direction check verifies the boundary held.

### 2.1 Declaring a secret

`register_canonical_fact` gains an additive `secrecy` input:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecrecyScope {
    /// Characters who know at declaration time (the initial circle).
    pub holder_ids: Vec<String>,
    /// Optional guidance rendered into the envelope, e.g. "she deflects
    /// questions about her past with dry humor".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concealment_note: Option<String>,
}
```

Storage: `canonical_fact.secret INTEGER NOT NULL DEFAULT 0` + nullable
`canonical_fact.concealment_note TEXT` (additive migration). Declaring secrecy writes one
`knowledge_fact` row per holder with a new nullable link column
`knowledge_fact.secret_of_fact_id TEXT REFERENCES canonical_fact(id)` — the circle is thereafter
**derived, never duplicated**: `circle(fact) = characters with a knowledge_fact row where
secret_of_fact_id = fact.id`. The existing unique index and `learned_at` semantics apply
unchanged. (Anchoring on the fact id, not `normalized_fact` text-matching, keeps circles robust
against paraphrase — "is reincarnated" vs "was reborn".)

Existing secrets in existing projects: `update_entity` on the canonical fact + `record_knowledge`
with the link id retrofit any fact into a secret without migration of data.

### 2.2 The context gate (assembly rules, per scene)

For each canonical fact with `secret = 1`, during `get_scene_context`:

| Condition (scene cast ∩ circle, at this scene's story cursor) | Behavior |
| --- | --- |
| **No circle member present** (and POV ∉ circle) | **Withhold entirely** (P-A). The fact is stripped from hard constraints, subject snapshots, semantic recall results, STORY SO FAR open-threads rendering, and the knowledge briefing. The model cannot leak what it never saw. |
| **≥1 circle member present** | Render in a new hard-constraint block **`[SECRETS IN PLAY]`** (P-B): the fact, `Known ONLY to: <circle ∩ known-at-cursor>`, `Present and NOT in the know: <cast − circle>`, `These characters must not reference, imply, or react to this — they do not know it.` plus the concealment_note when set. |
| **POV ∈ circle, no other circle member present** | Same envelope; add `Narration may carry her private awareness; dialogue and other characters' behavior must not.` |

Cursor-awareness: circle membership is evaluated at the scene's story position via the existing
`learned_at <= cursor` filtering (same machinery as `future_knowledge`) — a reveal in ch 12 does
not leak backward into a ch 9 flashback drafted later.

Cross-surface consistency (the part that's easy to miss): withholding must cover **every** context
carrier — the fact must also be filtered from `semantic_references` hits, digest `open_threads`
strings, and `previous_scene_tail` excerpts are exempt (they are prose the reader already has).
The filter lives in ONE place (a `SecretVisibility` resolver the assemblers call), not scattered
per-section.

### 2.3 Reveals (circle expansion)

A reveal is not a new mechanism — it is `record_knowledge` with `secret_of_fact_id` set and
`learned_at` = the reveal scene's placement. Three paths produce one:

1. **Manual:** the author calls `record_knowledge` (works today, minus the link column).
2. **Draft-time:** the scene-writer skill instructs the drafter to flag an on-page reveal in the
   continuity package via `knowledge_learned` entries on `save_scene_draft` (added by the NEXT
   wave — the original draft of this doc wrongly assumed the field pre-existed; `save_scene_draft`
   carried no package at all). The save path pre-flight-validates secret links before any write
   and records entries through the same path as `record_knowledge`.
3. **Mined (P1):** the canon-mining pass detects a reveal in committed prose and stages a
   `knowledge_learned` delta carrying `secret_of_fact_id`; ratification expands the circle. The
   `canon_delta` class list in `spindle-evolution-design.md` §3.1 already includes
   `knowledge_learned` — no new class needed.

Off-page reveals ("between chapters, she told him") are supported: `record_knowledge` with a
placement between scenes and a `source_summary` saying so.

### 2.4 The audience-direction check: `secret_leak`

Complement to `knowledge_timing` (holder-direction). Two tiers, following the established pattern
(deterministic always; model-backed behind `deep_check` with honest-skip semantics per
`promise_payoff_detection`):

- **Deterministic tier:** for each scene in scope, for each secret fact whose circle does NOT
  cover a present character: scan that character's dialogue lines (the voice-drift check already
  attributes dialogue per character — reuse its attribution) and the scene prose for the fact's
  key/value lexemes (reuse `canonical_fact_prose_drift`'s matching, inverted: a *hit* by an
  out-of-circle speaker is the violation). Severity `warning`, message naming the fact, the
  character, and the reveal status ("no recorded reveal to <char> before this scene").
- **Deep tier (`deep_check=true`):** model-backed pass asking the audience question directly —
  "does any out-of-circle present character reference, imply knowledge of, or act upon X?" —
  catching behavioral leaks lexeme matching can't (B avoiding the graveyard she has no reason to
  avoid). One call per (scene, secret-set), strict parse, findings `warning`, honest skip finding
  when no review route resolves. Rating discipline: this pass carries scene prose ⇒ it resolves
  through the rating-gated dispatch chokepoint like every prose-bearing pass (evolution design §4).
- **False-positive escape:** a finding may be resolved as *deliberate* (dramatic irony, the
  character guessed) by recording the reveal or dismissing with a note — mirroring how other
  checks' findings are triaged at checkpoints.

The in-run verify loop (evolution P2) picks `secret_leak`'s deterministic tier up automatically
once it's in the scene-scoped subset — a leak then triggers a revision directive while the
context is hot, which is the real payoff of P-B.

### 2.5 What this is NOT (non-goals)

- **Not omniscient-narrator policing.** Projects with an omniscient narrator that confides in the
  reader set `reader_visible = true` on the holders' rows; the gate governs *characters*, not the
  reader. Dramatic irony is a supported feature, not a violation.
- **Not automatic secret detection (v1).** Marking a fact secret is an authorial act. (LATER:
  mining may *suggest* secrecy when it extracts a fact from a scene where only one character
  could know it.)
- **Not deception modeling.** Who *believes* what false thing is a different system (belief vs
  knowledge). Out of scope.

---

## 3. Schema summary (one additive migration)

```sql
ALTER TABLE canonical_fact ADD COLUMN secret INTEGER NOT NULL DEFAULT 0;
ALTER TABLE canonical_fact ADD COLUMN concealment_note TEXT;
ALTER TABLE knowledge_fact ADD COLUMN secret_of_fact_id TEXT REFERENCES canonical_fact(id);
CREATE INDEX idx_knowledge_fact_secret_link ON knowledge_fact(secret_of_fact_id)
    WHERE secret_of_fact_id IS NOT NULL;
```

No new tables. Circle = derived join. Migration number assigned at implementation (V0023 if free —
V0022 was taken by the thread-audit field bundle).

---

## 4. Roadmap

**NOW (the gate itself):**
1. Migration + `SecrecyScope` on `register_canonical_fact` + link-column plumbing +
   `update_entity` retrofit path. Tests: declaration writes holder rows; circle derivation;
   old rows unaffected.
2. `SecretVisibility` resolver + context-gate rules in `get_scene_context` (withhold / envelope /
   POV-only variants, cursor-aware), covering hard constraints, snapshots, knowledge briefing,
   semantic recall, and digest open-threads rendering. Tests per row of the §2.2 table + a
   flashback cursor test.
3. `secret_leak` deterministic tier + continuity-editor skill section.
4. Scene-writer + supervisor skill guidance: the envelope block's meaning, and flagging on-page
   reveals in the continuity package.

**NEXT:**
5. `secret_leak` deep tier (rating-gated, honest-skip).
6. `save_scene_draft` continuity-package reveal linking (path 2 of §2.3).
7. Reveal-aware briefing: when a scene plan *targets* a reveal (plan-level intent), the envelope
   flips to "this scene MAY reveal X to Y — write the reveal".

**LATER:**
8. Mining-suggested secrecy; secrecy on world rules and relationship facts (an affair is a secret
   *relationship*, not a secret fact — needs the same envelope on relationship rows).
9. Belief/deception modeling (explicit non-goal until secrets prove out).

## 5. Invariants (inherited from the evolution design)

- Secrets never appear in run events, SSE payloads, or artifact *names* (ids only) — evolution §3.4.
- The deep tier routes through `resolve_cleared_route`; an explicit-rated scene's secret audit
  never reaches an uncleared model — evolution §4.
- All fields additive; a project with zero secret facts is byte-identical in behavior and context
  output to today (pinned by a no-secrets regression test in NOW-2).

---
name: worldbuilder
description: >
  Use when building or expanding any aspect of the story's world. This includes creating realms,
  locations, factions, religions, economies, technologies, magic systems, power systems, game
  mechanics (LitRPG/cultivation), world rules, glossary terms, and establishing the physical/social
  laws of the setting. Also triggers when the user says "build a magic system", "create a city",
  "I need factions", "what's the political structure", "how does magic work in this world",
  "design the economy", or any request about the setting, lore, or world logic. If anything
  about the world feels thin, inconsistent, or undefined, this skill fixes that.
---

# Worldbuilder

A world is not a backdrop — it's a character. The best settings shape conflict, constrain choices,
and make the impossible feel inevitable. This skill guides you through building worlds that serve
the story rather than existing as decoration.

## Tool workflow

Use this workflow to keep world updates traceable and enforceable:

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch) without re-passing `project_id`.
2. Call `find_entity` and `get_entity` before creating rules, factions, or
   locations to avoid duplicate canon.
3. Persist constraints with `create_world_rule` (and `create_system_overlay`
   when applicable) before scenes rely on them. Set `scan_pattern` on every
   rule whose violation should be detectable in prose — the
   `world_rule_semantic_drift` validator scans scene text for this anchor.
4. Edit existing rules with `update_world_rule { world_rule_id, changes }`,
   not the generic `update_entity` tool.
5. Use `record_knowledge` for canonical world facts that don't fit the
   `world_rule` shape (e.g. "the moon gate is sealed at dawn").
6. Use the batch creators when seeding glossary or motif content:
   `batch_create_terms`, `batch_create_motifs`.
7. Call `find_scenes_referencing` on the affected subject after major world
   changes to identify prose likely to drift.
8. Call `check_consistency` to validate world-rule compliance after updates.
   The `world_rule_semantic_drift` validator runs by default.
9. If prose and canon diverge, hand off scene updates to scene-writer or
   revision-manager and re-run checks.

## Sanderson's Laws of Magic (Applied to All Worldbuilding)

These three laws apply to EVERY system in your world — not just magic. Technology, politics,
economics, religions, and social structures all follow the same principles.

### Law 1: Transparency Determines Utility

An author's ability to solve conflict with a world system is directly proportional to how
well the reader understands that system.

- **Hard systems** (reader understands the rules): Magic, technology, or social structures
  can be used to solve problems. The reader feels clever, not cheated.
- **Soft systems** (reader doesn't understand the rules): Systems create wonder and atmosphere
  but CANNOT be used to solve the climax. Using unexplained power to resolve conflict
  is deus ex machina.
- **Most systems are mixed**: Some rules are explicit (hard), others mysterious (soft).
  The reader understands ENOUGH to follow the logic but not enough to predict everything.

**For every world system you build, decide: is this hard, soft, or mixed?** This determines
how the scene-writer skill can use it in the narrative.

### Law 2: Limitations > Powers

What the system CANNOT do is more interesting than what it can. Limitations create:
- **Struggle**: Characters must work around constraints, making them cleverer
- **Stakes**: If magic has a cost, every use is a choice with consequences
- **Tension**: Limitations create situations where the system can't save you

For every power, ability, or advantage in your world, define:
- **Costs**: What does using it consume? (Energy, lifespan, sanity, resources, time)
- **Limitations**: What can't it do? (Range, target type, duration, frequency)
- **Weaknesses**: What neutralizes or counters it? (Not just "kryptonite" — something organic)
- **Side effects**: What unintended consequences does it produce?

### Law 3: Expand Before You Add

Before inventing a new power, species, or system, ask whether you can expand or
recontextualize what you already have. A single magic system interpreted differently
by three cultures is richer than three separate magic systems.

**The Iceberg Method**: Only 10-20% of your worldbuilding appears on the page. The rest
exists in the Bible so you can maintain consistency and depth. The reader senses the
depth without being burdened by exposition.

---

## Building a Realm

A realm is a self-contained world with its own physics, magic, and rules.
Spindle does not ship a `realm` entity — instead, "realm" is an optional
string field on `create_location`, `create_faction`, `create_economy`, etc.,
and realm-level constraints are encoded as `world_rule` entries that all
subsequent locations and factions in that realm must respect.

### Step 1: Define the Realm

Pick a realm name and decide what makes it distinct. There is no separate
`create_realm` call — for each distinguishing property, create a `world_rule`
entry whose `rule_type` describes the kind of constraint:

- "Gravity is 1.5x Earth" → `physical_law`
- "Night lasts 18 hours" → `physical_law`
- "Travel between realms requires a moon-bound key" → `magic_limitation`
- "Time runs at 1/3 speed compared to the Mortal Realm" → `time_constraint`

Then pass the realm string to every location, faction, religion, and economy
you create in that realm so they're filterable.

### Step 2: Establish the Rule Set

For EVERY rule, limitation, or cost in the realm, call `create_world_rule`
with the typed input:

```
rule_name: "Magic requires physical contact"
rule_type: "magic_limitation"
description: "All magical effects require the caster to touch the target.
              Ranged magic is impossible. This forces mages into melee range."
scan_pattern: "magic"          // anchor the validator scans for in scene prose
relevance_tags: ["magic", "combat"]
established_in: { book_number: 1, chapter_number: 2 }
```

The `scan_pattern` field is what makes a world rule enforceable: the
`world_rule_semantic_drift` validator scans every scene for this term within
80 characters of "violate", "break", "ignore", or "without", and flags
violations in `check_consistency`. Without a `scan_pattern`, the validator
silently skips the rule.

**Critical rule types to define:**
- **magic_limitation**: What magic can't do, or the constraints on how it works
- **power_cost**: What using power consumes (health, time, resources, sanity)
- **ability_prerequisite**: What you need before you can use a power (training, bloodline, item)
- **technology_constraint**: What technology can or can't do in this world
- **physical_law**: Gravity, light, sound, time — anything that differs from Earth
- **social_law**: Enforced social contracts, laws, taboos that shape behavior
- **resource_scarcity**: What's rare, what's abundant, what's fought over
- **time_constraint**: How long things take (travel, healing, communication, construction)

**For each rule, plan when to DEMONSTRATE it on page.** A rule that's stated but
never shown is an empty decoration. The first time magic costs the caster something
real, the reader believes it. Capture that proof in later scene summaries,
annotations, and consistency review.

### Step 3: Create a System Overlay (if applicable)

For LitRPG, cultivation, or game-mechanic worlds, call `create_system_overlay`
to record the mechanical layer (system_name, system_type, rules, visibility,
etc.).

Then create `world_rule` entries for each system constraint so the consistency
validators can enforce them:
- Level caps per tier
- Experience diminishing returns
- Class restrictions
- Skill prerequisites
- Stat point allocation rules

For per-character or per-arc progression budgets, use `create_pacing_config`
and `set_arc_pacing_constraints` (covered in the plot-architect skill). There
is no `character_system_state` or `system_pacing_constraint` tool — those
concepts are expressed via pacing config + world rules + canonical facts
recorded with `register_canonical_fact` (predicate names like
`progression.level`, `progression.xp`).

---

## Building Locations

Every location should be a PLACE, not a label. A good location constrains action,
creates atmosphere, and tells you something about who lives there.

### Step 1: Create the Location

Call `create_location` with the typed input:
- **project_id** (required)
- **name** (required)
- **summary** (required)
- **kind** (defaults to inferred best-effort if omitted; accepts `type` as
  serde alias). Typical kinds: city, village, fortress, wilderness, ruin,
  ship, dungeon, market, gym.
- **realm** (`Option<String>`)
- **initial_state** (`WorldStateInput`, optional — see Step 2)

### Step 2: Establish Initial State

The `initial_state` field on `create_location` is a `WorldStateInput`
container with these fields:
- **controlling_faction** (`Option<String>`): Free-form name of the faction in
  control. This does NOT auto-create a `controls` edge — it's just text the
  scene context renders.
- **status** (`Option<String>`): "thriving", "occupied", "besieged",
  "abandoned", "contested"
- **prosperity** (`Option<String>`): Free-form descriptor (e.g. "thriving",
  "subsistence", "collapsed"). Not a numeric scale.
- **stability** (`Option<String>`): Same shape as prosperity.
- **threat_level** (`Option<String>`): "safe", "uneasy", "dangerous", "deadly"
- **sensory_details** (`Vec<String>`): Smells, sounds, light quality, texture,
  temperature. The scene-writer reads these to ground prose in physical
  reality.

### Step 3: Connect to Other Locations

`create_relationship` is character-to-character only. There is no shipped
tool for `borders`, `trades_with`, or other location-location edges. Encode
geographic adjacency and trade ties as `world_rule` entries with appropriate
`relevance_tags`, or as canonical facts via `register_canonical_fact`. For
travel time, use a `time_constraint` world rule — this prevents characters
from teleporting between cities and is enforced by the world-rule validator
when the rule has a `scan_pattern`.

### Sensory Design

When defining a location, include sensory details in the summary that the
scene-writer skill will draw on:
- What does it SMELL like? (smoke, salt air, rotting vegetation, baking bread)
- What does it SOUND like? (hammering, waves, silence, distant drums)
- What's the LIGHT quality? (torchlit, sun-bleached, overcast, bioluminescent)
- What's the dominant TEXTURE? (rough stone, slick metal, soft earth, crumbling brick)
- What's the TEMPERATURE? (stifling, chilly, bone-cold, uncomfortably warm)

These ground the reader in physical reality and prevent "white room syndrome."

---

## Building Factions

Factions drive political conflict. Every faction needs:

### Step 1: Create the Faction

Call `create_faction` with the typed input:
- **project_id** (required)
- **name** (required)
- **faction_type** (required) — examples: military, political, religious,
  criminal, merchant, revolutionary, secret_society
- **realm** (`Option<String>`)
- **summary** (required)
- **tags** (`Vec<String>`, defaults empty)

### Step 2: Define State and Relationships

Capture faction state in the summary and related world entities. There is no
dedicated `update_faction` tool — use the generic `update_entity` to amend a
faction's fields after creation.

There is no shipped tool for `controls`, `allied_with`, or `trades_with`
edges between factions and other entities. Encode these relationships as:
- Canonical facts (`register_canonical_fact` with predicate names like
  `faction.controls_location`, `faction.allied_with`)
- World rules (`create_world_rule` for political constraints like "House
  Vossal cannot enter the Free Cities without losing protection")

### Step 3: Faction Conflict Design

Every faction should have:
- **A goal** that conflicts with at least one other faction's goal
- **A method** that reveals their character (diplomacy, violence, manipulation, commerce)
- **A weakness** that can be exploited
- **Internal tension** — factions aren't monolithic. There should be moderates vs extremists,
  old guard vs reformers, loyal vs corruptible

---

## Building Magic / Power Systems

This is where Sanderson's Laws matter most.

### Step 1: Core Concept

What is the ONE interesting thing about this magic? Start with the most exciting idea
(Sanderson's Zeroth Law: err on the side of awesome).

Examples:
- "You can only use magic by consuming memories — the more powerful the spell, the more
  precious the memory you lose"
- "Magic is performed by singing, but the same song can never be sung twice"
- "Power comes from emotional bonds — the stronger the bond, the stronger the magic.
  But if the bond breaks, the backlash is devastating"

### Step 2: Define Limitations (More Important Than Powers)

For each power, create `world_rule` entries:

```
Costs:
  - "Healing magic accelerates the caster's aging. Heal a mortal wound, age 5 years."
    → rule_type: power_cost
  
Limitations:
  - "Magic cannot affect anything the caster cannot see."
    → rule_type: magic_limitation
  
  - "No spell can bring back the dead. Anyone who tries creates an undying abomination."
    → rule_type: magic_limitation
    
Weaknesses:
  - "Iron disrupts magical fields. A mage holding iron cannot cast."
    → rule_type: magic_limitation

Prerequisites:
  - "Only those with the Singing Voice (genetic trait, 1 in 1000) can use magic."
    → rule_type: ability_prerequisite
```

### Step 3: Extrapolate Consequences

Ask "What happens when...?" for every rule:
- If magic consumes memories, what does a powerful mage look like? (Someone who has
  sacrificed their identity for power. Terrifying AND sympathetic.)
- If iron blocks magic, what does warfare look like? (Iron-clad anti-mage soldiers.
  Mages avoid cities with iron infrastructure.)
- If power comes from bonds, what happens to a hermit? (Powerless. What happens to
  someone who forms bonds purely for power? Do fake bonds work? That's a plot.)

These extrapolations create `world_rule` entries AND plot hooks AND character motivations.

When updating an existing `world_rule` later, use the dedicated
`update_world_rule { world_rule_id, changes }` tool (not `update_entity`):
- use `description` for the main rule text
- use `relevance_tags` for context targeting
- use `scan_pattern` to give the validator something to scan prose for

### Step 4: Connect to LitRPG / Cultivation (if applicable)

If the world has a game-like system on top of the magic:
- Create the `system_overlay` for the mechanical layer.
- Create `world_rule` entries for each progression constraint (level caps,
  cooldowns, prerequisites). Set `scan_pattern` so the consistency validator
  can flag scenes that violate the constraint.
- For per-arc progression budgets, use `create_pacing_config` and
  `set_arc_pacing_constraints` (covered in plot-architect).
- Record per-character progression state via `register_canonical_fact` with
  typed payloads (predicate `progression.level`, value_kind `number`,
  value_number 12, value_unit "tier").

---

## Building Religions

Religions provide moral frameworks, social control, and plot conflict.

Call `create_religion` with the typed input:
- **project_id** (required)
- **name** (required)
- **deity_or_principle** (required): What they worship or follow.
- **summary** (required): Beliefs, practices, social structure.
- **tags** (`Vec<String>`, defaults empty)

Then create:
- `world_rule` entries for religious laws and taboos (with `scan_pattern` so
  the validator can flag scenes that violate them).
- Canonical facts (`register_canonical_fact` with predicates like
  `religion.observances`, `religion.taboos`) for character-specific religious
  positions.

There is no shipped tool for `worships_at` or other religion-character
edges; encode those relationships as canonical facts.

---

## Building Economies

Economies create stakes around resources, trade, and power.

Call `create_economy` with the typed input:
- **project_id** (required)
- **name** (required)
- **realm** (`Option<String>`)
- **summary** (required)
- **scarce_resources** (`Vec<String>`)
- **trade_goods** (`Vec<String>`)
- **currency** (`Option<String>`)
- **notes** (`Vec<String>`)

Then:
- Define what's scarce (scarcity drives conflict). Encode trade routes as
  `world_rule` entries with `relevance_tags: ["trade", "economy"]` since
  there is no shipped trades_with edge for economies.
- Create `world_rule` entries for economic constraints
  ("Only the guild can mine spiritstone" → `resource_scarcity`,
  scan_pattern: "spiritstone").

---

## Glossary

For every fictional term, call `create_term` with the typed input:
- **project_id** (required)
- **term_text** (required): The word itself
- **definition** (required): What it means in this world
- **pronunciation** (`Option<String>`): How to say it (important for audiobook
  consistency)
- **usage_context** (`Option<String>`): When would a character use this word?
- **origin** (`Option<String>`): Where did this word come from in-world?

Use `batch_create_terms` to seed an entire glossary in one call rather than
many.

Terms appear in the context package when relevant, helping the scene-writer
use them correctly and consistently.

---

## The Worldbuilder's Checklist

Before declaring a world "ready for writing," verify:

| Check | Tool | Why It Matters |
|-------|------|----------------|
| Every power has a cost | `check_consistency` | No deus ex machina |
| Every rule will be demonstrated | consistency review + scene evidence | No empty rules |
| Travel times are defined | `world_rule` (time_constraint) | Characters can't teleport |
| Factions have conflicting goals | Manual check | No conflict = no plot |
| Locations have sensory details | Location summaries | Grounds the reader |
| The magic system position (hard/soft) is decided | `system_overlay.visibility` | Determines how scenes can use it |
| Glossary covers all invented terms | `create_term` for each | Consistency |
| The economy creates scarcity | `world_rule` (resource_scarcity) | Stakes |

---

## Skill Chains

- **→ character-creator**: After building the world, create characters who are SHAPED by it.
  A character from a memory-magic world will have a very different relationship with their
  past than one from a mundane world.
- **→ plot-architect**: World rules create plot constraints. A magic system with costs
  creates natural try-fail cycles (the cost escalates each attempt).
- **→ scene-writer**: The scene-writer reads world_rules from context and never violates them.
  If a rule hasn't been demonstrated yet, the scene-writer will prioritize showing it.
- **→ continuity-editor**: Checks world rules for violations across all written scenes.

---

## References

The four embedded craft references most relevant to worldbuilders are
exposed as `bible://references/<name>` resources:

- `bible://references/anti-slop` — Avoiding generic AI prose patterns when
  describing locations, magic, and world atmosphere.
- `bible://references/voice-differentiation` — Useful when writing dialogue
  that reflects faction, cultural, or class differences.
- `bible://references/swain-scene-sequel` and `bible://references/mru-guide`
  — Craft references the scene-writer skill draws from when realizing
  worldbuilding on the page.

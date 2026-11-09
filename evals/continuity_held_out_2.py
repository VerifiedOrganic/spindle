"""Independent held-out-2 continuity corpus.

Authored from the seven public contradiction categories and the MCP tool
interface only. This file deliberately does not import or inspect detector
implementations, the baseline corpus, or the #72 held-out corpus.

Two new 15-chapter manuscripts (a wry close-third coastal mystery and a lyrical
first-person orchard science fantasy) plant four contradictions per category
per manuscript: 56 total. A clean twin of each manuscript measures false
positives. Each plant records a distinctive phrase so structural detections can
be matched without relying on a private detector implementation.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import Any, Iterable

from continuity_recall import McpClient

EXPECTED_CHECKS = {
    "character_attribute": {"character_consistency", "character_attribute"},
    "dead_character_acting": {"dead_character_acting"},
    "timeline_chronology_rewind": {"chronology", "timeline_continuity"},
    "knowledge_leak_before_learned": {"knowledge_timing"},
    "world_rule_cost_violation": {"world_rule_cost_violation"},
    "quantity_affordability": {"affordability"},
    "secret_leak": {"secret_leak"},
}

BROAD_CHECKS = {
    "canonical_fact_prose_drift",
    "world_rule_semantic_drift",
    "character_consistency",
}
ALL_CHECK_TYPES = {
    name for names in EXPECTED_CHECKS.values() for name in names
} | BROAD_CHECKS


def c(kind: str, keyword: str, text: str) -> dict[str, Any]:
    return {"type": kind, "keyword": keyword, "text": text}


SALT_CHAPTERS: list[dict[str, Any]] = [
    {
        "base": "Mara Voss kept the harbor's maps in a cedar chest and its debts in her head. The tide gate at Salt Quay had not opened on time in eleven years, and she liked it that way: predictable water made honest charts. Her aunt Cress ran the chandlery below the workroom, and Soren Pike, seventeen and certain of everything, carried her ink.",
        "plants": [],
    },
    {
        "base": "A squall came in low over the mole, and Mara spent the morning redrawing the shoal line. Ilya Brant, the harbor master, held the lantern while she worked. Cress brought up tea and complained about the price of wicks.",
        "plants": [
            c("character_attribute", "right hand", "Mara wrote with her right hand, though the ink blotted at every turn."),
            c("secret_leak", "forged the tide charts", "Everyone on the quay already knew that Lord Vale had forged the tide charts."),
        ],
    },
    {
        "base": "The gate jammed at noon and the ferry missed the tide. Mara found a brass token wedged in the lock and pocketed it. Soren asked why the harbor office kept two sets of charts.",
        "plants": [
            c("knowledge_leak_before_learned", "charts were forged", "Soren said the charts were forged, and Mara did not correct him."),
            c("world_rule_cost_violation", "without a brass token", "The tide gate swung open without a brass token."),
        ],
    },
    {
        "base": "Mara walked the market with her compass and a list of soundings. The chandler's boy sold her a new needle, and she checked the bearing twice before she paid.",
        "plants": [
            c("character_attribute", "five feet nine", "Mara stood five feet nine in her boots and had to duck under the sailmaker's awning."),
            c("quantity_affordability", "twelve marks", "She paid twelve marks for the compass, though her purse held three."),
        ],
    },
    {
        "base": "The storm turned the harbor the color of old pewter. Cress took a fever and would not leave her bed. Mara sat with her until the lamp burned out, then went down to the gate alone.",
        "plants": [
            c("timeline_chronology_rewind", "the previous week", "The previous week had already ended; still, the next morning came before the storm."),
            c("knowledge_leak_before_learned", "charts were false", "Mara told the harbormaster the charts were false, though no one had shown her the forgery yet."),
        ],
    },
    {
        "base": "They buried Cress on the hill above the chandlery. The harbor closed for the afternoon. Mara packed her aunt's account books into a crate and found a note in the margins about Vale's payments.",
        "plants": [
            c("dead_character_acting", "Aunt Cress met her", "Aunt Cress met her at the door and took her coat."),
            c("world_rule_cost_violation", "without giving up a memory", "Mara bound the salt-ink map without giving up a memory."),
        ],
    },
    {
        "base": "Ilya showed her the gate's ledger. Someone had been paying the lock keeper in brass tokens that were not in the harbor accounts. Mara copied the entries by lamplight.",
        "plants": [
            c("character_attribute", "left hand", "Ilya raised his left hand to stop her, though he had lost the fingers on that side years ago."),
            c("secret_leak", "Vale's charts were false", "A dockworker shouted that Vale's charts were false."),
        ],
    },
    {
        "base": "Mara took the ferry across the mouth and walked the outer mole. The gate stood open behind her, and the water ran clean through the pilings. She began a new chart of the channel.",
        "plants": [
            c("dead_character_acting", "Cress laughed", "Cress laughed from the chandlery door and told her to mind the ebb."),
            c("quantity_affordability", "forty marks", "Mara bought the boat for forty marks and counted the coins twice."),
        ],
    },
    {
        "base": "Soren brought word from the north quay: a pilot had seen lights on the forbidden side of the mole. Mara checked the clock and found the tide still falling.",
        "plants": [
            c("timeline_chronology_rewind", "before the burial", "The pilot's sighting came before the burial, though the burial had happened two days earlier."),
            c("knowledge_leak_before_learned", "the forgery", "Soren repeated the forgery to the gate keeper as if it were common knowledge."),
        ],
    },
    {
        "base": "Vale's clerk came for the harbor accounts. Mara gave him the clean copy and kept the ledger with the brass-token entries. The gate opened at dusk, on time, with a token she had seen the lock keeper feed into the slot.",
        "plants": [
            c("world_rule_cost_violation", "without a token", "The gate opened without a token and the lock keeper did not notice."),
            c("character_attribute", "scar had never existed", "The scar over Mara's brow had never existed, and the mirror showed smooth skin."),
        ],
    },
    {
        "base": "The broadsheet office smelled of wet paper. Mara read the harbor notices and found Vale's name beside a contract for new charts. She folded the page and put it in her coat.",
        "plants": [
            c("dead_character_acting", "Cress poured the tea", "Cress poured the tea and asked what the broadsheet said."),
            c("secret_leak", "forged charts", "The broadsheet printed the forged charts in full, with Vale's seal."),
        ],
    },
    {
        "base": "The gate failed at the evening tide. Mara and Ilya worked the winch until midnight, and the ferry came in under a lantern. She paid the pilot in full and wrote the sum in the ledger.",
        "plants": [
            c("quantity_affordability", "a hundred marks", "Mara paid a hundred marks for the pilot's silence."),
            c("timeline_chronology_rewind", "three days before", "The failure had happened three days before, but the next morning arrived first."),
        ],
    },
    {
        "base": "Soren found the lock keeper's copy of the charts behind a loose brick. The bearings were wrong by a hand's width, enough to put a boat on the shoal. Mara compared them with her own soundings.",
        "plants": [
            c("dead_character_acting", "Cress said", "Cress said the bearings had always been wrong, and tapped the table."),
            c("knowledge_leak_before_learned", "Vale forged", "Mara said Vale forged the bearings, though the lock keeper's copy was the first proof she had seen."),
        ],
    },
    {
        "base": "At the hearing, Mara spread the two charts side by side. Vale's seal was on the false one; the lock keeper's brass tokens were in her pocket. The harbor board asked for a full account.",
        "plants": [
            c("secret_leak", "Vale forged the charts", "The clerk announced that Vale forged the charts before the board had heard the evidence."),
            c("world_rule_cost_violation", "without a memory", "Mara sealed the hearing map with salt ink without a memory and felt nothing."),
        ],
    },
    {
        "base": "The board ruled for the harbor and against Vale's contract. Mara walked home along the mole as the gate opened on the evening tide. Soren carried the cedar chest, and the water ran clean through the pilings.",
        "plants": [
            c("quantity_affordability", "twenty marks", "Mara lost twenty marks on the ferry fare and laughed about it."),
            c("timeline_chronology_rewind", "the week after", "The week after the hearing came before the hearing itself."),
        ],
    },
]

ORCHARD_CHAPTERS: list[dict[str, Any]] = [
    {
        "base": "I keep the orchard's records in a ledger my mother bound with copper wire. The glass trees ring when the wind crosses them, a sound like teaspoons on a sill. Tam is what the pickers call me; Tamsin Okoro is what the survey office writes on the forms. Cassian Reed runs the signal shed at the south wall.",
        "plants": [],
    },
    {
        "base": "The first copper shard goes into the old tree at dawn. It rings once, a clean note that travels down the row. Rook, our blind arborist, says the glass remembers every hand that has touched it.",
        "plants": [
            c("character_attribute", "left-handed", "I am left-handed, so I hold the shard in my left hand and tap the trunk with my right."),
            c("secret_leak", "Cassian had cut the roots", "Everyone at the south wall already knew that Cassian had cut the roots."),
        ],
    },
    {
        "base": "The survey office sends a new map of the orchard every spring. This year the lines run through the old well, which has been dry since before I was born. I walk the rows and mark the trees that still ring.",
        "plants": [
            c("knowledge_leak_before_learned", "Cassian cut the roots", "I tell the survey clerk that Cassian cut the roots, though I have not yet seen the shed."),
            c("world_rule_cost_violation", "without a copper shard", "The tree rings without a copper shard and the note hangs in the air."),
        ],
    },
    {
        "base": "The pickers bring in the first baskets at noon. Nia, my sister, counts them twice and writes the totals on the shed door. We have two copper coins between us and a week before the carrier comes.",
        "plants": [
            c("character_attribute", "six feet", "I stand six feet in my boots and still cannot reach the highest branch."),
            c("quantity_affordability", "five copper", "I pay five copper for the carrier's stamp and put the receipt in my pocket."),
        ],
    },
    {
        "base": "A late frost cracks the youngest tree. Nia and I wrap the trunk in sacking and stay until the moon is high. In the morning the tree is still, and the orchard is quiet in a way that means the glass has gone cold.",
        "plants": [
            c("timeline_chronology_rewind", "the previous harvest", "The previous harvest had not yet happened; still, the next frost came first."),
            c("knowledge_leak_before_learned", "sabotage", "I tell Rook the sabotage began in the shed, though Cassian has not shown me the cut roots."),
        ],
    },
    {
        "base": "Nia dies before the carrier comes. The fever takes her in three days, and we bury her under the oldest tree. I write the date in the ledger and press the page flat with my palm.",
        "plants": [
            c("dead_character_acting", "Nia counts", "Nia counts the baskets at the shed door and calls me Tam."),
            c("world_rule_cost_violation", "without a living relay", "The signal crosses the salt marsh without a living relay and arrives clear."),
        ],
    },
    {
        "base": "Rook shows me how to read the ring pattern by touch. A healthy tree gives a low note; a dying one gives a tick like a clock. We walk the south row together and he names each tree from memory.",
        "plants": [
            c("character_attribute", "Rook saw", "Rook saw the crack in the glass before I did."),
            c("secret_leak", "Cassian cut the roots", "A picker says Cassian cut the roots and the whole row goes quiet."),
        ],
    },
    {
        "base": "The carrier brings a letter from the survey office and a box of copper shards. I sign for both and carry the box to the shed. Cassian is not there; his coat hangs on the hook and the signal log is open.",
        "plants": [
            c("dead_character_acting", "Nia says", "Nia says the carrier is late and puts the kettle on."),
            c("quantity_affordability", "twenty copper", "I buy the box of shards for twenty copper and count them twice."),
        ],
    },
    {
        "base": "The signal log lists every transmission since the thaw. Most are weather reports; one is a line of numbers I do not recognize. I copy it into my ledger and leave the shed as I found it.",
        "plants": [
            c("timeline_chronology_rewind", "before the thaw", "The transmission came before the thaw, though the thaw had already passed."),
            c("knowledge_leak_before_learned", "Cassian's sabotage", "I write Cassian's sabotage in the log as if I had known it for years."),
        ],
    },
    {
        "base": "The glass trees ring all afternoon without a wind. Rook says that means the roots are touching something they should not. We dig along the south wall and find a copper wire running under the fence.",
        "plants": [
            c("world_rule_cost_violation", "without a relay", "The signal reaches the survey office without a relay and the wire stays cold."),
            c("character_attribute", "burn scar", "The burn scar on my right forearm is gone, and the skin is smooth."),
        ],
    },
    {
        "base": "Cassian comes back at dusk with mud on his boots. He says the wire was an old survey line and nothing more. I ask him to open the shed, and he does not move.",
        "plants": [
            c("dead_character_acting", "Nia opens", "Nia opens the shed door and asks Cassian to step inside."),
            c("secret_leak", "Cassian cut the roots", "The pickers' notice board says Cassian cut the roots and sold the copper."),
        ],
    },
    {
        "base": "The survey office sends an inspector. We walk the rows together and he writes down the dead trees. At the south wall he stops and looks at the wire, and I see that he has been here before.",
        "plants": [
            c("quantity_affordability", "fifty copper", "I pay the inspector fifty copper to leave the wire out of his report."),
            c("timeline_chronology_rewind", "three days earlier", "The inspection had happened three days earlier, but the next morning came first."),
        ],
    },
    {
        "base": "Rook finds the cut roots under the oldest tree. The glass has gone milky where the copper touched it. We follow the wire back to the shed and find Cassian's logbook under a loose floorboard.",
        "plants": [
            c("dead_character_acting", "Nia reads", "Nia reads the logbook over my shoulder and taps the page."),
            c("knowledge_leak_before_learned", "Cassian cut", "I tell Rook that Cassian cut the roots before we open the floorboard."),
        ],
    },
    {
        "base": "The logbook names every tree Cassian sold and every buyer at the survey office. I copy the list and take it to the carrier. The orchard is quiet; the glass trees have stopped ringing.",
        "plants": [
            c("secret_leak", "Cassian cut the roots", "The carrier says Cassian cut the roots before I show her the logbook."),
            c("world_rule_cost_violation", "without a living relay", "The signal crosses the marsh without a living relay and the office answers at once."),
        ],
    },
    {
        "base": "The survey office voids Cassian's contract and sends a new relay for the south wall. Rook and I plant copper shards along the row. By evening the trees are ringing again, one note after another, and the orchard sounds like rain on a tin roof.",
        "plants": [
            c("quantity_affordability", "ten copper", "I lose ten copper on the carrier fare and say nothing."),
            c("timeline_chronology_rewind", "the week after", "The week after the replanting came before the replanting itself."),
        ],
    },
]


MANUSCRIPTS = [
    {
        "name": "salt_cartographer",
        "genre": "coastal literary mystery",
        "voice": "wry close third, past tense",
        "promise": "A harbor cartographer proves a noble forged the tide charts.",
        "calendar": {"days_per_week": 7, "hours_per_day": 24, "days_per_year": 360, "epoch_label": "Harbor Year"},
        "protagonist": "Mara Voss",
        "characters": [
            ("Mara Voss", "A cartographer who keeps the harbor's charts.", "protagonist", "Six feet two, left-handed, a pale scar over the left brow.", ["Moss"]),
            ("Ilya Brant", "The harbor master.", "supporting", "Missing two fingers on the left hand; right-handed.", []),
            ("Soren Pike", "An apprentice chart carrier.", "supporting", "Seventeen, freckled, always cold.", []),
            ("Cress Voss", "Mara's aunt and the chandlery keeper.", "supporting", "Grey braid, ink-stained cuffs.", ["Aunt Cress"]),
            ("Lord Vale", "The harbor's largest chart contractor.", "antagonist", "Immaculate gloves; a signet ring.", []),
        ],
        "death": {"chapter": 5, "character": "Cress Voss", "fact": "Cress Voss died of fever on the hill above the chandlery."},
        "rule": {
            "name": "Tide gate cost",
            "type": "cost",
            "description": "The tide gate opens only when a brass token is fed to the lock; salt-ink bindings cost the mapmaker one memory.",
            "scan": "tide gate|salt-ink",
        },
        "quantity": {"measure": "coin", "amount": 3, "unit": "marks", "band": "comfortable"},
        "secret": {"holder": "Lord Vale", "key": "Vale:tide_charts", "fact": "Lord Vale forged the harbor tide charts."},
        "chapters": SALT_CHAPTERS,
        "learned": {"chapter": 14, "character": "Mara Voss", "fact": "Lord Vale forged the harbor tide charts."},
    },
    {
        "name": "glass_orchard",
        "genre": "pastoral science fantasy",
        "voice": "lyrical first person, present tense",
        "promise": "An orchard keeper uncovers who is killing the signal-bearing glass trees.",
        "calendar": {"days_per_week": 8, "hours_per_day": 20, "days_per_year": 320, "epoch_label": "Orchard Spring"},
        "protagonist": "Tamsin Okoro",
        "characters": [
            ("Tamsin Okoro", "An orchard keeper and record-keeper.", "protagonist", "Five feet four, right-handed, a burn scar on the right forearm.", ["Tam"]),
            ("Elder Rook", "A blind arborist.", "supporting", "Blind since the frost; reads trees by touch.", []),
            ("Cassian Reed", "The signal-shed engineer.", "antagonist", "Mud on his boots; a copper-wire bracelet.", []),
            ("Nia Okoro", "Tamsin's sister.", "supporting", "Quick hands, a chipped front tooth.", []),
            ("The orcharders", "The picking crew.", "supporting", "A rotating crew of neighbours.", []),
        ],
        "death": {"chapter": 5, "character": "Nia Okoro", "fact": "Nia Okoro died of fever before the carrier came."},
        "rule": {
            "name": "Glass signal cost",
            "type": "cost",
            "description": "A glass tree rings only when fed a copper shard; the signal cannot cross the salt marsh without a living relay.",
            "scan": "copper shard|living relay",
        },
        "quantity": {"measure": "copper", "amount": 2, "unit": "copper", "band": "destitute"},
        "secret": {"holder": "Cassian Reed", "key": "Cassian:root_cut", "fact": "Cassian Reed cut the orchard roots and sold the copper."},
        "chapters": ORCHARD_CHAPTERS,
        "learned": {"chapter": 14, "character": "Tamsin Okoro", "fact": "Cassian Reed cut the orchard roots and sold the copper."},
    },
]


def _setup(client: McpClient, spec: dict[str, Any]) -> dict[str, Any]:
    project = client.tool(
        "create_project",
        {
            "name": f"Held-out-2: {spec['name']}",
            "project_type": "novel",
            "genre": spec["genre"],
            "reader_contract": {
                "promise": spec["promise"],
                "style_notes": [f"Voice: {spec['voice']}"],
                "boundaries": [],
            },
        },
    )
    project_id = project["project_id"]
    client.tool(
        "set_project_calendar",
        {"project_id": project_id, "calendar": spec["calendar"]},
    )
    characters: dict[str, str] = {}
    for name, summary, role, appearance, aliases in spec["characters"]:
        out = client.tool(
            "create_character",
            {
                "project_id": project_id,
                "name": name,
                "summary": summary,
                "role": role,
                "appearance": appearance,
                "aliases": aliases,
                "voice_profile": {
                    "tone": "wry" if spec["name"] == "salt_cartographer" else "lyrical",
                    "example_lines": [f"{name} speaks with a measured cadence."],
                },
                "emotional_profile": {
                    "base_emotions": {"resolve": 0.7, "grief": 0.3},
                    "triggers": ["loss"],
                    "defense_mechanisms": ["humour" if spec["name"] == "salt_cartographer" else "silence"],
                },
            },
        )
        characters[name] = out["character_id"]
    client.tool(
        "create_location",
        {
            "project_id": project_id,
            "name": "The harbor" if spec["name"] == "salt_cartographer" else "The orchard",
            "kind": "setting",
            "summary": "The manuscript's central working place.",
        },
    )
    client.tool(
        "create_world_rule",
        {
            "project_id": project_id,
            "rule_name": spec["rule"]["name"],
            "rule_type": spec["rule"]["type"],
            "description": spec["rule"]["description"],
            "scan_pattern": spec["rule"]["scan"],
            "established_in": {"book_number": 1, "chapter_number": 1},
        },
    )
    client.tool(
        "set_project_quantity_scheme",
        {
            "project_id": project_id,
            "scheme": {
                "measure": spec["quantity"]["measure"],
                "bands": [
                    {"name": "destitute", "lower_bound": 0},
                    {"name": "comfortable", "lower_bound": 3},
                    {"name": "wealthy", "lower_bound": 20},
                ],
                "denominations": [
                    {"name": spec["quantity"]["unit"], "per_base": 1}
                ],
                "max_band_jump": 1,
            },
        },
    )
    secret = client.tool(
        "register_canonical_fact",
        {
            "project_id": project_id,
            "fact_type": "secret",
            "key": spec["secret"]["key"],
            "predicate": "root_cause",
            "value_text": spec["secret"]["fact"],
            "value_kind": "text",
            "scope": "invariant",
            "book_number": 1,
            "chapter_number": 1,
            "secrecy": {
                "holder_ids": [characters[spec["secret"]["holder"]]],
                "concealment_note": "Known only to the holder until the protagonist proves it.",
            },
        },
    )
    attribute_facts = {
        "Mara Voss": [("handedness", "left"), ("height", "six feet two"), ("scar", "left brow")],
        "Ilya Brant": [("handedness", "right"), ("fingers", "missing two on the left hand")],
        "Tamsin Okoro": [("handedness", "right"), ("height", "five feet four"), ("scar", "burn on the right forearm")],
        "Elder Rook": [("sight", "blind")],
    }
    for name, facts in attribute_facts.items():
        if name not in characters:
            continue
        for predicate, value in facts:
            client.tool(
                "register_canonical_fact",
                {
                    "project_id": project_id,
                    "fact_type": "attribute",
                    "key": f"{name}:{predicate}",
                    "predicate": predicate,
                    "value_text": value,
                    "value_kind": "text",
                    "scope": "invariant",
                    "subject_table": "character",
                    "subject_id": characters[name],
                    "book_number": 1,
                    "chapter_number": 1,
                },
            )
    return {
        "project_id": project_id,
        "characters": characters,
        "secret_fact_id": secret["canonical_fact_id"],
    }


def _build_manuscript(
    client: McpClient,
    spec: dict[str, Any],
    planted: bool,
) -> dict[str, Any]:
    setup = _setup(client, spec)
    project_id = setup["project_id"]
    characters = setup["characters"]
    scene_ids: dict[int, str] = {}
    plants: list[dict[str, Any]] = []
    for chapter in range(1, 16):
        chapter_spec = spec["chapters"][chapter - 1]
        text = chapter_spec["base"]
        chapter_plants = chapter_spec["plants"] if planted else []
        if chapter_plants:
            text = text + " " + " ".join(plant["text"] for plant in chapter_plants)
        chapter_out = client.tool(
            "create_chapter",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter,
                "title": f"Chapter {chapter}",
            },
        )
        scene = client.tool(
            "save_scene_draft",
            {
                "project_id": project_id,
                "chapter_id": chapter_out["chapter_id"],
                "scene_order": 1,
                "full_text": text,
                "summary": text[:180],
                "content_rating": "General",
                "tone": "measured" if spec["name"] == "salt_cartographer" else "luminous",
            },
        )
        scene_id = scene["scene_id"]
        scene_ids[chapter] = scene_id
        day_index = chapter
        if planted and chapter in {5, 9, 12, 15}:
            day_index = {5: 3, 9: 6, 12: 5, 15: 9}[chapter]
        client.tool(
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": scene_id,
                "clock": {"day_index": day_index},
                "temporal_mode": "linear",
            },
        )
        client.tool(
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter,
                "summary": chapter_spec["base"][:220],
            },
        )
        for plant in chapter_plants:
            plant["scene_id"] = scene_id
            plants.append(plant)
        if chapter == 1:
            client.tool(
                "commit_quantity_state",
                {
                    "project_id": project_id,
                    "subject_table": "character",
                    "subject_id": characters[spec["protagonist"]],
                    "measure": spec["quantity"]["measure"],
                    "book_number": 1,
                    "chapter_number": 1,
                    "scene_order": 1,
                    "state": {
                        "band": spec["quantity"]["band"],
                        "amount": spec["quantity"]["amount"],
                        "unit": spec["quantity"]["unit"],
                    },
                },
            )
        if chapter == spec["death"]["chapter"]:
            death_id = characters[spec["death"]["character"]]
            client.tool(
                "commit_character_state",
                {
                    "character_id": death_id,
                    "scene_id": scene_id,
                    "changes": {
                        "status": ["dead"],
                        "notes": [spec["death"]["fact"]],
                        "source_summary": spec["death"]["fact"],
                    },
                },
            )
            client.tool(
                "register_canonical_fact",
                {
                    "project_id": project_id,
                    "fact_type": "death",
                    "key": f"{spec['death']['character']}:life_status",
                    "predicate": "life_status",
                    "value_text": "dead",
                    "value_kind": "text",
                    "scope": "invariant",
                    "book_number": 1,
                    "chapter_number": chapter,
                    "scene_id": scene_id,
                    "context": spec["death"]["fact"],
                },
            )
    # Record the late reveal so prior knowledge leaks are genuinely early.
    client.tool(
        "record_knowledge",
        {
            "project_id": project_id,
            "character_id": characters[spec["learned"]["character"]],
            "fact": spec["learned"]["fact"],
            "learned_at": {"book_number": 1, "chapter_number": spec["learned"]["chapter"]},
            "source_summary": "Proven from the recovered records.",
            "reader_visible": True,
            "secret_of_fact_id": setup["secret_fact_id"],
        },
    )
    report = client.tool(
        "check_consistency",
        {"project_id": project_id, "scope": {"scope_type": "full"}},
    )
    return {
        "project_id": project_id,
        "issues": report.get("issues", []),
        "plants": plants,
        "scene_ids": scene_ids,
    }


def _score(issues: Iterable[dict[str, Any]], plants: Iterable[dict[str, Any]]) -> dict[str, Any]:
    issue_list = list(issues)
    plant_list = list(plants)
    per_type: dict[str, dict[str, int]] = {
        kind: {"planted": 0, "localized": 0, "type_present": 0}
        for kind in EXPECTED_CHECKS
    }
    for plant in plant_list:
        kind = plant["type"]
        per_type[kind]["planted"] += 1
        expected = EXPECTED_CHECKS[kind]
        keyword = plant["keyword"].lower()
        scene_id = plant.get("scene_id")
        localized = any(
            issue.get("severity") in {"error", "warning"}
            and issue.get("check_type") in (expected | BROAD_CHECKS)
            and (
                keyword in str(issue.get("message", "")).lower()
                or (scene_id and scene_id in issue.get("entity_ids", []))
            )
            for issue in issue_list
        )
        type_present = any(
            issue.get("severity") in {"error", "warning"}
            and issue.get("check_type") in expected
            for issue in issue_list
        )
        per_type[kind]["localized"] += int(localized)
        per_type[kind]["type_present"] += int(type_present)
    relevant_issues = [
        issue
        for issue in issue_list
        if issue.get("check_type") in ALL_CHECK_TYPES
        and issue.get("severity") in {"error", "warning"}
    ]
    info_issues = [
        issue
        for issue in issue_list
        if issue.get("check_type") in ALL_CHECK_TYPES
        and issue.get("severity") not in {"error", "warning"}
    ]
    return {
        "per_type": per_type,
        "planted": len(plant_list),
        "localized_detected": sum(values["localized"] for values in per_type.values()),
        "type_detected": sum(values["type_present"] for values in per_type.values()),
        "control_false_positive_count": len(relevant_issues) if not plant_list else 0,
        "control_info_count": len(info_issues) if not plant_list else 0,
        "issues": [
            {
                "check_type": issue.get("check_type"),
                "severity": issue.get("severity"),
                "message": issue.get("message"),
                "entity_ids": issue.get("entity_ids", []),
            }
            for issue in relevant_issues
        ],
    }


def _aggregate_scores(
    manuscript_scores: list[dict[str, Any]],
    control_scores: list[dict[str, Any]],
    manuscript_meta: list[dict[str, Any]],
) -> dict[str, Any]:
    per_type: dict[str, dict[str, int]] = {
        kind: {"planted": 0, "localized": 0, "type_present": 0}
        for kind in EXPECTED_CHECKS
    }
    for score in manuscript_scores:
        for kind, values in score["per_type"].items():
            per_type[kind]["planted"] += values["planted"]
            per_type[kind]["localized"] += values["localized"]
            per_type[kind]["type_present"] += values["type_present"]
    total = sum(values["planted"] for values in per_type.values())
    localized = sum(values["localized"] for values in per_type.values())
    type_level = sum(values["type_present"] for values in per_type.values())
    control_issues = [issue for score in control_scores for issue in score["issues"]]
    control_info = sum(score["control_info_count"] for score in control_scores)
    return {
        "corpus": "held-out-2",
        "description": "Two independently authored 15-chapter manuscripts; 4 planted contradictions per public category per manuscript; clean twin controls.",
        "manuscripts": manuscript_meta,
        "per_type": {
            kind: {
                **values,
                "localized_recall": round(values["localized"] / values["planted"], 4)
                if values["planted"]
                else 0.0,
                "type_level_recall": round(values["type_present"] / values["planted"], 4)
                if values["planted"]
                else 0.0,
            }
            for kind, values in per_type.items()
        },
        "planted_total": total,
        "localized_detected": localized,
        "type_level_detected": type_level,
        "overall_localized_recall": round(localized / total, 4) if total else 0.0,
        "overall_type_level_recall": round(type_level / total, 4) if total else 0.0,
        "control_false_positive_count": len(control_issues),
        "control_info_count": control_info,
        "control_false_positives": [
            {
                "check_type": issue.get("check_type"),
                "severity": issue.get("severity"),
                "message": issue.get("message"),
            }
            for issue in control_issues
        ],
    }


def run_held_out_2_benchmark(
    *,
    binary: Path,
    deep_check: bool = False,
    agent_config: Path | None = None,
) -> dict[str, Any]:
    if deep_check:
        raise RuntimeError("held-out-2 currently runs the deterministic check surface only")
    manuscript_scores: list[dict[str, Any]] = []
    control_scores: list[dict[str, Any]] = []
    manuscript_meta: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="spindle-held-out-2-") as temp:
        root = Path(temp)
        for spec in MANUSCRIPTS:
            for planted in (True, False):
                data_dir = root / f"{spec['name']}-{'planted' if planted else 'control'}"
                data_dir.mkdir()
                client = McpClient(binary, data_dir, agent_config=agent_config)
                try:
                    result = _build_manuscript(client, spec, planted=planted)
                finally:
                    client.close()
                score = _score(result["issues"], result["plants"])
                if planted:
                    manuscript_scores.append(score)
                    manuscript_meta.append(
                        {
                            "name": spec["name"],
                            "genre": spec["genre"],
                            "voice": spec["voice"],
                            "planted": score["planted"],
                            "localized_detected": score["localized_detected"],
                            "type_detected": score["type_detected"],
                            "per_type": score["per_type"],
                            "issues": score["issues"],
                        }
                    )
                else:
                    control_scores.append(score)
    return _aggregate_scores(manuscript_scores, control_scores, manuscript_meta)


def held_out_2_self_test() -> None:
    """Offline corpus invariants; never starts the MCP server."""
    assert len(MANUSCRIPTS) >= 2
    total_per_type: dict[str, int] = {kind: 0 for kind in EXPECTED_CHECKS}
    for spec in MANUSCRIPTS:
        assert len(spec["chapters"]) >= 15, spec["name"]
        assert all(chapter["plants"] for chapter in spec["chapters"][1:]), spec["name"]
        counts: dict[str, int] = {kind: 0 for kind in EXPECTED_CHECKS}
        for chapter in spec["chapters"]:
            for plant in chapter["plants"]:
                assert plant["keyword"]
                assert plant["text"]
                assert plant["type"] in EXPECTED_CHECKS
                assert plant["text"] not in chapter["base"]
                counts[plant["type"]] += 1
                total_per_type[plant["type"]] += 1
        assert counts == {kind: 4 for kind in EXPECTED_CHECKS}, (spec["name"], counts)
    assert total_per_type == {kind: 8 for kind in EXPECTED_CHECKS}, total_per_type

    # Info-only rule mentions are not contradiction credit.
    info = _score(
        [
            {
                "check_type": "world_rule_cost_violation",
                "severity": "info",
                "message": "scene references world rule 'x' (no violation context detected)",
                "entity_ids": [],
            }
        ],
        [{"type": "world_rule_cost_violation", "keyword": "without a token", "scene_id": "scene:1"}],
    )
    assert info["localized_detected"] == 0
    assert info["type_detected"] == 0

    # A broad check with evidence can localize, but still cannot stand in for
    # the category-specific type-present lane.
    broad_positive = _score(
        [
            {
                "check_type": "canonical_fact_prose_drift",
                "severity": "warning",
                "message": "Cress appears after the recorded death",
                "entity_ids": [],
            }
        ],
        [{"type": "dead_character_acting", "keyword": "Cress", "scene_id": "scene:1"}],
    )
    assert broad_positive["per_type"]["dead_character_acting"]["localized"] == 1
    assert broad_positive["per_type"]["dead_character_acting"]["type_present"] == 0

    # A broad attribute warning cannot credit a death, knowledge, or cost plant.
    broad = _score(
        [
            {
                "check_type": "character_consistency",
                "severity": "warning",
                "message": "character texture is thin",
                "entity_ids": [],
            }
        ],
        [
            {"type": "dead_character_acting", "keyword": "Nia", "scene_id": "scene:1"},
            {"type": "knowledge_leak_before_learned", "keyword": "sabotage", "scene_id": "scene:1"},
            {"type": "world_rule_cost_violation", "keyword": "relay", "scene_id": "scene:1"},
        ],
    )
    for kind in (
        "dead_character_acting",
        "knowledge_leak_before_learned",
        "world_rule_cost_violation",
    ):
        assert broad["per_type"][kind]["localized"] == 0
        assert broad["per_type"][kind]["type_present"] == 0

    # Aggregation keeps manuscript identity: one finding in one manuscript
    # cannot credit another manuscript's plants.
    empty = {kind: {"planted": 4, "localized": 0, "type_present": 0} for kind in EXPECTED_CHECKS}
    found = {kind: dict(values) for kind, values in empty.items()}
    found["dead_character_acting"]["type_present"] = 1
    aggregate = _aggregate_scores(
        [
            {"per_type": found, "issues": []},
            {"per_type": empty, "issues": []},
        ],
        [],
        [],
    )
    assert aggregate["per_type"]["dead_character_acting"]["planted"] == 8
    assert aggregate["per_type"]["dead_character_acting"]["type_present"] == 1

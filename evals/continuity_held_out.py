#!/usr/bin/env python3
"""Held-out continuity contradiction corpus (independent evaluation).

This module deliberately does not import or inspect detector implementations or
the existing fixture prose. It seeds two new 16-chapter manuscripts through the
public MCP tools, plants four contradictions for each public contradiction type
(28 per manuscript), and includes near-miss controls for negation, hypotheticals,
flashbacks, dreams, quoted rumors and mentions.

Run through `python3 evals/continuity_recall.py held-out` so the same disposable
server and scoring pipeline are used. The existing benchmark fixtures and
baseline are untouched.
"""
from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import tempfile
import time
from typing import Any

from continuity_recall import (
    CONTRADICTION_TYPES,
    McpClient,
    PlantedContradiction,
    issue_matches_contradiction,
    score_manuscript,
)

EXPECTED_CHECK_TYPES: dict[str, tuple[str, ...]] = {
    "character_attribute": ("canonical_fact_prose_drift", "character_attribute"),
    "dead_character_acting": (
        "dead_character_acting",
        "canonical_fact_prose_drift",
        "retcon",
    ),
    "timeline_chronology_rewind": (
        "timeline_chronology_rewind",
        "chronology",
        "temporal_coherence",
    ),
    "knowledge_leak_before_learned": (
        "knowledge_leak_before_learned",
        "knowledge_timing",
    ),
    "world_rule_cost_violation": (
        "world_rule_cost_violation",
        "world_rule_semantic_drift",
        "world_rule_compliance",
    ),
    "quantity_affordability": (
        "quantity_affordability",
        "affordability",
        "quantity_drift",
    ),
    "secret_leak": ("secret_leak",),
}

# Each manuscript declares clean chapter prose plus contradiction phrases. The
# chapter/type assignment is fixed before the run; no detector was consulted.
MANUSCRIPTS: list[dict[str, Any]] = [
    {
        "id": "cartographers_wake",
        "title": "The Cartographer's Wake",
        "genre": "comic bureaucratic fantasy",
        "voice": "dry close third, material detail, understated jokes",
        "promise": "A comic bureaucratic fantasy about maps, debts and the names a harbor forgets.",
        "characters": [
            ("miro", "Miro Voss", "protagonist", "left-handed cartographer with an unreliable memory for names"),
            ("sela", "Sela Quill", "ally", "grey-eyed harbor clerk who keeps the permit cabinet"),
            ("brann", "Brann Tallow", "ally", "tall surveyor who measures everything twice"),
            ("orla", "Orla Finch", "ally", "archivist with a scar on her left cheek"),
            ("oren", "Oren Pike", "quartermaster", "quartermaster who dies at the opening tide"),
            ("lira", "Lira Vane", "pilot", "pilot who dies in the first squall"),
            ("piet", "Piet Moss", "clerk", "clerk who dies under a crate of sounding leads"),
            ("sera", "Sera Bell", "guard", "seawall guard who dies at the north gate"),
            ("toma", "Toma Reed", "dockhand", "dockhand with a talent for repeating what he overhears"),
            ("nessa", "Nessa Wren", "courier", "courier who trades in other people's business"),
        ],
        "locations": [
            ("chart_room", "The Chart Room", "office", "A warm room of wet vellum, coal smoke and unshelved rolls."),
            ("dry_dock", "The Dry Dock", "dock", "A tidal dock where hulls are scraped and debts are counted."),
        ],
        "world_rule": {
            "name": "Lantern-ink",
            "description": "Sealing a chart with lantern-ink costs the cartographer one remembered name.",
            "scan_pattern": "lantern-ink",
            "tags": ["ink", "memory", "cost"],
        },
        "attributes": [
            ("miro", "handedness", "left", 1, "Miro signed the ledger with his right hand and blotted the ink."),
            ("sela", "eye_color", "grey", 4, "Sela's brown eyes narrowed at the permit."),
            ("brann", "height", "tall", 8, "Brann could not reach the top shelf, even standing on the map chest."),
            ("orla", "scar_location", "left cheek", 11, "Orla turned her right cheek to the candle, showing the scar."),
        ],
        "deaths": [
            ("oren", 1, 3, "Oren leaned over the chart and tapped the reef with one finger."),
            ("lira", 2, 6, "Lira stepped out of the squall and asked for her flight log."),
            ("piet", 5, 10, "Piet set the ledger down and complained about the damp."),
            ("sera", 7, 13, "Sera opened the seawall gate and waved Miro through."),
        ],
        "knowledge": [
            ("cistern", "sela", "the cistern gate opens at the third bell", 8, 2,
             "Sela said, \"the cistern gate opens at the third bell.\""),
            ("cipher", "miro", "the dry dock ledger uses a double-entry cipher", 10, 6,
             "Miro said, \"the dry dock ledger uses a double-entry cipher.\""),
            ("north_channel", "brann", "the north channel is silted to the second marker", 12, 9,
             "Brann said, \"the north channel is silted to the second marker.\""),
            ("charter_seal", "orla", "the charter seal is wax over lead", 14, 13,
             "Orla said, \"the charter seal is wax over lead.\""),
        ],
        "secrets": [
            ("heir", "miro", "toma", "The heir's name is written in the tide ledger.", 3,
             "Toma said, \"The heir's name is written in the tide ledger.\""),
            ("north_sale", "sela", "nessa", "The harbor master sold the north channel.", 7,
             "Nessa said, \"The harbor master sold the north channel.\""),
            ("survey_forgery", "brann", "toma", "The survey is a forgery.", 10,
             "Toma said, \"The survey is a forgery.\""),
            ("burned_charter", "orla", "nessa", "The archivist burned the original charter.", 14,
             "Nessa said, \"The archivist burned the original charter.\""),
        ],
        "rule_uses": [
            (1, "Miro sealed the chart with lantern-ink and did not lose a single memory."),
            (5, "Brann marked the sounding line with lantern-ink and did not lose a single memory."),
            (8, "Sela copied the survey in lantern-ink and did not lose a single memory."),
            (12, "Miro sealed the harbor map in lantern-ink and did not lose a single memory."),
        ],
        "quantity": [
            ("miro", 12, 2, "Miro paid twenty silver for the ferry and still had twelve in the strongbox."),
            ("sela", 20, 5, "Sela bought a forty-silver lamp and put the change in her purse."),
            ("brann", 30, 9, "Brann settled a hundred-silver debt with a handful of coins."),
            ("orla", 40, 12, "Orla tipped the dockhands forty silver and kept the rest."),
        ],
        "timeline": [
            (4, "Three days earlier, the harbor bell had not yet cracked."),
            (7, "The next morning, Sela found the charter box already open."),
            (11, "Before the survey began, Orla had already sealed the charter."),
            (14, "Two days before the tide turned, Sela delivered the permits."),
        ],
        "chapters": [
            "The Chart Room smelled of wet vellum and coal smoke. Miro unrolled the harbor chart while Oren Pike argued with the tide clock.",
            "Rain drummed on the Dry Dock roof. Lira Vane checked the mooring lines and Sela logged the morning tide.",
            "A bell rang twice from the north channel. Miro counted the ships and found one missing from the register.",
            "The harbor clerk's office was warm and cluttered. Sela sorted the day's permits into three uneven stacks.",
            "Piet Moss dropped a crate of sounding leads on the dock. Brann measured the water line and wrote the depth in chalk.",
            "A squall closed the channel before noon. Lira's old flight log sat open on the chart table.",
            "Sera Bell walked the seawall with a lantern. Orla waited in the archive with the original charter box.",
            "The surveyors gathered in the Chart Room. Brann pinned a new map to the wall and stepped back.",
            "A customs cutter blocked the north channel. Miro counted the coins in the strongbox twice.",
            "Piet's replacement arrived with a ledger under one arm. Orla compared the ink to the older entries.",
            "The archive smelled of dust and salt. Orla laid the charter on the table and opened the seal.",
            "The tide turned early. Miro paid the dockhands and watched the last barge leave.",
            "Sera's replacement found the seawall gate unlocked. Miro checked the chart for the missing channel.",
            "Orla burned a candle down to the wick. Sela brought the final packet of permits.",
            "The Chart Room emptied slowly. Miro rolled the harbor chart and put it away.",
            "By dusk the harbor was quiet. Sela locked the permit cabinet and walked home.",
        ],
        "padding": "The lamps guttered; somewhere below, a rope creaked against the pilings.",
        "controls": {
            1: "If Miro had been right-handed, the signature would have looked different.",
            3: "Oren's letter, written before he died, was still folded in the chart case.",
            6: "In her dream, Lira laughed at the weather.",
            7: "Sela remembered the previous winter's storm.",
            9: "Miro did not know the cipher yet.",
            10: "Miro counted the coins but did not spend them.",
            12: "No seal was made that night, and no name was forgotten.",
            14: "No one said the heir's name aloud.",
            15: "A rumor claimed the survey was a forgery, but no one repeated it.",
            16: "The funeral bell rang for Oren, who had died in the spring.",
        },
    },
    {
        "id": "saltwater_hymnal",
        "title": "Saltwater Hymnal",
        "genre": "coastal folk horror",
        "voice": "lyrical close third, tidal imagery, restrained dread",
        "promise": "A coastal folk horror about songs that cost memory and debts the sea keeps.",
        "characters": [
            ("anwen", "Anwen Rill", "protagonist", "left-handed singer who keeps the chapel's tide-songs"),
            ("cade", "Cade Morrow", "ally", "grey-eyed lighthouse keeper with a careful log"),
            ("mor", "Mor Tarn", "ally", "tall boatwright who measures hulls by hand"),
            ("rhys", "Rhys Vell", "ally", "archivist with a scar on his left cheek"),
            ("ianto", "Ianto Gorse", "boatman", "boatman who dies at the opening tide"),
            ("wynn", "Wynn Spar", "rower", "rower who dies in the first fog"),
            ("eira", "Eira Sow", "netmaker", "netmaker who dies under the winter net"),
            ("gethin", "Gethin Mott", "guard", "Bell Rock guard who dies at the gate"),
            ("briar", "Briar Nook", "ferryman", "ferryman who trades in overheard confessions"),
            ("nell", "Nell Cray", "courier", "courier who carries other people's secrets"),
        ],
        "locations": [
            ("chapel", "The Tide Chapel", "chapel", "A salt-stained chapel that leans into the wind."),
            ("bell_rock", "Bell Rock", "reef", "A black reef where the bell is heard before it is seen."),
        ],
        "world_rule": {
            "name": "Tide-song",
            "description": "A tide-song sung to call the water costs the singer one memory of the sea.",
            "scan_pattern": "tide-song",
            "tags": ["song", "memory", "sea"],
        },
        "attributes": [
            ("anwen", "handedness", "left", 1, "Anwen signed the hymnal with her right hand and closed the book."),
            ("cade", "eye_color", "grey", 4, "Cade's brown eyes narrowed at the lighthouse log."),
            ("mor", "height", "tall", 8, "Mor could not reach the top shelf, even standing on the tool chest."),
            ("rhys", "scar_location", "left cheek", 11, "Rhys turned his right cheek to the candle, showing the scar."),
        ],
        "deaths": [
            ("ianto", 1, 3, "Ianto lifted the boat hook and called to Anwen across the shingle."),
            ("wynn", 2, 6, "Wynn stepped out of the fog and asked for his oar."),
            ("eira", 5, 10, "Eira set the net down and complained about the wind."),
            ("gethin", 7, 13, "Gethin opened the Bell Rock gate and waved Cade through."),
        ],
        "knowledge": [
            ("tide_gate", "cade", "the tide gate opens only at the second bell", 8, 2,
             "Cade said, \"the tide gate opens only at the second bell.\""),
            ("drowned_hymn", "anwen", "the chapel hymn is a drowned language", 10, 6,
             "Anwen sang, \"the chapel hymn is a drowned language.\""),
            ("bell_passage", "mor", "the Bell Rock passage is dry at low tide", 12, 9,
             "Mor said, \"the Bell Rock passage is dry at low tide.\""),
            ("left_saw", "rhys", "the keel was cut with a left-handed saw", 14, 13,
             "Rhys said, \"the keel was cut with a left-handed saw.\""),
        ],
        "secrets": [
            ("ship_bell", "anwen", "briar", "The chapel bell was cast from a drowned ship's bell.", 3,
             "Briar said, \"The chapel bell was cast from a drowned ship's bell.\""),
            ("missing_boat", "cade", "nell", "The lighthouse keeper's log names the missing boat.", 7,
             "Nell said, \"The lighthouse keeper's log names the missing boat.\""),
            ("sawed_keel", "mor", "briar", "The boatwright sawed the keel on purpose.", 10,
             "Briar said, \"The boatwright sawed the keel on purpose.\""),
            ("buried_mother", "rhys", "nell", "The singer's mother is buried under the chapel step.", 14,
             "Nell said, \"The singer's mother is buried under the chapel step.\""),
        ],
        "rule_uses": [
            (1, "Anwen sang the tide-song and did not lose a single memory of the sea."),
            (5, "Mor sang the tide-song over the keel and did not lose a single memory of the sea."),
            (8, "Cade sang the tide-song to the lens and did not lose a single memory of the sea."),
            (12, "Anwen sang the tide-song and did not lose a single memory of the sea."),
        ],
        "quantity": [
            ("anwen", 11, 2, "Anwen paid twenty silver for the ferry and still had eleven in the chapel box."),
            ("cade", 18, 5, "Cade bought a forty-silver lamp and put the change away."),
            ("mor", 26, 9, "Mor settled a hundred-silver debt with a handful of coins."),
            ("rhys", 34, 12, "Rhys tipped the boatmen forty silver and kept the rest."),
        ],
        "timeline": [
            (4, "Three tides earlier, the chapel bell had not yet cracked."),
            (7, "The next morning, Rhys found the chest already open."),
            (11, "Before the crossing, Cade had already written the passage in the log."),
            (14, "Two days before the tide turned, Cade brought the final page."),
        ],
        "chapters": [
            "The Tide Chapel leaned into the wind. Anwen lit the votive lamp while Ianto counted the boats.",
            "Fog swallowed Bell Rock. Wynn rowed the supply line and Cade wrote the tide in the lighthouse log.",
            "A gull circled the chapel three times. Anwen sang the opening hymn and Mor listened from the door.",
            "The lighthouse kitchen smelled of kelp and tea. Cade wiped the lens cloth and watched the grey water.",
            "Eira hauled the net onto the shingle. Mor measured the broken keel and marked the repair with chalk.",
            "The storm came in sideways. Wynn's old oar leaned against the chapel wall.",
            "Gethin walked the Bell Rock path with a lantern. Rhys waited in the chapel with the locked chest.",
            "The boatwright's shed was cold. Mor planed a new keel and set it against the wall.",
            "A customs boat crossed the channel. Anwen counted the coins in the chapel box twice.",
            "Eira's replacement brought the lighthouse log inside. Rhys compared the ink to the older pages.",
            "The chapel smelled of salt and candle smoke. Rhys laid the old chart on the altar and opened the chest.",
            "The tide turned before noon. Anwen paid the ferryman and watched the last boat leave.",
            "Gethin's replacement found the Bell Rock gate unlocked. Cade checked the log for the missing passage.",
            "Rhys burned a candle to the wick. Cade brought the final page of the log.",
            "The chapel emptied slowly. Anwen put out the votive lamp and locked the door.",
            "By dusk the fog had lifted. Cade closed the lighthouse log and walked down to the water.",
        ],
        "padding": "The tide breathed against the stones; the bell buoy answered once and fell silent.",
        "controls": {
            1: "If Anwen had been right-handed, the hymnal signature would have looked different.",
            3: "Ianto's letter, written before he died, was still folded in the chapel chest.",
            6: "In his dream, Wynn laughed at the fog.",
            7: "Cade remembered the previous winter's storm.",
            9: "Anwen did not know the drowned language yet.",
            10: "Anwen counted the coins but did not spend them.",
            12: "No song was sung that night, and no memory of the sea was lost.",
            14: "No one said the singer's mother's name aloud.",
            15: "A rumor claimed the keel was sawed on purpose, but no one repeated it.",
            16: "The funeral bell rang for Ianto, who had died in the spring.",
        },
    },
]

CONTRADICTION_CHAPTERS = {
    "character_attribute": (1, 4, 8, 11),
    "dead_character_acting": (3, 6, 10, 13),
    "timeline_chronology_rewind": (4, 7, 11, 14),
    "knowledge_leak_before_learned": (2, 6, 9, 13),
    "world_rule_cost_violation": (1, 5, 8, 12),
    "quantity_affordability": (2, 5, 9, 12),
    "secret_leak": (3, 7, 10, 14),
}


@dataclass(frozen=True)
class SeededIds:
    project_id: str
    characters: dict[str, str]
    locations: dict[str, str]
    world_rule_id: str
    chapter_scenes: dict[int, str]
    secret_fact_ids: dict[str, str]
    knowledge_fact_ids: dict[str, str]


def _tool(client: McpClient, name: str, payload: dict[str, Any]) -> dict[str, Any]:
    return client.tool(name, payload)


def _create_project(client: McpClient, spec: dict[str, Any]) -> str:
    project = _tool(
        client,
        "create_project",
        {
            "name": spec["title"],
            "project_type": "novel",
            "genre": spec["genre"],
            "reader_contract": {
                "promise": spec["promise"],
                "style_notes": [spec["voice"]],
                "boundaries": [],
            },
        },
    )
    return project["project_id"]


def _seed_setup(client: McpClient, spec: dict[str, Any], project_id: str) -> SeededIds:
    _tool(
        client,
        "set_project_calendar",
        {
            "project_id": project_id,
            "calendar": {
                "days_per_week": 7,
                "hours_per_day": 24,
                "week_day_names": ["First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh"],
                "months": [
                    {"name": "Tide", "days": 30},
                    {"name": "Salt", "days": 30},
                    {"name": "Fog", "days": 30},
                    {"name": "Bell", "days": 30},
                    {"name": "Wrack", "days": 30},
                    {"name": "Ember", "days": 30},
                ],
                "days_per_year": 180,
                "epoch_label": "the first bell",
            },
        },
    )
    _tool(
        client,
        "set_project_quantity_scheme",
        {
            "project_id": project_id,
            "scheme": {
                "measure": "silver",
                "denominations": [
                    {"name": "copper", "per_base": 1},
                    {"name": "silver", "per_base": 100},
                    {"name": "crown", "per_base": 1000},
                ],
                "bands": [
                    {"name": "destitute"},
                    {"name": "strained"},
                    {"name": "comfortable"},
                    {"name": "wealthy"},
                ],
                "max_band_jump": 1,
            },
        },
    )
    characters: dict[str, str] = {}
    for key, name, role, summary in spec["characters"]:
        out = _tool(
            client,
            "create_character",
            {
                "project_id": project_id,
                "name": name,
                "summary": summary,
                "role": role,
                "realm": None,
                "aliases": [name.split()[0]],
                "voice_profile": {
                    "tone": spec["voice"],
                    "vocabulary": ["tide", "ledger", "bell"],
                    "sentence_structure": ["short declarative sentences"],
                    "tics": ["counts twice"],
                    "forbidden_words": [],
                    "example_lines": [f"{name} counted the tide twice before speaking."],
                    "established_in_scene_id": None,
                    "updated_at": None,
                },
                "emotional_profile": {
                    "base_emotions": {"steady": 0.6, "wary": 0.4},
                    "suppressed": ["fear"],
                    "triggers": ["a bell without a ringer"],
                    "defense_mechanisms": ["work", "dry humor"],
                    "flex_range": {"low": "guarded", "high": "open"},
                },
                "initial_state": None,
            },
        )
        characters[key] = out["character_id"]
    locations: dict[str, str] = {}
    for key, name, kind, summary in spec["locations"]:
        out = _tool(
            client,
            "create_location",
            {
                "project_id": project_id,
                "name": name,
                "kind": kind,
                "realm": None,
                "summary": summary,
                "initial_state": {},
            },
        )
        locations[key] = out["location_id"]
    rule = _tool(
        client,
        "create_world_rule",
        {
            "project_id": project_id,
            "rule_name": spec["world_rule"]["name"],
            "rule_type": "magic",
            "description": spec["world_rule"]["description"],
            "scan_pattern": spec["world_rule"]["scan_pattern"],
            "relevance_tags": spec["world_rule"]["tags"],
            "established_in": None,
        },
    )
    return SeededIds(
        project_id=project_id,
        characters=characters,
        locations=locations,
        world_rule_id=rule["world_rule_id"],
        chapter_scenes={},
        secret_fact_ids={},
        knowledge_fact_ids={},
    )


def _register_facts(client: McpClient, spec: dict[str, Any], ids: SeededIds) -> SeededIds:
    names = {key: name for key, name, _role, _summary in spec["characters"]}
    attribute_aliases = {
        "handedness": ["hand", "handed", "writing hand"],
        "eye_color": ["eyes", "eye colour", "gaze"],
        "height": ["tall", "stature", "height"],
        "scar_location": ["scar", "cheek"],
    }
    for character_key, predicate, value, _chapter, _phrase in spec["attributes"]:
        _tool(
            client,
            "register_canonical_fact",
            {
                "project_id": ids.project_id,
                "book_number": 1,
                "chapter_number": 1,
                "fact_type": "attribute",
                "key": f"{names[character_key]}:{predicate}",
                "value": value,
                "context": "held-out invariant attribute",
                "subject_table": "character",
                "subject_id": ids.characters[character_key],
                "predicate": predicate,
                "value_kind": "text",
                "value_text": value,
                "aliases": attribute_aliases.get(predicate, [predicate]),
                "scope": "invariant",
                "valid_from": {
                    "book_number": 1,
                    "chapter_number": 1,
                    "scene_order": 1,
                    "note": None,
                },
            },
        )
    for character_key, death_chapter, _act_chapter, _phrase in spec["deaths"]:
        _tool(
            client,
            "register_canonical_fact",
            {
                "project_id": ids.project_id,
                "book_number": 1,
                "chapter_number": death_chapter,
                "fact_type": "life_status",
                "key": f"{names[character_key]}:life_status",
                "value": "dead",
                "context": "held-out death fact",
                "subject_table": "character",
                "subject_id": ids.characters[character_key],
                "predicate": "life_status",
                "value_kind": "text",
                "value_text": "dead",
                "aliases": [names[character_key], "dead"],
                "scope": "invariant",
                "valid_from": {
                    "book_number": 1,
                    "chapter_number": death_chapter,
                    "scene_order": 1,
                    "note": None,
                },
            },
        )
    secret_fact_ids: dict[str, str] = {}
    for key, holder_key, _leaker_key, text, _leak_chapter, _phrase in spec["secrets"]:
        out = _tool(
            client,
            "register_canonical_fact",
            {
                "project_id": ids.project_id,
                "book_number": 1,
                "chapter_number": 1,
                "fact_type": "secret",
                "key": f"secret:{key}",
                "value": text,
                "context": "held-out secret",
                "subject_table": "character",
                "subject_id": ids.characters[holder_key],
                "predicate": "secret",
                "value_kind": "text",
                "value_text": text,
                "aliases": [key],
                "scope": "invariant",
                "secrecy": {
                    "holder_ids": [ids.characters[holder_key]],
                    "concealment_note": "Do not reveal outside the holder circle.",
                },
            },
        )
        secret_fact_ids[key] = out["canonical_fact_id"]
    return SeededIds(
        project_id=ids.project_id,
        characters=ids.characters,
        locations=ids.locations,
        world_rule_id=ids.world_rule_id,
        chapter_scenes=ids.chapter_scenes,
        secret_fact_ids=secret_fact_ids,
        knowledge_fact_ids=ids.knowledge_fact_ids,
    )


def _chapter_text(spec: dict[str, Any], chapter_number: int, plant: bool) -> str:
    text = spec["chapters"][chapter_number - 1] + " " + spec["padding"]
    control = spec["controls"].get(chapter_number)
    if control:
        text += " " + control
    if plant:
        for character_key, _predicate, _value, chapter, phrase in spec["attributes"]:
            if chapter == chapter_number:
                text += " " + phrase
        for _character_key, _death_chapter, act_chapter, phrase in spec["deaths"]:
            if act_chapter == chapter_number:
                text += " " + phrase
        for chapter, phrase in spec["timeline"]:
            if chapter == chapter_number:
                text += " " + phrase
        for _key, _knower, _fact, _learned, leak_chapter, phrase in spec["knowledge"]:
            if leak_chapter == chapter_number:
                text += " " + phrase
        for chapter, phrase in spec["rule_uses"]:
            if chapter == chapter_number:
                text += " " + phrase
        for _character, _initial, bad_chapter, phrase in spec["quantity"]:
            if bad_chapter == chapter_number:
                text += " " + phrase
        for _key, _holder, _leaker, _text, leak_chapter, phrase in spec["secrets"]:
            if leak_chapter == chapter_number:
                text += " " + phrase
    return text


def _required_cast(spec: dict[str, Any], chapter_number: int) -> set[str]:
    required: set[str] = set()
    for character_key, _predicate, _value, chapter, _phrase in spec["attributes"]:
        if chapter == chapter_number:
            required.add(character_key)
    for character_key, _death_chapter, act_chapter, _phrase in spec["deaths"]:
        if act_chapter == chapter_number:
            required.add(character_key)
    for _key, knower_key, _fact, _learned, leak_chapter, _phrase in spec["knowledge"]:
        if leak_chapter == chapter_number:
            required.add(knower_key)
    for _key, _holder, leaker, _text, leak_chapter, _phrase in spec["secrets"]:
        if leak_chapter == chapter_number:
            required.add(leaker)
    return required


def _cast_for_chapter(
    spec: dict[str, Any], chapter_number: int, text: str, plant: bool
) -> list[str]:
    death_acts = {
        character_key: act_chapter
        for character_key, _death_chapter, act_chapter, _phrase in spec["deaths"]
    }
    keys: list[str] = []
    for key, name, _role, _summary in spec["characters"]:
        if key in death_acts:
            # Dead characters are cast only when they act in the planted copy;
            # clean-copy letters, dreams and mentions must not smuggle them in.
            if plant and death_acts[key] == chapter_number:
                keys.append(key)
            continue
        first_name = name.split()[0]
        if name in text or first_name in text:
            keys.append(key)
    if plant:
        for key in sorted(_required_cast(spec, chapter_number)):
            if key not in keys:
                keys.append(key)
    return keys


def seed_held_out_manuscript(
    client: McpClient, spec: dict[str, Any], plant_contradictions: bool
) -> tuple[str, list[PlantedContradiction]]:
    project_id = _create_project(client, spec)
    ids = _seed_setup(client, spec, project_id)
    ids = _register_facts(client, spec, ids)

    chapter_scenes: dict[int, str] = {}
    for chapter_number in range(1, len(spec["chapters"]) + 1):
        if chapter_number > 1:
            _tool(
                client,
                "create_chapter",
                {
                    "project_id": project_id,
                    "book_number": 1,
                    "chapter_number": chapter_number,
                },
            )
        text = _chapter_text(spec, chapter_number, plant_contradictions)
        location_key = spec["locations"][chapter_number % len(spec["locations"])][0]
        knowledge_learned = [
            {
                "character_id": ids.characters[knower_key],
                "fact": fact_text,
                "reader_visible": False,
                "source_summary": "Held-out acquisition scene.",
            }
            for _key, knower_key, fact_text, learned_chapter, _leak, _phrase in spec["knowledge"]
            if learned_chapter == chapter_number
        ]
        saved = _tool(
            client,
            "save_scene_draft",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter_number,
                "scene_order": 1,
                "full_text": text,
                "summary": spec["chapters"][chapter_number - 1][:120],
                "content_rating": "general",
                "tone": spec["voice"],
                "location_id": ids.locations[location_key],
                "knowledge_learned": knowledge_learned,
            },
        )
        scene_id = saved["scene_id"]
        chapter_scenes[chapter_number] = scene_id
        _tool(
            client,
            "save_summary",
            {
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": chapter_number,
                "summary": spec["chapters"][chapter_number - 1],
            },
        )
        cast = _cast_for_chapter(spec, chapter_number, text, plant_contradictions)
        if plant_contradictions:
            missing = _required_cast(spec, chapter_number) - set(cast)
            assert not missing, (spec["id"], chapter_number, missing)
        _tool(
            client,
            "commit_scene_changes",
            {
                "project_id": project_id,
                "scene_id": scene_id,
                "character_states": [
                    {
                        "character_id": ids.characters[key],
                        "changes": {"notes": ["present in this scene"]},
                    }
                    for key in cast
                ],
                "canonical_facts": [],
                "relationship_updates": [],
                "accept_world_rule_risks": True,
                "accept_continuity_risks": True,
                "continuity_gate": "off",
            },
        )
        base_day = 100 + chapter_number * 3
        if plant_contradictions and chapter_number in {
            chapter for chapter, _phrase in spec["timeline"]
        }:
            base_day -= 10
        _tool(
            client,
            "set_scene_clock",
            {
                "project_id": project_id,
                "scene_id": scene_id,
                "clock": {
                    "day_index": base_day,
                    "time_of_day": 8 * 60,
                    "duration_days": 0.5,
                    "precision": "hour",
                },
                "temporal_mode": "linear",
            },
        )

    for character_key, initial, bad_chapter, _spend in spec["quantity"]:
        _tool(
            client,
            "commit_quantity_state",
            {
                "project_id": project_id,
                "subject_table": "character",
                "subject_id": ids.characters[character_key],
                "measure": "silver",
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "state": {"amount": initial, "unit": "silver", "band": "comfortable"},
            },
        )
        if plant_contradictions:
            _tool(
                client,
                "commit_quantity_state",
                {
                    "project_id": project_id,
                    "subject_table": "character",
                    "subject_id": ids.characters[character_key],
                    "measure": "silver",
                    "book_number": 1,
                    "chapter_number": bad_chapter,
                    "scene_order": 1,
                    "state": {"amount": 0, "unit": "silver", "band": "destitute"},
                },
            )

    knowledge_fact_ids: dict[str, str] = {}
    contradictions: list[PlantedContradiction] = []
    if plant_contradictions:
        for character_key, predicate, value, chapter, phrase in spec["attributes"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.attribute.{character_key}",
                    contradiction_type="character_attribute",
                    variant="natural_prose",
                    chapter_number=chapter,
                    description=f"{predicate} established as {value}",
                    expected_check_types=EXPECTED_CHECK_TYPES["character_attribute"],
                    match_tokens=(chapter_scenes[chapter],),
                )
            )
        for character_key, death_chapter, act_chapter, _phrase in spec["deaths"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.dead.{character_key}",
                    contradiction_type="dead_character_acting",
                    variant="present_action",
                    chapter_number=act_chapter,
                    description=f"{character_key} died in chapter {death_chapter} but acts in chapter {act_chapter}",
                    expected_check_types=EXPECTED_CHECK_TYPES["dead_character_acting"],
                    match_tokens=(chapter_scenes[act_chapter],),
                )
            )
        for chapter, _phrase in spec["timeline"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.timeline.{chapter}",
                    contradiction_type="timeline_chronology_rewind",
                    variant="clock_rewind",
                    chapter_number=chapter,
                    description="scene clock moves backwards",
                    expected_check_types=EXPECTED_CHECK_TYPES["timeline_chronology_rewind"],
                    match_tokens=(chapter_scenes[chapter],),
                )
            )
        for key, knower_key, _fact_text, learned_chapter, leak_chapter, _phrase in spec["knowledge"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.knowledge.{key}",
                    contradiction_type="knowledge_leak_before_learned",
                    variant="used_before_learned",
                    chapter_number=leak_chapter,
                    description=f"knowledge learned in chapter {learned_chapter} is used in chapter {leak_chapter}",
                    expected_check_types=EXPECTED_CHECK_TYPES["knowledge_leak_before_learned"],
                    match_tokens=(chapter_scenes[leak_chapter],),
                )
            )
        for chapter, _phrase in spec["rule_uses"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.rule.{chapter}",
                    contradiction_type="world_rule_cost_violation",
                    variant="cost_omitted",
                    chapter_number=chapter,
                    description="tide or ink rule used without its stated cost",
                    expected_check_types=EXPECTED_CHECK_TYPES["world_rule_cost_violation"],
                    match_tokens=(chapter_scenes[chapter],),
                )
            )
        for character_key, _initial, bad_chapter, _spend in spec["quantity"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.quantity.{character_key}",
                    contradiction_type="quantity_affordability",
                    variant="band_jump_and_overspend",
                    chapter_number=bad_chapter,
                    description="quantity state drops two bands without a change reason",
                    expected_check_types=EXPECTED_CHECK_TYPES["quantity_affordability"],
                    match_tokens=(ids.characters[character_key],),
                )
            )
        for key, holder_key, leaker_key, _text, leak_chapter, _phrase in spec["secrets"]:
            contradictions.append(
                PlantedContradiction(
                    contradiction_id=f"{spec['id']}.secret.{key}",
                    contradiction_type="secret_leak",
                    variant="non_holder_dialogue",
                    chapter_number=leak_chapter,
                    description=f"secret held by {holder_key} is disclosed by {leaker_key}",
                    expected_check_types=EXPECTED_CHECK_TYPES["secret_leak"],
                    match_tokens=(chapter_scenes[leak_chapter],),
                )
            )

    return project_id, contradictions


def run_held_out_benchmark(
    binary: Path,
    deep_check: bool = False,
    agent_config: Path | None = None,
) -> dict[str, Any]:
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="spindle-continuity-held-out-") as tmp:
        client = McpClient(binary, Path(tmp), agent_config=agent_config)
        try:
            reports = []
            for spec in MANUSCRIPTS:
                control_project_id, _ = seed_held_out_manuscript(client, spec, False)
                control_check = client.tool(
                    "check_consistency",
                    {
                        "project_id": control_project_id,
                        "scope": {"scope_type": "full"},
                        "checks": [],
                        "severity_filter": [],
                        "deep_check": deep_check,
                        "subjects": [],
                    },
                )
                planted_project_id, planted_items = seed_held_out_manuscript(client, spec, True)
                planted_check = client.tool(
                    "check_consistency",
                    {
                        "project_id": planted_project_id,
                        "scope": {"scope_type": "full"},
                        "checks": [],
                        "severity_filter": [],
                        "deep_check": deep_check,
                        "subjects": [],
                    },
                )
                reports.append(
                    score_manuscript(
                        manuscript_id=spec["id"],
                        chapter_count=len(spec["chapters"]),
                        control_issues=control_check.get("issues", []),
                        planted_issues=planted_check.get("issues", []),
                        planted_contradictions=planted_items,
                    )
                )
        finally:
            client.close()

    aggregate: dict[str, Any] = {}
    for ct in CONTRADICTION_TYPES:
        planted = sum(report["per_type"][ct]["planted"] for report in reports)
        detected = sum(report["per_type"][ct]["detected"] for report in reports)
        aggregate[ct] = {
            "planted": planted,
            "detected": detected,
            "recall": round(detected / planted, 4) if planted else 0.0,
        }
    total_planted = sum(report["planted_total"] for report in reports)
    total_detected = sum(report["detected_total"] for report in reports)
    control_fp_by_type: dict[str, int] = {}
    for report in reports:
        for check_type, count in report["control_false_positives_by_check_type"].items():
            control_fp_by_type[check_type] = control_fp_by_type.get(check_type, 0) + count
    return {
        "benchmark": "continuity_recall_held_out",
        "version": 1,
        "tier": "deep" if deep_check else "deterministic",
        "held_out": True,
        "manuscript_count": len(reports),
        "chapters_per_manuscript": [report["chapters"] for report in reports],
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "summary": {
            "total_planted": total_planted,
            "total_detected": total_detected,
            "overall_recall": round(total_detected / total_planted, 4) if total_planted else 0.0,
            "control_false_positive_count": sum(
                report["control_false_positive_count"] for report in reports
            ),
            "control_false_positives_by_check_type": dict(sorted(control_fp_by_type.items())),
            "per_type_recall": aggregate,
        },
        "manuscripts": reports,
    }


def held_out_self_test() -> None:
    assert len(MANUSCRIPTS) >= 2
    for spec in MANUSCRIPTS:
        assert len(spec["chapters"]) >= 15, spec["id"]
        counts = {ct: 0 for ct in CONTRADICTION_TYPES}
        for _character, _predicate, _value, chapter, _phrase in spec["attributes"]:
            counts["character_attribute"] += 1
            assert 1 <= chapter <= len(spec["chapters"])
        for _character, death_chapter, act_chapter, _phrase in spec["deaths"]:
            counts["dead_character_acting"] += 1
            assert death_chapter < act_chapter
        for chapter, _phrase in spec["timeline"]:
            counts["timeline_chronology_rewind"] += 1
            assert chapter > 1
        for _key, _knower, _fact, learned, leak, _phrase in spec["knowledge"]:
            counts["knowledge_leak_before_learned"] += 1
            assert leak < learned
        for _chapter, _phrase in spec["rule_uses"]:
            counts["world_rule_cost_violation"] += 1
        for _character, _initial, bad_chapter, _spend in spec["quantity"]:
            counts["quantity_affordability"] += 1
            assert bad_chapter > 1
        for _key, _holder, _leaker, _text, leak_chapter, _phrase in spec["secrets"]:
            counts["secret_leak"] += 1
            assert 1 <= leak_chapter <= len(spec["chapters"])
        assert all(count >= 4 for count in counts.values()), (spec["id"], counts)
        # No chapter carries two contradictions of the same type, so
        # type+chapter scoring is unambiguous.
        by_chapter: dict[tuple[str, int], int] = {}
        for kind, chapters in CONTRADICTION_CHAPTERS.items():
            for chapter in chapters:
                by_chapter[(kind, chapter)] = by_chapter.get((kind, chapter), 0) + 1
        assert all(count == 1 for count in by_chapter.values()), spec["id"]
        assert spec["controls"], spec["id"]
    # Representative issue shapes: Phase-4 findings carry the planted scene id
    # in entity_ids and often no chapter marker in the message. The matcher must
    # accept those shapes and reject a different scene.
    shape_cases = [
        ("character_attribute", "canonical_fact_prose_drift"),
        ("dead_character_acting", "retcon"),
        ("knowledge_leak_before_learned", "knowledge_timing"),
        ("secret_leak", "secret_leak"),
    ]
    # Quantity findings must match only their own subject, not every silver
    # finding through a shared measure token.
    quantity_items = [
        PlantedContradiction(
            contradiction_id=f"shape.quantity.{subject}",
            contradiction_type="quantity_affordability",
            variant="shape_test",
            chapter_number=5,
            description="shape test",
            expected_check_types=("quantity_drift",),
            match_tokens=(subject,),
        )
        for subject in ("character:quantity-a", "character:quantity-b")
    ]
    issue = {
        "check_type": "quantity_drift",
        "entity_ids": ["character:quantity-a"],
        "message": "silver for 'character:quantity-a' jumped 2 tiers",
    }
    assert issue_matches_contradiction(issue, quantity_items[0])
    assert not issue_matches_contradiction(issue, quantity_items[1])

    for contradiction_type, check_type in shape_cases:
        scene_id = "scene:held-out-shape"
        item = PlantedContradiction(
            contradiction_id=f"shape.{contradiction_type}",
            contradiction_type=contradiction_type,
            variant="shape_test",
            chapter_number=4,
            description="shape test",
            expected_check_types=(check_type,),
            match_tokens=(scene_id,),
        )
        issue = {
            "check_type": check_type,
            "entity_ids": [scene_id],
            "message": "representative held-out finding without a chapter marker",
        }
        assert issue_matches_contradiction(issue, item), (contradiction_type, issue)
        wrong_scene = dict(issue, entity_ids=["scene:other"])
        assert not issue_matches_contradiction(wrong_scene, item), contradiction_type

    print("continuity held-out corpus self-test: ok")
    print(
        "  manuscripts="
        + str(len(MANUSCRIPTS))
        + " chapters="
        + str([len(spec["chapters"]) for spec in MANUSCRIPTS])
        + " planted_each="
        + str([sum(len(spec[key]) for key in ("attributes", "deaths", "timeline", "knowledge", "rule_uses", "quantity", "secrets")) for spec in MANUSCRIPTS])
    )


if __name__ == "__main__":
    held_out_self_test()

#!/usr/bin/env python3
"""Plan-quality evaluation harness for the Spindle planning engine.

Commands:
  self-test    Run offline unit tests for bracketing, gates, rubric judging,
               swap-order debiasing, bootstrap CIs, calibration and release gates.
  gate         Run Layer 1 deterministic hard gates over fixture plans.
  judge        Run Layer 2 pairwise rubric judging with swapped orders and two
               independent judge families. Offline fixtures by default; a live
               judge config can be selected explicitly.
  calibrate    Compare judge consensus against the calibration pilot file.
  report       Run gate + judge + calibration and print one release report.

This harness never treats a model output as authoritative: Layer 1 hard gates
must pass before a candidate plan is eligible for a release-quality claim, and
offline fixtures are labelled as fixtures. It never prints secrets.
"""
from __future__ import annotations

import argparse
from collections import Counter
import copy
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import random
import re
import statistics
import sys

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import prose_judge  # noqa: E402

BRIEFS_PATH = HERE / "plan_judge" / "briefs.json"
JUDGES_PATH = HERE / "plan_judge" / "judges.json"
CALIBRATION_PATH = HERE / "plan_judge_calibration.json"

SCHEMA_VERSION = 1
SUITE_ID = "spindle.plan_judge.v1"

RUBRIC_DIMENSIONS = (
    "contract_fidelity",
    "causal_progression",
    "character_agency",
    "arc_credibility",
    "promise_setup_progress_payoff",
    "world_feasibility",
    "pov_allocation",
    "pacing_fit",
    "scene_specificity_draftability",
    "room_for_discovery",
)

VERDICT_LABELS = ("baseline", "candidate", "tie")

KNOWN_OPERATIONS = {
    "create_project",
    "create_character",
    "create_location",
    "create_world_rule",
    "create_relationship",
    "create_plot_line",
    "create_conflict",
    "create_theme",
    "create_motif",
    "create_narrative_promise",
    "create_character_arc",
    "create_pacing_config",
    "create_pacing_curve",
    "set_arc_pacing_constraints",
    "create_book",
    "create_chapter",
    "set_book_outline",
    "set_chapter_outline",
    "plan_chapter",
    "update_reader_contract",
}

RATING_RANK = {"general": 0, "teen": 1, "mature": 2, "explicit": 3}

OFFLINE_JUDGES = [
    {
        "id": "fixture-structural-critic",
        "family": "fixture-structural",
        "provider": "fake",
        "profile": "structural",
    },
    {
        "id": "fixture-thematic-critic",
        "family": "fixture-thematic",
        "provider": "fake",
        "profile": "thematic",
    },
]

RELEASE_GATE = {
    "candidate_win_rate_excluding_ties": 0.60,
    "candidate_win_rate_ci_low": 0.50,
    "max_contract_fidelity_regressions": 0,
}

WORD_RE = re.compile(r"[a-z0-9][a-z0-9'-]+")
STOPWORDS = {
    "the", "a", "an", "and", "or", "but", "of", "to", "in", "on", "for", "with",
    "is", "are", "was", "were", "be", "been", "as", "at", "by", "from", "that",
    "this", "it", "its", "into", "than", "then", "so", "not", "no", "must", "can",
    "will", "would", "should", "could", "their", "they", "them", "she", "he",
    "his", "her", "who", "what", "when", "where", "why", "how", "each", "every",
    "after", "before", "while", "because", "about", "over", "under", "between",
}


def read_json(path: Path | str):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path: Path | str, value) -> None:
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def digest(value) -> str:
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, ensure_ascii=False).encode("utf-8")
    ).hexdigest()


def tokens(text: str) -> list[str]:
    return [
        word
        for word in WORD_RE.findall(str(text).lower())
        if word not in STOPWORDS and len(word) > 2
    ]


def overlap_ratio(text: str, reference: str) -> float:
    wanted = set(tokens(reference))
    if not wanted:
        return 0.0
    got = set(tokens(text)) & wanted
    return round(len(got) / len(wanted), 4)


def load_briefs(path: Path | str = BRIEFS_PATH) -> list[dict]:
    data = read_json(path)
    return data["briefs"] if isinstance(data, dict) and "briefs" in data else data


def load_judges(path: Path | str = JUDGES_PATH) -> list[dict]:
    data = read_json(path)
    return data["offline_judges"] if isinstance(data, dict) else data


def brief_by_id(briefs: list[dict]) -> dict[str, dict]:
    return {brief["id"]: brief for brief in briefs}


# =============================================================================
# Fixture plan construction
# =============================================================================

def _pov_for_chapter(brief: dict, index: int) -> str:
    policy = brief.get("pov_policy") or {}
    if policy.get("single_pov"):
        return policy["single_pov"]
    roster = policy.get("allowed_roster") or [cast["ref"] for cast in brief["cast"]]
    return roster[index % len(roster)]


def _declaration_refs(brief: dict) -> dict[str, list[str]]:
    def refs(key: str) -> list[str]:
        return [item["ref"] for item in brief.get(key, [])]

    return {
        "cast": refs("cast"),
        "locations": refs("locations"),
        "world_rules": refs("world_rules"),
        "themes": refs("themes"),
        "conflicts": refs("conflicts"),
        "plot_lines": refs("plot_lines"),
        "arcs": refs("arcs"),
        "promises": refs("promises"),
    }


def _promise_event(promise: dict, action: str, placement: dict) -> dict:
    return {
        "promise_ref": promise["ref"],
        "action": action,
        "placement": {
            "book_number": placement["book_number"],
            "chapter_number": placement["chapter_number"],
            "scene_order": placement["scene_order"],
        },
        **({"note": promise["deferred_reason"]} if promise.get("deferred_reason") else {}),
    }


def fixture_plan(brief: dict, arm: str = "candidate") -> dict:
    """Build a deterministic plan fixture for one brief.

    `candidate` is specific and causally linked; `baseline` is structurally
    valid but deliberately generic. Both are labelled fixtures: they prove
    evaluation plumbing, not literary quality.
    """
    if arm not in ("baseline", "candidate"):
        raise ValueError(f"unknown fixture arm: {arm}")
    horizon = brief["horizon"]
    cast = brief["cast"]
    locations = brief["locations"]
    promise = brief.get("promises", [{}])[0] if brief.get("promises") else None
    declarations = _declaration_refs(brief)

    chapters = []
    previous_scene_ref: str | None = None
    global_scene = 0
    for chapter_index, chapter in enumerate(brief["chapters"]):
        chapter_number = chapter["chapter_number"]
        pov_ref = _pov_for_chapter(brief, chapter_index)
        pov_name = next(
            (item["name"] for item in cast if item["ref"] == pov_ref),
            pov_ref,
        )
        scenes = []
        for scene_index, scene_seed in enumerate(chapter["scenes"], start=1):
            global_scene += 1
            scene_ref = f"chapter:{chapter_number}:scene:{scene_index}"
            if arm == "candidate":
                summary = scene_seed
                purpose = (
                    f"{pov_name} must choose between protecting {brief['stakes']} "
                    f"and the cost of {chapter['turn'].rstrip('.')}."
                )
                beats = [
                    {"beat": "turn", "detail": scene_seed},
                    {"beat": "consequence", "detail": chapter["turn"]},
                ]
                scene_rating = brief["rating_ceiling"]
            else:
                summary = (
                    f"Scene {chapter_number}.{scene_index}: the main situation "
                    f"develops and the characters respond."
                )
                purpose = "Advances the main plot."
                beats = [{"beat": "development", "detail": "The situation changes."}]
                scene_rating = brief["rating_ceiling"]
            scene_cast = [pov_ref]
            for item in cast:
                if item["ref"] != pov_ref and len(scene_cast) < 2:
                    scene_cast.append(item["ref"])
            scenes.append(
                {
                    "scene_order": scene_index,
                    "summary": summary,
                    "purpose": purpose,
                    "cast_refs": scene_cast,
                    "location_ref": locations[global_scene % len(locations)]["ref"],
                    "content_rating": scene_rating,
                    "beats": beats,
                    "causal_from": [previous_scene_ref] if previous_scene_ref else [],
                }
            )
            previous_scene_ref = scene_ref
        chapters.append(
            {
                "chapter_number": chapter_number,
                "pov_ref": pov_ref,
                "synopsis": (
                    chapter["turn"]
                    if arm == "candidate"
                    else f"Chapter {chapter_number} continues the main thread."
                ),
                "scenes": scenes,
            }
        )

    promise_events = []
    if promise:
        planted = {
            "book_number": horizon["book_number"],
            "chapter_number": brief["chapters"][0]["chapter_number"],
            "scene_order": 1,
        }
        promise_events.append(_promise_event(promise, "plant", planted))
        if promise.get("deferred_reason"):
            promise_events.append(
                _promise_event(
                    promise,
                    "defer",
                    {
                        "book_number": horizon["book_number"],
                        "chapter_number": horizon["chapter_end"],
                        "scene_order": horizon["scenes_per_chapter"],
                    },
                )
            )
        else:
            promise_events.append(
                _promise_event(
                    promise,
                    "payoff",
                    {
                        "book_number": horizon["book_number"],
                        "chapter_number": horizon["chapter_end"],
                        "scene_order": horizon["scenes_per_chapter"],
                    },
                )
            )

    pov_plan = [
        {
            "book_number": horizon["book_number"],
            "chapter_number": chapter["chapter_number"],
            "pov_character_id": _pov_for_chapter(brief, index),
        }
        for index, chapter in enumerate(brief["chapters"])
    ]
    intensity_targets = brief.get("intensity_targets") or [
        {
            "book_number": horizon["book_number"],
            "chapter_number": chapter["chapter_number"],
            "target_intensity": round(0.2 + 0.6 * index / max(1, len(brief["chapters"]) - 1), 2),
        }
        for index, chapter in enumerate(brief["chapters"])
    ]

    operations = []
    for item in cast:
        operations.append(
            {
                "output_ref": item["ref"],
                "operation": {
                    "operation": "create_character",
                    "input": {"name": item["name"], "role": item["role"]},
                },
            }
        )
    for item in locations:
        operations.append(
            {
                "output_ref": item["ref"],
                "operation": {
                    "operation": "create_location",
                    "input": {"name": item["name"], "kind": item.get("kind", "place")},
                },
            }
        )
    for item in brief.get("world_rules", []):
        operations.append(
            {
                "output_ref": item["ref"],
                "operation": {
                    "operation": "create_world_rule",
                    "input": {"rule_name": item["rule_name"], "description": item["description"]},
                },
            }
        )
    for key, operation_name in (
        ("themes", "create_theme"),
        ("conflicts", "create_conflict"),
        ("plot_lines", "create_plot_line"),
        ("arcs", "create_character_arc"),
        ("promises", "create_narrative_promise"),
    ):
        for item in brief.get(key, []):
            operations.append(
                {
                    "output_ref": item["ref"],
                    "operation": {"operation": operation_name, "input": item.get("input", {"name": item.get("name", item["ref"])})},
                }
            )
    for chapter in chapters:
        operations.append(
            {
                "output_ref": f"plan:ch{chapter['chapter_number']}",
                "operation": {
                    "operation": "plan_chapter",
                    "input": {
                        "book_number": horizon["book_number"],
                        "chapter_number": chapter["chapter_number"],
                        "pov_character_id": chapter["pov_ref"],
                        "synopsis": chapter["synopsis"],
                    },
                },
            }
        )

    assumptions = (
        ["The preceding chapters established the named cast and place."]
        if brief["kind"] != "greenfield"
        else ["The reader will accept a close third-person opening."]
    )
    if arm == "candidate":
        assumptions.append("The plan favors concrete cost over lore expansion.")
    author_questions = (
        [{"id": "q1", "question": brief.get("open_question", "Which thread should lead the next chapter?")}]
        if arm == "candidate"
        else []
    )

    return {
        "schema_version": SCHEMA_VERSION,
        "brief_id": brief["id"],
        "fixture_arm": arm,
        "horizon": copy.deepcopy(horizon),
        "contract": {
            "promise": brief["contract_promise"],
            "style_notes": list(brief.get("style_notes", [])),
            "boundaries": list(brief.get("boundaries", [])),
        },
        "declarations": declarations,
        "schedule": {
            "promise_events": promise_events,
            "pov_plan": pov_plan,
            "intensity_targets": intensity_targets,
        },
        "chapters": chapters,
        "operations": operations,
        "assumptions": assumptions,
        "author_questions": author_questions,
    }


def plan_pair(brief: dict) -> dict[str, dict]:
    return {
        "baseline": fixture_plan(brief, "baseline"),
        "candidate": fixture_plan(brief, "candidate"),
    }


# =============================================================================
# Layer 1: deterministic hard gates
# =============================================================================

def _scene_refs(plan: dict) -> list[dict]:
    refs = []
    for chapter in plan.get("chapters", []):
        for scene in chapter.get("scenes", []):
            refs.append(
                {
                    "chapter_number": chapter.get("chapter_number"),
                    "scene_order": scene.get("scene_order"),
                    "ref": f"chapter:{chapter.get('chapter_number')}:scene:{scene.get('scene_order')}",
                    "scene": scene,
                }
            )
    return refs


def check_schema(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    failures: list[str] = []
    if not isinstance(plan, dict):
        return False, ["plan must be an object"], 0.0
    if plan.get("schema_version") != SCHEMA_VERSION:
        failures.append("schema_version must be 1")
    if plan.get("brief_id") != brief["id"]:
        failures.append("brief_id does not match the brief")
    horizon = plan.get("horizon")
    if not isinstance(horizon, dict):
        failures.append("horizon must be an object")
    else:
        for key in ("book_number", "chapter_start", "chapter_end", "scenes_per_chapter"):
            if not isinstance(horizon.get(key), int) or horizon.get(key, 0) < 1:
                failures.append(f"horizon.{key} must be a positive integer")
        if horizon.get("chapter_end", 0) < horizon.get("chapter_start", 0):
            failures.append("horizon.chapter_end must be >= chapter_start")
    contract = plan.get("contract")
    if not isinstance(contract, dict) or not str(contract.get("promise", "")).strip():
        failures.append("contract.promise is required")
    declarations = plan.get("declarations")
    if not isinstance(declarations, dict):
        failures.append("declarations must be an object")
    else:
        for key in ("cast", "locations", "world_rules", "themes", "conflicts", "plot_lines", "arcs", "promises"):
            if not isinstance(declarations.get(key), list):
                failures.append(f"declarations.{key} must be an array")
    schedule = plan.get("schedule")
    if not isinstance(schedule, dict):
        failures.append("schedule must be an object")
    chapters = plan.get("chapters")
    if not isinstance(chapters, list) or not chapters:
        failures.append("chapters must be a non-empty array")
        return False, failures, 0.0
    expected_chapters = list(range(horizon["chapter_start"], horizon["chapter_end"] + 1)) if isinstance(horizon, dict) else []
    actual_chapters = [chapter.get("chapter_number") for chapter in chapters]
    if actual_chapters != expected_chapters:
        failures.append(f"chapter numbers {actual_chapters} do not cover horizon {expected_chapters}")
    for chapter in chapters:
        if not str(chapter.get("pov_ref", "")).strip():
            failures.append(f"chapter {chapter.get('chapter_number')} needs a pov_ref")
        scenes = chapter.get("scenes")
        if not isinstance(scenes, list) or not scenes:
            failures.append(f"chapter {chapter.get('chapter_number')} has no scenes")
            continue
        if isinstance(horizon, dict) and len(scenes) != horizon.get("scenes_per_chapter"):
            failures.append(
                f"chapter {chapter.get('chapter_number')} has {len(scenes)} scenes; expected {horizon.get('scenes_per_chapter')}"
            )
        expected_orders = list(range(1, len(scenes) + 1))
        if [scene.get("scene_order") for scene in scenes] != expected_orders:
            failures.append(f"chapter {chapter.get('chapter_number')} scene orders are not contiguous")
        for scene in scenes:
            if not str(scene.get("summary", "")).strip():
                failures.append(f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} needs a summary")
            if not str(scene.get("purpose", "")).strip():
                failures.append(f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} needs a purpose")
            if not isinstance(scene.get("cast_refs"), list) or not scene.get("cast_refs"):
                failures.append(f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} needs cast_refs")
            if not isinstance(scene.get("causal_from", []), list):
                failures.append(f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} causal_from must be an array")
            if scene.get("content_rating") not in RATING_RANK:
                failures.append(f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} has an unknown content_rating")
    if not isinstance(plan.get("operations"), list) or not plan.get("operations"):
        failures.append("operations must be a non-empty array")
    return not failures, failures, 1.0 if not failures else 0.0


def check_entity_refs(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    failures: list[str] = []
    declarations = plan.get("declarations", {})
    declared = set()
    duplicates = []
    for key in ("cast", "locations", "world_rules", "themes", "conflicts", "plot_lines", "arcs", "promises"):
        for ref in declarations.get(key, []):
            if ref in declared:
                duplicates.append(ref)
            declared.add(ref)
    for ref in duplicates:
        failures.append(f"duplicate declaration ref {ref}")
    allowed = set(declared) | set(brief.get("existing_refs", []))
    total = 0
    resolved = 0

    def check(ref: str | None, where: str) -> None:
        nonlocal total, resolved
        if not ref:
            return
        total += 1
        if ref in allowed:
            resolved += 1
        else:
            failures.append(f"{where} references unknown entity {ref}")

    for chapter in plan.get("chapters", []):
        check(chapter.get("pov_ref"), f"chapter {chapter.get('chapter_number')} pov")
        for scene in chapter.get("scenes", []):
            where = f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')}"
            for ref in scene.get("cast_refs", []):
                check(ref, where)
            check(scene.get("location_ref"), where)
    for event in plan.get("schedule", {}).get("promise_events", []):
        check(event.get("promise_ref"), "promise event")
    for event in plan.get("schedule", {}).get("pov_plan", []):
        check(event.get("pov_character_id"), "pov plan")
    metric = 1.0 if total == 0 else round(resolved / total, 4)
    return not failures, failures, metric


def check_causality(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    failures: list[str] = []
    scenes = _scene_refs(plan)
    scene_refs = {item["ref"] for item in scenes}
    connected = 0
    for index, item in enumerate(scenes):
        scene = item["scene"]
        if index == 0:
            connected += 1
            continue
        antecedents = scene.get("causal_from", [])
        if not antecedents:
            failures.append(f"{item['ref']} has no causal antecedent")
            continue
        if all(ref not in scene_refs for ref in antecedents):
            failures.append(f"{item['ref']} cites no resolvable causal antecedent")
            continue
        connected += 1
    metric = 1.0 if not scenes else round(connected / len(scenes), 4)
    return not failures and metric >= 0.95, failures, metric


def check_promises(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    failures: list[str] = []
    events = plan.get("schedule", {}).get("promise_events", [])
    by_promise: dict[str, list[dict]] = {}
    for event in events:
        by_promise.setdefault(event.get("promise_ref", ""), []).append(event)
    obligations = 0
    satisfied = 0
    horizon = plan.get("horizon", {})
    in_horizon = lambda placement: (
        placement.get("book_number") == horizon.get("book_number")
        and horizon.get("chapter_start", 0) <= placement.get("chapter_number", 0) <= horizon.get("chapter_end", 0)
    )
    for promise in brief.get("promises", []):
        obligations += 1
        promise_events = by_promise.get(promise["ref"], [])
        plants = [event for event in promise_events if event.get("action") == "plant"]
        payoffs = [event for event in promise_events if event.get("action") == "payoff"]
        defers = [event for event in promise_events if event.get("action") == "defer"]
        if not plants:
            failures.append(f"{promise['ref']} has no setup event")
            continue
        if promise.get("deferred_reason"):
            if not defers:
                failures.append(f"{promise['ref']} is deferred without a recorded reason")
                continue
            satisfied += 1
            continue
        if not payoffs:
            failures.append(f"{promise['ref']} has no payoff event")
            continue
        plant = plants[0]["placement"]
        payoff = payoffs[0]["placement"]
        if (payoff.get("chapter_number"), payoff.get("scene_order", 0)) <= (
            plant.get("chapter_number"),
            plant.get("scene_order", 0),
        ):
            failures.append(f"{promise['ref']} pays off before setup")
            continue
        if not in_horizon(payoff):
            failures.append(f"{promise['ref']} pays off outside the horizon without deferral")
            continue
        satisfied += 1
    metric = 1.0 if obligations == 0 else round(satisfied / obligations, 4)
    return not failures and metric == 1.0, failures, metric


def check_constraints(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    failures: list[str] = []
    spec = brief.get("constraint_spec", {})
    horizon = plan.get("horizon", {})
    expected = brief.get("horizon", {})
    if horizon != expected:
        failures.append("plan horizon does not match the requested horizon")
    chapters = plan.get("chapters", [])
    if len(chapters) != len(brief.get("chapters", [])):
        failures.append("plan chapter count does not match the brief")
    ceiling = spec.get("rating_ceiling", brief.get("rating_ceiling", "general"))
    allowed_pov = set(spec.get("allowed_pov_refs", [cast["ref"] for cast in brief.get("cast", [])]))
    used_cast: set[str] = set()
    used_locations: set[str] = set()
    forbidden = [term.lower() for term in spec.get("forbidden_terms", [])]
    for chapter in chapters:
        if chapter.get("pov_ref") not in allowed_pov:
            failures.append(f"chapter {chapter.get('chapter_number')} uses POV outside the allowed roster")
        for scene in chapter.get("scenes", []):
            if RATING_RANK.get(scene.get("content_rating"), 99) > RATING_RANK.get(ceiling, 0):
                failures.append(
                    f"chapter {chapter.get('chapter_number')} scene {scene.get('scene_order')} exceeds rating ceiling {ceiling}"
                )
            used_cast.update(scene.get("cast_refs", []))
            if scene.get("location_ref"):
                used_locations.add(scene["location_ref"])
            text = f"{scene.get('summary', '')} {scene.get('purpose', '')}".lower()
            for term in forbidden:
                if term in text:
                    failures.append(f"forbidden term {term!r} appears in chapter {chapter.get('chapter_number')}")
    for ref in spec.get("required_cast_refs", []):
        if ref not in used_cast:
            failures.append(f"required cast {ref} does not appear in any scene")
    for ref in spec.get("required_location_refs", []):
        if ref not in used_locations:
            failures.append(f"required location {ref} does not appear in any scene")
    total = 4 + len(spec.get("required_cast_refs", [])) + len(spec.get("required_location_refs", [])) + len(forbidden)
    return not failures, failures, 1.0 if not failures else 0.0


def _simulate_apply(plan: dict) -> dict:
    mapping = {}
    for index, item in enumerate(plan.get("operations", [])):
        output_ref = item.get("output_ref")
        operation = item.get("operation", {})
        kind = operation.get("operation")
        if not output_ref:
            raise ValueError(f"operation {index} has no output_ref")
        if output_ref in mapping:
            raise ValueError(f"duplicate output_ref {output_ref}")
        if kind not in KNOWN_OPERATIONS:
            raise ValueError(f"unknown operation {kind!r}")
        if not isinstance(operation.get("input"), dict):
            raise ValueError(f"operation {output_ref} has no input object")
        mapping[output_ref] = f"fixture_id:{hashlib.sha256(output_ref.encode()).hexdigest()[:16]}"
    return mapping


def check_idempotent_apply(plan: dict, brief: dict) -> tuple[bool, list[str], float]:
    try:
        first = _simulate_apply(plan)
        second = _simulate_apply(copy.deepcopy(plan))
    except ValueError as error:
        return False, [str(error)], 0.0
    if first != second:
        return False, ["second apply produced a different ID mapping"], 0.0
    return True, [], 1.0


GATES = {
    "schema_validity_rate": (check_schema, 1.0),
    "entity_ref_integrity": (check_entity_refs, 1.0),
    "causality_connectivity": (check_causality, 0.95),
    "promise_coverage": (check_promises, 1.0),
    "constraint_compliance": (check_constraints, 1.0),
    "idempotent_apply": (check_idempotent_apply, 1.0),
}


def evaluate_gates(plan: dict, brief: dict) -> dict:
    gates = {}
    for name, (fn, threshold) in GATES.items():
        passed, failures, metric = fn(plan, brief)
        gates[name] = {
            "passed": bool(passed),
            "metric": metric,
            "threshold": threshold,
            "failures": failures,
        }
    return {
        "brief_id": brief["id"],
        "passed": all(gate["passed"] for gate in gates.values()),
        "gates": gates,
    }


def layer1_summary(results: list[dict]) -> dict:
    if not results:
        return {"candidate_count": 0, "passed_count": 0, "all_passed": False, "gates": {}}
    gates = {}
    for name in GATES:
        values = [result["gates"][name]["metric"] for result in results]
        gates[name] = {
            "mean": round(statistics.mean(values), 4),
            "min": round(min(values), 4),
            "passed": all(result["gates"][name]["passed"] for result in results),
            "threshold": GATES[name][1],
        }
    return {
        "candidate_count": len(results),
        "passed_count": sum(1 for result in results if result["passed"]),
        "all_passed": all(result["passed"] for result in results),
        "gates": gates,
    }


# =============================================================================
# Negative fixtures
# =============================================================================

def mutate_plan(plan: dict, mutation: str) -> dict:
    mutated = copy.deepcopy(plan)
    if mutation == "schema_invalid":
        mutated.pop("contract", None)
    elif mutation == "unknown_ref":
        mutated["chapters"][0]["scenes"][0]["cast_refs"].append("cast:unknown-person")
    elif mutation == "disconnected":
        mutated["chapters"][1]["scenes"][0]["causal_from"] = []
    elif mutation == "missing_payoff":
        mutated["schedule"]["promise_events"] = [
            event
            for event in mutated["schedule"]["promise_events"]
            if event.get("action") not in ("payoff", "defer")
        ]
    elif mutation == "constraint_violation":
        mutated["chapters"][0]["scenes"][0]["content_rating"] = "explicit"
    elif mutation == "duplicate_apply":
        mutated["operations"].append(copy.deepcopy(mutated["operations"][0]))
    elif mutation == "unknown_operation":
        mutated["operations"][0]["operation"]["operation"] = "invent_entity"
    else:
        raise ValueError(f"unknown mutation: {mutation}")
    return mutated


NEGATIVE_CASES = {
    "schema_invalid": "schema_validity_rate",
    "unknown_ref": "entity_ref_integrity",
    "disconnected": "causality_connectivity",
    "missing_payoff": "promise_coverage",
    "constraint_violation": "constraint_compliance",
    "duplicate_apply": "idempotent_apply",
    "unknown_operation": "idempotent_apply",
}


# =============================================================================
# Layer 2: pairwise rubric judging
# =============================================================================

def neutral_plan(plan: dict) -> dict:
    view = copy.deepcopy(plan)
    view.pop("fixture_arm", None)
    return view


def build_judge_prompts(brief: dict, plan_a: dict, plan_b: dict) -> tuple[str, str]:
    system_prompt = (
        "You are a blinded, adversarial story-plan judge. You receive a brief and two "
        "candidate plans, A and B, in randomized order. Judge only the plans, never the "
        "labels or writing style. A high score requires concrete connected choices; generic "
        "beat labels earn no specificity credit. Return ONE JSON object with the exact shape:\n"
        "{\"dimensions\":{\"<dimension>\":{\"winner\":\"A\"|\"B\"|\"tie\","
        "\"score_a\":1-5,\"score_b\":1-5,\"evidence\":[\"chapter/scene/proposal refs\"]}},"
        "\"overall_winner\":\"A\"|\"B\"|\"tie\",\"rationale\":\"short\"}.\n"
        "Every dimension is required: " + ", ".join(RUBRIC_DIMENSIONS) + "."
    )
    user_prompt = (
        "BRIEF\n" + json.dumps(brief, ensure_ascii=False, sort_keys=True, indent=2) +
        "\n\nPLAN A\n" + json.dumps(neutral_plan(plan_a), ensure_ascii=False, sort_keys=True, indent=2) +
        "\n\nPLAN B\n" + json.dumps(neutral_plan(plan_b), ensure_ascii=False, sort_keys=True, indent=2) +
        "\n\nScore the dimensions above. Use chapter/scene refs as evidence."
    )
    return system_prompt, user_prompt


def validate_judge_response(payload: dict) -> dict:
    if not isinstance(payload, dict):
        raise ValueError("judge response must be an object")
    dimensions = payload.get("dimensions")
    if not isinstance(dimensions, dict):
        raise ValueError("judge response needs dimensions")
    missing = [dim for dim in RUBRIC_DIMENSIONS if dim not in dimensions]
    if missing:
        raise ValueError(f"judge response missing dimensions: {missing}")
    clean = {}
    for dim in RUBRIC_DIMENSIONS:
        value = dimensions[dim]
        if value.get("winner") not in ("A", "B", "tie"):
            raise ValueError(f"{dim}.winner must be A, B, or tie")
        for key in ("score_a", "score_b"):
            score = value.get(key)
            if not isinstance(score, int) or not 1 <= score <= 5:
                raise ValueError(f"{dim}.{key} must be an integer 1-5")
        evidence = value.get("evidence", [])
        if not isinstance(evidence, list) or not all(isinstance(item, str) for item in evidence):
            raise ValueError(f"{dim}.evidence must be a string array")
        clean[dim] = {
            "winner": value["winner"],
            "score_a": value["score_a"],
            "score_b": value["score_b"],
            "evidence": evidence,
        }
    if payload.get("overall_winner") not in ("A", "B", "tie"):
        raise ValueError("overall_winner must be A, B, or tie")
    rationale = payload.get("rationale", "")
    if not isinstance(rationale, str):
        raise ValueError("rationale must be a string")
    return {"dimensions": clean, "overall_winner": payload["overall_winner"], "rationale": rationale}


def _contract_text(plan: dict) -> str:
    contract = plan.get("contract", {})
    return " ".join([str(contract.get("promise", ""))] + list(contract.get("boundaries", [])))


def plan_quality_metrics(brief: dict, plan: dict) -> dict:
    scenes = _scene_refs(plan)
    scene_count = max(1, len(scenes))
    specific = 0.0
    agency = 0.0
    reference = " ".join(
        [brief.get("premise", ""), brief.get("hook", ""), brief.get("stakes", "")]
        + [chapter.get("turn", "") for chapter in brief.get("chapters", [])]
        + [
            scene
            for chapter in brief.get("chapters", [])
            for scene in chapter.get("scenes", [])
        ]
    )
    for item in scenes:
        scene = item["scene"]
        text = f"{scene.get('summary', '')} {scene.get('purpose', '')}"
        specific += overlap_ratio(text, reference)
        if any(
            verb in str(scene.get("purpose", "")).lower()
            for verb in ("choose", "decide", "risk", "refuse", "protect", "trade", "pay", "confront")
        ) and scene.get("cast_refs"):
            agency += 1.0
    metrics = {
        "specificity": round(specific / scene_count, 4),
        "agency": round(agency / scene_count, 4),
        "contract_fidelity": round(
            (overlap_ratio(_contract_text(plan), brief.get("premise", "")) +
             overlap_ratio(_contract_text(plan), " ".join(brief.get("boundaries", [])))) / 2.0,
            4,
        ),
        "assumptions": len(plan.get("assumptions", [])),
        "questions": len(plan.get("author_questions", [])),
    }
    evaluated = evaluate_gates(plan, brief)
    metrics["causality"] = evaluated["gates"]["causality_connectivity"]["metric"]
    metrics["promises"] = evaluated["gates"]["promise_coverage"]["metric"]
    metrics["constraints"] = evaluated["gates"]["constraint_compliance"]["metric"]
    metrics["refs"] = evaluated["gates"]["entity_ref_integrity"]["metric"]
    chapter_povs = [chapter.get("pov_ref") for chapter in plan.get("chapters", [])]
    policy = brief.get("pov_policy", {})
    if policy.get("single_pov"):
        metrics["pov"] = 1.0 if set(chapter_povs) == {policy["single_pov"]} else 0.0
    else:
        roster = set(policy.get("allowed_roster", []))
        metrics["pov"] = 1.0 if all(pov in roster for pov in chapter_povs) else 0.0
    targets = plan.get("schedule", {}).get("intensity_targets", [])
    metrics["pacing"] = 1.0 if len(targets) == len(plan.get("chapters", [])) else 0.5
    return metrics


def _score_metric(value: float, profile: str) -> int:
    weights = {
        "structural": {"causality": 1.0, "promises": 1.0, "constraints": 1.0, "refs": 0.8, "specificity": 0.2},
        "thematic": {"specificity": 1.0, "agency": 1.0, "contract_fidelity": 1.0, "pov": 0.6, "pacing": 0.6},
    }[profile]
    total = sum(value.get(key, 0.0) * weight for key, weight in weights.items())
    max_total = sum(weights.values())
    normalized = total / max_total if max_total else 0.0
    return max(1, min(5, int(round(1 + normalized * 4))))


def run_fake_plan_judge(
    judge_cfg: dict,
    brief: dict,
    plan_a: dict,
    plan_b: dict,
) -> dict:
    profile = judge_cfg.get("profile", "structural")
    metrics_a = plan_quality_metrics(brief, plan_a)
    metrics_b = plan_quality_metrics(brief, plan_b)
    dimensions = {}
    for dim in RUBRIC_DIMENSIONS:
        if dim == "contract_fidelity":
            score_a = _score_metric({"contract_fidelity": metrics_a["contract_fidelity"]}, profile)
            score_b = _score_metric({"contract_fidelity": metrics_b["contract_fidelity"]}, profile)
        elif dim == "causal_progression":
            score_a = _score_metric({"causality": metrics_a["causality"]}, "structural")
            score_b = _score_metric({"causality": metrics_b["causality"]}, "structural")
        elif dim == "character_agency":
            score_a = _score_metric({"agency": metrics_a["agency"]}, "thematic")
            score_b = _score_metric({"agency": metrics_b["agency"]}, "thematic")
        elif dim == "arc_credibility":
            score_a = _score_metric({"specificity": metrics_a["specificity"], "agency": metrics_a["agency"]}, "thematic")
            score_b = _score_metric({"specificity": metrics_b["specificity"], "agency": metrics_b["agency"]}, "thematic")
        elif dim == "promise_setup_progress_payoff":
            score_a = _score_metric({"promises": metrics_a["promises"]}, "structural")
            score_b = _score_metric({"promises": metrics_b["promises"]}, "structural")
        elif dim == "world_feasibility":
            score_a = _score_metric({"refs": metrics_a["refs"], "constraints": metrics_a["constraints"]}, "structural")
            score_b = _score_metric({"refs": metrics_b["refs"], "constraints": metrics_b["constraints"]}, "structural")
        elif dim == "pov_allocation":
            score_a = _score_metric({"pov": metrics_a["pov"]}, "thematic")
            score_b = _score_metric({"pov": metrics_b["pov"]}, "thematic")
        elif dim == "pacing_fit":
            score_a = _score_metric({"pacing": metrics_a["pacing"]}, "thematic")
            score_b = _score_metric({"pacing": metrics_b["pacing"]}, "thematic")
        elif dim == "scene_specificity_draftability":
            score_a = _score_metric({"specificity": metrics_a["specificity"]}, "thematic")
            score_b = _score_metric({"specificity": metrics_b["specificity"]}, "thematic")
        else:
            score_a = 3 if metrics_a["questions"] or metrics_a["assumptions"] else 2
            score_b = 3 if metrics_b["questions"] or metrics_b["assumptions"] else 2
        if score_a > score_b:
            winner = "A"
        elif score_b > score_a:
            winner = "B"
        else:
            winner = "tie"
        dimensions[dim] = {
            "winner": winner,
            "score_a": score_a,
            "score_b": score_b,
            "evidence": [f"chapter:{brief['chapters'][0]['chapter_number']}:scene:1"],
        }
    wins_a = sum(1 for value in dimensions.values() if value["winner"] == "A")
    wins_b = sum(1 for value in dimensions.values() if value["winner"] == "B")
    if wins_a > wins_b:
        overall = "A"
    elif wins_b > wins_a:
        overall = "B"
    else:
        overall = "tie"
    return {
        "dimensions": dimensions,
        "overall_winner": overall,
        "rationale": f"fixture {judge_cfg['id']} scored A={wins_a}, B={wins_b}",
    }


def evaluate_single_judge_pass(
    judge_cfg: dict,
    brief: dict,
    plan_a: dict,
    plan_b: dict,
) -> dict:
    if str(judge_cfg.get("provider", "fake")).lower() == "fake":
        return run_fake_plan_judge(judge_cfg, brief, plan_a, plan_b)
    system_prompt, user_prompt = build_judge_prompts(brief, plan_a, plan_b)
    response = prose_judge.call_model_provider(judge_cfg, system_prompt, user_prompt)
    payload = prose_judge.extract_json_object(response["text"])
    validated = validate_judge_response(payload)
    validated["receipt"] = {
        "judge_id": judge_cfg.get("id"),
        "model": response.get("model"),
        "input_tokens": response.get("input_tokens"),
        "output_tokens": response.get("output_tokens"),
        "elapsed_seconds": response.get("elapsed_seconds"),
    }
    return validated


def judge_pair_double(
    judge_cfg: dict,
    brief: dict,
    pair: dict,
    seed: int,
) -> dict:
    rng = random.Random(f"{seed}:{brief['id']}:{judge_cfg['id']}")
    initial_order = ("baseline", "candidate") if rng.random() < 0.5 else ("candidate", "baseline")
    swapped_order = (initial_order[1], initial_order[0])
    pass_1 = evaluate_single_judge_pass(judge_cfg, brief, pair[initial_order[0]], pair[initial_order[1]])
    pass_2 = evaluate_single_judge_pass(judge_cfg, brief, pair[swapped_order[0]], pair[swapped_order[1]])

    def deblind(payload: dict, order: tuple[str, str]) -> str:
        winner = payload["overall_winner"]
        return winner if winner == "tie" else order[0] if winner == "A" else order[1]

    first = deblind(pass_1, initial_order)
    second = deblind(pass_2, swapped_order)
    if first == second:
        verdict = first
        order_consistent = True
        position_bias = "none"
    elif first == "tie":
        verdict = second
        order_consistent = False
        position_bias = "none"
    elif second == "tie":
        verdict = first
        order_consistent = False
        position_bias = "none"
    else:
        verdict = "tie"
        order_consistent = False
        if pass_1["overall_winner"] == pass_2["overall_winner"] == "A":
            position_bias = "first_position"
        elif pass_1["overall_winner"] == pass_2["overall_winner"] == "B":
            position_bias = "second_position"
        else:
            position_bias = "verdict_flip"
    combined = {}
    for dim in RUBRIC_DIMENSIONS:
        r1 = pass_1["dimensions"][dim]
        r2 = pass_2["dimensions"][dim]
        score_baseline = round((r1["score_a"] + r2["score_b"]) / 2.0, 2)
        score_candidate = round((r1["score_b"] + r2["score_a"]) / 2.0, 2)
        wins = []
        for payload, order in ((pass_1, initial_order), (pass_2, swapped_order)):
            winner = payload["dimensions"][dim]["winner"]
            if winner == "tie":
                wins.append("tie")
            else:
                wins.append("baseline" if order[0 if winner == "A" else 1] == "baseline" else "candidate")
        if wins[0] == wins[1]:
            winner = wins[0]
        elif score_candidate > score_baseline:
            winner = "candidate"
        elif score_baseline > score_candidate:
            winner = "baseline"
        else:
            winner = "tie"
        combined[dim] = {
            "winner": winner,
            "baseline_score": score_baseline,
            "candidate_score": score_candidate,
            "evidence": r1.get("evidence", []) + r2.get("evidence", []),
        }
    return {
        "brief_id": brief["id"],
        "judge_id": judge_cfg["id"],
        "judge_family": judge_cfg.get("family", "unknown"),
        "verdict": verdict,
        "order_consistent": order_consistent,
        "position_bias_kind": position_bias,
        "pass_1": pass_1,
        "pass_2": pass_2,
        "rubric": combined,
    }


def _majority(labels: list[str]) -> str:
    counted = Counter(label for label in labels if label in VERDICT_LABELS)
    if not counted:
        return "tie"
    top = counted.most_common()
    if len(top) > 1 and top[0][1] == top[1][1] and top[0][1] > len(labels) / 2:
        return "tie"
    if top[0][1] > len(labels) / 2:
        return top[0][0]
    return "tie"


def bootstrap_rate(values: list[float], seed: int = 20261107, n_resamples: int = 4000) -> dict:
    return prose_judge.bootstrap_ci(values, seed=seed, n_resamples=n_resamples)


def run_pairwise_evaluation(
    briefs: list[dict],
    plans_by_brief: dict[str, dict[str, dict]],
    judges: list[dict],
    seed: int = 20261107,
) -> dict:
    records = []
    per_brief = []
    for brief in briefs:
        pair = plans_by_brief[brief["id"]]
        judge_records = [judge_pair_double(judge, brief, pair, seed) for judge in judges]
        consensus = _majority([record["verdict"] for record in judge_records])
        contract = _majority([
            record["rubric"]["contract_fidelity"]["winner"] for record in judge_records
        ])
        per_brief.append(
            {
                "brief_id": brief["id"],
                "kind": brief["kind"],
                "verdict": consensus,
                "contract_fidelity_winner": contract,
                "judge_verdicts": [record["verdict"] for record in judge_records],
            }
        )
        records.extend(judge_records)
    wins = [item for item in per_brief if item["verdict"] == "candidate"]
    losses = [item for item in per_brief if item["verdict"] == "baseline"]
    ties = [item for item in per_brief if item["verdict"] == "tie"]
    decisive = len(wins) + len(losses)
    win_rate = round(len(wins) / decisive, 4) if decisive else None
    rates = [1.0 if item["verdict"] == "candidate" else 0.0 for item in wins + losses]
    ci = bootstrap_rate(rates, seed=seed) if rates else {"estimate": None, "ci95_low": None, "ci95_high": None}
    position_bias = Counter(record["position_bias_kind"] for record in records if record["position_bias_kind"] != "none")
    consistency = round(sum(1 for record in records if record["order_consistent"]) / len(records), 4) if records else 0.0
    contract_regressions = [item for item in per_brief if item["contract_fidelity_winner"] == "baseline"]
    gate_passed = bool(
        win_rate is not None
        and win_rate >= RELEASE_GATE["candidate_win_rate_excluding_ties"]
        and ci.get("ci95_low") is not None
        and ci["ci95_low"] > RELEASE_GATE["candidate_win_rate_ci_low"]
        and len(contract_regressions) <= RELEASE_GATE["max_contract_fidelity_regressions"]
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "fixture_mode": all(str(judge.get("provider", "fake")).lower() == "fake" for judge in judges),
        "brief_count": len(per_brief),
        "candidate_wins": len(wins),
        "baseline_wins": len(losses),
        "ties": len(ties),
        "candidate_win_rate_excluding_ties": win_rate,
        "bootstrap_ci": ci,
        "contract_fidelity_regressions": contract_regressions,
        "position_bias": dict(position_bias),
        "order_consistency_rate": consistency,
        "judge_family_count": len({judge.get("family") for judge in judges}),
        "release_gate": {
            "passed": gate_passed,
            "thresholds": RELEASE_GATE,
        },
        "rows": per_brief,
        "judgments": records,
    }


# =============================================================================
# Calibration
# =============================================================================

def load_calibration(path: Path | str = CALIBRATION_PATH) -> dict:
    return read_json(path)


def calibration_report(pairwise: dict, calibration: dict) -> dict:
    consensus = {row["brief_id"]: row["verdict"] for row in pairwise["rows"]}
    pairs = calibration.get("pairs", [])
    labels_a = []
    labels_b = []
    mismatches = []
    for pair in pairs:
        brief_id = pair["brief_id"]
        human = pair.get("human_preference")
        judged = consensus.get(brief_id)
        labels_a.append(judged if judged in VERDICT_LABELS else "tie")
        labels_b.append(human if human in VERDICT_LABELS else "tie")
        if judged != human:
            mismatches.append({"brief_id": brief_id, "judge": judged, "human": human})
    matched = sum(1 for a, b in zip(labels_a, labels_b) if a == b)
    agreement = round(matched / len(pairs), 4) if pairs else None
    kappa = prose_judge.cohens_kappa(labels_a, labels_b, VERDICT_LABELS) if pairs else None
    return {
        "schema_version": SCHEMA_VERSION,
        "synthetic_fixture": bool(calibration.get("synthetic_fixture")),
        "pair_count": len(pairs),
        "recommended_pairs": calibration.get("recommended_pairs", 20),
        "agreement": agreement,
        "cohens_kappa": kappa,
        "mismatches": mismatches,
        "note": calibration.get("description", ""),
    }


# =============================================================================
# Reporting
# =============================================================================

def negative_boundary_summary(
    briefs: list[dict], plans_by_brief: dict[str, dict[str, dict]]
) -> dict:
    rows = []
    for brief in briefs:
        plan = plans_by_brief[brief["id"]]["candidate"]
        for mutation, gate in NEGATIVE_CASES.items():
            result = evaluate_gates(mutate_plan(plan, mutation), brief)
            rows.append(
                {
                    "brief_id": brief["id"],
                    "mutation": mutation,
                    "expected_gate": gate,
                    "failed_as_expected": not result["gates"][gate]["passed"],
                }
            )
    return {
        "total": len(rows),
        "failed_as_expected": sum(1 for row in rows if row["failed_as_expected"]),
        "all_passed": all(row["failed_as_expected"] for row in rows),
        "rows": rows,
    }


def gate_fixture_suite(briefs: list[dict], plans_by_brief: dict[str, dict[str, dict]]) -> dict:
    results = [
        evaluate_gates(plans_by_brief[brief["id"]]["candidate"], brief) for brief in briefs
    ]
    negative = negative_boundary_summary(briefs, plans_by_brief)
    return {
        "schema_version": SCHEMA_VERSION,
        "brief_count": len(briefs),
        "summary": {
            **layer1_summary(results),
            "negative_boundary_checks": {
                "total": negative["total"],
                "failed_as_expected": negative["failed_as_expected"],
                "all_passed": negative["all_passed"],
            },
        },
        "results": results,
        "negative_cases": negative,
    }


def negative_fixture_report(brief: dict, plan: dict) -> dict:
    rows = {}
    for mutation, gate in NEGATIVE_CASES.items():
        result = evaluate_gates(mutate_plan(plan, mutation), brief)
        rows[mutation] = {
            "expected_gate": gate,
            "failed_as_expected": not result["gates"][gate]["passed"],
            "all_gates_failed": [name for name, value in result["gates"].items() if not value["passed"]],
        }
    return rows


def fixture_plans(briefs: list[dict]) -> dict[str, dict[str, dict]]:
    return {brief["id"]: plan_pair(brief) for brief in briefs}


def validate_brief_suite(briefs: list[dict]) -> list[str]:
    problems: list[str] = []
    if len(briefs) != 30:
        problems.append(f"expected 30 briefs, got {len(briefs)}")
    kinds = Counter(brief.get("kind") for brief in briefs)
    for kind, expected in (("greenfield", 10), ("continuation", 10), ("replan", 10)):
        if kinds.get(kind) != expected:
            problems.append(f"expected {expected} {kind} briefs, got {kinds.get(kind)}")
    ids = [brief.get("id") for brief in briefs]
    if len(ids) != len(set(ids)):
        problems.append("brief ids must be unique")
    required = {
        "id", "kind", "genre", "mode", "pov_mode", "rating_ceiling", "horizon",
        "premise", "contract_promise", "cast", "locations", "promises", "chapters",
        "constraint_spec",
    }
    for brief in briefs:
        missing = required - set(brief)
        if missing:
            problems.append(f"{brief.get('id')} missing {sorted(missing)}")
        if len(brief.get("chapters", [])) != len(range(
            brief.get("horizon", {}).get("chapter_start", 1),
            brief.get("horizon", {}).get("chapter_end", 0) + 1,
        )):
            problems.append(f"{brief.get('id')} chapter list does not cover its horizon")
        for chapter in brief.get("chapters", []):
            if len(chapter.get("scenes", [])) != brief.get("horizon", {}).get("scenes_per_chapter"):
                problems.append(f"{brief.get('id')} chapter {chapter.get('chapter_number')} has the wrong scene count")
    if not any(brief.get("pov_mode") == "rotating" for brief in briefs):
        problems.append("suite needs rotating-POV briefs")
    if not any(brief.get("mode") == "serial" for brief in briefs):
        problems.append("suite needs serial briefs")
    if not any(brief.get("kind") == "replan" and brief.get("conflicting_input") for brief in briefs):
        problems.append("suite needs a replan brief with a deliberately conflicting input")
    return problems


# =============================================================================
# Self-test
# =============================================================================

def self_test() -> None:
    briefs = load_briefs()
    problems = validate_brief_suite(briefs)
    assert not problems, problems
    by_id = brief_by_id(briefs)
    plans = fixture_plans(briefs)

    # Deterministic fixtures.
    assert digest(fixture_plan(by_id["PJG01"])) == digest(fixture_plan(by_id["PJG01"]))
    assert fixture_plan(by_id["PJG01"], "candidate")["fixture_arm"] == "candidate"

    # All valid candidate fixtures pass Layer 1.
    gates = gate_fixture_suite(briefs, plans)
    assert gates["summary"]["all_passed"], [
        result for result in gates["results"] if not result["passed"]
    ]
    for name, gate in gates["summary"]["gates"].items():
        assert gate["passed"], (name, gate)
    assert gates["summary"]["negative_boundary_checks"]["all_passed"], gates["negative_cases"]

    # Invalid-input cases fail at the right boundary.
    negative = negative_fixture_report(by_id["PJG01"], plans["PJG01"]["candidate"])
    for mutation, row in negative.items():
        assert row["failed_as_expected"], (mutation, row)

    # Judge prompt and response validation.
    system_prompt, user_prompt = build_judge_prompts(
        by_id["PJG01"], plans["PJG01"]["baseline"], plans["PJG01"]["candidate"]
    )
    for dim in RUBRIC_DIMENSIONS:
        assert dim in system_prompt
    assert "PLAN A" in user_prompt and "PLAN B" in user_prompt
    fake = run_fake_plan_judge(
        OFFLINE_JUDGES[0],
        by_id["PJG01"],
        plans["PJG01"]["baseline"],
        plans["PJG01"]["candidate"],
    )
    validate_judge_response(fake)
    assert fake["overall_winner"] in ("A", "B", "tie")

    # Swapped-order judging resolves candidate wins and records order consistency.
    record = judge_pair_double(
        OFFLINE_JUDGES[0], by_id["PJG01"], plans["PJG01"], 20261107
    )
    assert record["verdict"] in VERDICT_LABELS
    assert record["order_consistent"] is True, record

    # A deliberately position-biased judge flips and is recorded as bias.
    biased = {"id": "bias", "family": "bias", "provider": "fake", "profile": "thematic"}
    original = evaluate_single_judge_pass

    def biased_pass(judge_cfg, brief, plan_a, plan_b):
        del judge_cfg, brief, plan_a, plan_b
        return {
            "dimensions": {
                dim: {"winner": "A", "score_a": 5, "score_b": 1, "evidence": []}
                for dim in RUBRIC_DIMENSIONS
            },
            "overall_winner": "A",
            "rationale": "always first",
        }

    try:
        globals()["evaluate_single_judge_pass"] = biased_pass
        biased_record = judge_pair_double(biased, by_id["PJG01"], plans["PJG01"], 3)
    finally:
        globals()["evaluate_single_judge_pass"] = original
    assert biased_record["verdict"] == "tie"
    assert biased_record["position_bias_kind"] in ("first_position", "second_position", "verdict_flip")

    # Pairwise release gate on deterministic fixtures.
    pairwise = run_pairwise_evaluation(briefs, plans, OFFLINE_JUDGES)
    assert pairwise["candidate_wins"] >= 18, pairwise
    assert pairwise["release_gate"]["passed"], pairwise["release_gate"]
    assert pairwise["bootstrap_ci"]["ci95_low"] is not None

    calibration = load_calibration()
    assert calibration.get("recommended_pairs") == 20
    assert len(calibration.get("pairs", [])) == 20
    report = calibration_report(pairwise, calibration)
    assert report["pair_count"] == 20
    assert report["agreement"] is not None
    assert report["synthetic_fixture"] is True

    # Secret redaction boundary is inherited from the prose judge helper.
    os.environ["SPINDLE_PLAN_JUDGE_TEST_SECRET"] = "plan-secret-value"
    assert "plan-secret-value" not in prose_judge.redact_secrets("plan-secret-value")
    del os.environ["SPINDLE_PLAN_JUDGE_TEST_SECRET"]

    print("plan_judge self-test: ok")
    print(
        f"  briefs={len(briefs)} layer1_pass={gates['summary']['passed_count']}/30 "
        f"candidate_wins={pairwise['candidate_wins']} ties={pairwise['ties']} "
        f"win_rate={pairwise['candidate_win_rate_excluding_ties']} "
        f"ci_low={pairwise['bootstrap_ci']['ci95_low']} "
        f"calibration_agreement={report['agreement']}"
    )


# =============================================================================
# CLI
# =============================================================================

def resolve_plans(path: str | None, briefs: list[dict]) -> dict[str, dict[str, dict]]:
    if not path:
        return fixture_plans(briefs)
    data = read_json(path)
    plans = {}
    for brief in briefs:
        if brief["id"] not in data:
            raise ValueError(f"plans file missing brief {brief['id']}")
        plans[brief["id"]] = data[brief["id"]]
    return plans


def command_gate(args) -> int:
    briefs = load_briefs(args.briefs)
    plans = resolve_plans(args.plans, briefs)
    report = gate_fixture_suite(briefs, plans)
    if args.json_out:
        write_json(args.json_out, report)
    print(json.dumps(report["summary"], indent=2))
    return 0 if report["summary"]["all_passed"] and report["negative_cases"]["all_passed"] else 1


def command_judge(args) -> int:
    briefs = load_briefs(args.briefs)
    plans = resolve_plans(args.plans, briefs)
    judges = load_judges(args.judges)
    if args.live:
        data = read_json(args.judges)
        judges = [judge for judge in data.get("live_judges", []) if judge.get("provider") != "fake"]
        if not judges:
            raise SystemExit("no live judges configured")
    report = run_pairwise_evaluation(briefs, plans, judges, seed=args.seed)
    if args.json_out:
        write_json(args.json_out, report)
    summary = {key: value for key, value in report.items() if key not in ("rows", "judgments")}
    print(json.dumps(summary, indent=2))
    return 0 if report["release_gate"]["passed"] else 1


def command_calibrate(args) -> int:
    calibration = load_calibration(args.calibration)
    if args.report:
        pairwise = read_json(args.report)
    else:
        briefs = load_briefs(args.briefs)
        plans = resolve_plans(args.plans, briefs)
        judges = load_judges(args.judges)
        pairwise = run_pairwise_evaluation(briefs, plans, judges, seed=args.seed)
    report = calibration_report(pairwise, calibration)
    if args.json_out:
        write_json(args.json_out, report)
    print(json.dumps(report, indent=2))
    return 0 if report["agreement"] is not None and report["agreement"] >= 0.70 else 1


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    self_test_parser = sub.add_parser("self-test", help="Run offline unit tests")
    self_test_parser.set_defaults(func=lambda args: (self_test(), 0)[1])

    gate_parser = sub.add_parser("gate", help="Run Layer 1 hard gates")
    gate_parser.add_argument("--briefs", default=str(BRIEFS_PATH))
    gate_parser.add_argument("--plans")
    gate_parser.add_argument("--json-out")
    gate_parser.set_defaults(func=command_gate)

    judge_parser = sub.add_parser("judge", help="Run Layer 2 pairwise judging")
    judge_parser.add_argument("--briefs", default=str(BRIEFS_PATH))
    judge_parser.add_argument("--plans")
    judge_parser.add_argument("--judges", default=str(JUDGES_PATH))
    judge_parser.add_argument("--live", action="store_true")
    judge_parser.add_argument("--seed", type=int, default=20261107)
    judge_parser.add_argument("--json-out")
    judge_parser.set_defaults(func=command_judge)

    calibrate_parser = sub.add_parser("calibrate", help="Compare judge consensus with calibration labels")
    calibrate_parser.add_argument("--briefs", default=str(BRIEFS_PATH))
    calibrate_parser.add_argument("--plans")
    calibrate_parser.add_argument("--judges", default=str(JUDGES_PATH))
    calibrate_parser.add_argument("--calibration", default=str(CALIBRATION_PATH))
    calibrate_parser.add_argument("--report")
    calibrate_parser.add_argument("--seed", type=int, default=20261107)
    calibrate_parser.add_argument("--json-out")
    calibrate_parser.set_defaults(func=command_calibrate)

    report_parser = sub.add_parser("report", help="Run gate, judge and calibration together")
    report_parser.add_argument("--briefs", default=str(BRIEFS_PATH))
    report_parser.add_argument("--plans")
    report_parser.add_argument("--judges", default=str(JUDGES_PATH))
    report_parser.add_argument("--calibration", default=str(CALIBRATION_PATH))
    report_parser.add_argument("--seed", type=int, default=20261107)
    report_parser.add_argument("--json-out")
    report_parser.set_defaults(func=None)

    args = parser.parse_args()
    if args.command == "report":
        briefs = load_briefs(args.briefs)
        plans = resolve_plans(args.plans, briefs)
        judges = load_judges(args.judges)
        gate = gate_fixture_suite(briefs, plans)
        pairwise = run_pairwise_evaluation(briefs, plans, judges, seed=args.seed)
        calibration = calibration_report(pairwise, load_calibration(args.calibration))
        combined = {
            "schema_version": SCHEMA_VERSION,
            "fixture_mode": pairwise["fixture_mode"],
            "layer1": gate["summary"],
            "layer2": {key: value for key, value in pairwise.items() if key not in ("rows", "judgments")},
            "calibration": calibration,
            "release_ready": bool(
                gate["summary"]["all_passed"]
                and gate["negative_cases"]["all_passed"]
                and pairwise["release_gate"]["passed"]
            ),
        }
        if args.json_out:
            write_json(args.json_out, combined)
        print(json.dumps(combined, indent=2))
        return
    raise SystemExit(args.func(args))


if __name__ == "__main__":
    main()

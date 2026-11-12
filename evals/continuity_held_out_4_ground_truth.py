"""Post-freeze scoring errata for the blind held-out-4 corpus.

The corpus module and its manifest hash stay frozen. This overlay documents
ground-truth exclusions found after the freeze and applies them at scoring
time, so the original corpus bytes and hashes are preserved.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Iterable

ERRATA_PATH = Path(__file__).with_name("continuity_held_out_4_errata.json")


def load_errata() -> dict[str, Any]:
    return json.loads(ERRATA_PATH.read_text(encoding="utf-8"))


def excluded_plant_ids() -> set[str]:
    return {
        str(entry["id"]) for entry in load_errata().get("excluded_plants", [])
    }


def filter_plants(items: Iterable[Any]) -> list[Any]:
    """Drop excluded plants from a planted-item sequence before scoring."""
    excluded = excluded_plant_ids()
    return [
        item
        for item in items
        if getattr(item, "contradiction_id", None) not in excluded
    ]


def _recount_manuscript(manuscript: dict[str, Any], excluded: set[str]) -> None:
    rows = [
        row
        for row in manuscript.get("contradictions", [])
        if row.get("contradiction_id") not in excluded
    ]
    manuscript["contradictions"] = rows
    manuscript["planted_total"] = len(rows)
    manuscript["detected_total"] = sum(1 for row in rows if row.get("detected"))
    manuscript["recall"] = (
        round(manuscript["detected_total"] / manuscript["planted_total"], 4)
        if manuscript["planted_total"]
        else 0.0
    )
    per_type = {
        check_type: {"planted": 0, "detected": 0}
        for check_type in manuscript.get("per_type", {})
    }
    for row in rows:
        check_type = row.get("contradiction_type", "unknown")
        slot = per_type.setdefault(check_type, {"planted": 0, "detected": 0})
        slot["planted"] += 1
        if row.get("detected"):
            slot["detected"] += 1
    for check_type, slot in per_type.items():
        planted = slot["planted"]
        detected = slot["detected"]
        slot["recall"] = round(detected / planted, 4) if planted else 0.0
    manuscript["per_type"] = per_type


def apply_errata(report: dict[str, Any]) -> dict[str, Any]:
    """Recompute a held-out-4 report with the frozen-corpus exclusions applied."""
    excluded = excluded_plant_ids()
    if not excluded:
        return report
    manuscripts = report.get("manuscripts", [])
    for manuscript in manuscripts:
        _recount_manuscript(manuscript, excluded)

    per_type: dict[str, dict[str, Any]] = {}
    total_planted = 0
    total_detected = 0
    for manuscript in manuscripts:
        total_planted += manuscript["planted_total"]
        total_detected += manuscript["detected_total"]
        for check_type, slot in manuscript["per_type"].items():
            aggregate = per_type.setdefault(
                check_type, {"planted": 0, "detected": 0}
            )
            aggregate["planted"] += slot["planted"]
            aggregate["detected"] += slot["detected"]
    for slot in per_type.values():
        planted = slot["planted"]
        detected = slot["detected"]
        slot["recall"] = round(detected / planted, 4) if planted else 0.0

    summary = report.setdefault("summary", {})
    summary["total_planted"] = total_planted
    summary["total_detected"] = total_detected
    summary["overall_recall"] = (
        round(total_detected / total_planted, 4) if total_planted else 0.0
    )
    summary["per_type_recall"] = per_type
    report["errata"] = {
        "excluded_plant_ids": sorted(excluded),
        "scored_plants": total_planted,
        "corpus_manifest_sha256": load_errata().get("corpus_manifest_sha256"),
    }
    return report

#!/usr/bin/env python3
"""Generate privacy-safe candidate snapshots for the public continuity suites.

Seeds each suite's control and planted manuscripts through the public MCP
surface, calls `prepare_semantic_continuity`, and records only deterministic
selection metadata: scene ordinal, candidate counts, families, check types,
canon table names, evidence origins, and omitted-by-family counts. Prose,
record ids, canon ids, and evidence quotes are deliberately excluded so the
snapshot is stable and safe to review.

This is a P2 audit tool, not an evaluator: it makes no model calls.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import tempfile
from typing import Any

from continuity_recall import McpClient, seed_ashfall_manuscript, seed_veridian_manuscript
from continuity_held_out import MANUSCRIPTS as HELD_OUT_MANUSCRIPTS, seed_held_out_manuscript
from continuity_held_out_2 import MANUSCRIPTS as HELD_OUT_2_MANUSCRIPTS, _build_manuscript
from continuity_held_out_3 import MANUSCRIPTS as HELD_OUT_3_MANUSCRIPTS, seed_manuscript


def _scene_snapshot(index: int, request: dict[str, Any]) -> dict[str, Any]:
    candidates = request["candidate_set"]["candidates"]
    by_family: dict[str, int] = {}
    by_check_type: dict[str, int] = {}
    canon_tables: set[str] = set()
    origins: set[str] = set()
    for candidate in candidates:
        by_family[candidate["family"]] = by_family.get(candidate["family"], 0) + 1
        by_check_type[candidate["check_type"]] = by_check_type.get(candidate["check_type"], 0) + 1
        canon_tables.update(reference["table"] for reference in candidate["canon_refs"])
        origins.update(hint["origin"] for hint in candidate["evidence_hints"])
    return {
        "scene_index": index,
        "candidate_count": len(candidates),
        "by_family": dict(sorted(by_family.items())),
        "by_check_type": dict(sorted(by_check_type.items())),
        "canon_tables": sorted(canon_tables),
        "evidence_origins": sorted(origins),
    }


def _prepare(client: McpClient, project_id: str) -> dict[str, Any]:
    return client.tool(
        "prepare_semantic_continuity",
        {"project_id": project_id, "scope": {"scope_type": "full"}, "subjects": []},
    )


def _project_snapshot(client: McpClient, name: str, project_id: str) -> dict[str, Any]:
    output = _prepare(client, project_id)
    scenes = [_scene_snapshot(index, request) for index, request in enumerate(output["requests"])]
    return {
        "name": name,
        "scene_count": output["coverage"]["scene_count"],
        "candidate_scene_count": output["coverage"]["candidate_scene_count"],
        "candidate_count": output["coverage"]["candidate_count"],
        "omitted_by_family": dict(sorted(output["coverage"].get("omitted_by_family", {}).items())),
        "scenes": scenes,
    }


def _manifest(name: str, variant: str) -> dict[str, Any]:
    return {"suite": name, "variant": variant, "manuscripts": []}


def _baseline(client: McpClient, variant: str) -> dict[str, Any]:
    planted = variant == "planted"
    manifest = _manifest("baseline", variant)
    for name, seeder in [
        ("ashfall_expedition", seed_ashfall_manuscript),
        ("veridian_station", seed_veridian_manuscript),
    ]:
        project_id, _ = seeder(client, planted)
        manifest["manuscripts"].append(_project_snapshot(client, name, project_id))
    return manifest


def _held_out(client: McpClient, variant: str) -> dict[str, Any]:
    planted = variant == "planted"
    manifest = _manifest("held-out-72", variant)
    for spec in HELD_OUT_MANUSCRIPTS:
        project_id, _ = seed_held_out_manuscript(client, spec, planted)
        manifest["manuscripts"].append(_project_snapshot(client, spec["id"], project_id))
    return manifest


def _held_out_2(client: McpClient, variant: str) -> dict[str, Any]:
    planted = variant == "planted"
    manifest = _manifest("held-out-2", variant)
    for spec in HELD_OUT_2_MANUSCRIPTS:
        result = _build_manuscript(client, spec, planted=planted)
        manifest["manuscripts"].append(_project_snapshot(client, spec["name"], result["project_id"]))
    return manifest


def _held_out_3(client: McpClient, variant: str) -> dict[str, Any]:
    planted = variant == "planted"
    manifest = _manifest("held-out-3", variant)
    for spec in HELD_OUT_3_MANUSCRIPTS:
        project_id, _ = seed_manuscript(client, spec, planted)
        manifest["manuscripts"].append(_project_snapshot(client, spec["title"], project_id))
    return manifest


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument(
        "--out-dir",
        type=Path,
        default=Path(__file__).resolve().parent / "semantic_candidates",
    )
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    suites = {
        "baseline": _baseline,
        "held-out-72": _held_out,
        "held-out-2": _held_out_2,
        "held-out-3": _held_out_3,
    }
    for suite_name, builder in suites.items():
        for variant in ("control", "planted"):
            with tempfile.TemporaryDirectory(prefix=f"semantic-candidates-{suite_name}-") as temp:
                client = McpClient(args.binary, Path(temp))
                try:
                    snapshot = builder(client, variant)
                finally:
                    client.close()
            path = args.out_dir / f"{suite_name}.{variant}.json"
            path.write_text(json.dumps(snapshot, indent=2, sort_keys=True) + "\n")
            print(f"wrote {path}")


if __name__ == "__main__":
    main()

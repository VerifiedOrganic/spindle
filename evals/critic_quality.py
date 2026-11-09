#!/usr/bin/env python3
"""Offline critic-quality replay over saved G initial drafts.

This is a drafting eval, not a full G: it reconstructs each loop's initial
PlainTextDraftArtifactV1 and runs the critic + prior-draft revision + internal
acceptance judge on that exact draft (no redrafting, no 3-family G panel).
Before metrics come from the original G loop artifacts; after metrics come from
the replayed loop artifacts.

Run live with:
  ai run OLLAMA_API_KEY -- python3 evals/critic_quality.py replay \
    --candidates /path/to/candidates_loop.json \
    --config /path/to/agents.toml --db /path/to/eval.db \
    --binary /path/to/spindle-tokenizer \
    --out evals/results/drafting_critic_quality.json
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

RESULTS_DIR = Path(__file__).resolve().parent / "results"


def read_json(path: str | Path):
    with open(path) as f:
        return json.load(f)


def write_json(path: str | Path, value) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w") as f:
        json.dump(value, f, indent=2)
        f.write("\n")


def loop_metrics(loop: dict) -> dict:
    rounds = loop.get("rounds") or []
    first = rounds[0] if rounds else {}
    outcome = first.get("critique_outcome") or {}
    status = outcome.get("status")
    critique = outcome.get("critique") or {}
    directives = critique.get("directives") or []
    actionable = status == "critiqued" and len(directives) > 0
    no_change = status == "critiqued" and len(directives) == 0
    invalid = status == "critic_invalid"
    attempted = actionable
    promoted = bool(loop.get("revision_promoted"))
    return {
        "status": status,
        "directives": len(directives),
        "actionable": actionable,
        "no_change": no_change,
        "invalid": invalid,
        "attempted": attempted,
        "promoted": promoted,
        "no_change_reason": (critique.get("no_change") or {}).get("reason_code"),
        "retention_reason": loop.get("retention_reason"),
        "calls": int(loop.get("total_calls_used") or 0),
        "tokens": int(loop.get("total_tokens_used") or 0),
    }


def aggregate(rows: list[dict]) -> dict:
    n = len(rows)
    invalid = sum(r["invalid"] for r in rows)
    actionable = sum(r["actionable"] for r in rows)
    no_change = sum(r["no_change"] for r in rows)
    attempted = sum(r["attempted"] for r in rows)
    promoted = sum(r["promoted"] for r in rows)
    bridge_failed = sum(r.get("status") == "bridge_failed" for r in rows)
    rates = {
        "invalid_rate": round(invalid / n, 4) if n else 0.0,
        "actionable_rate": round(actionable / n, 4) if n else 0.0,
        "no_change_rate": round(no_change / n, 4) if n else 0.0,
        "revision_attempted_rate": round(attempted / n, 4) if n else 0.0,
        "revision_promoted_rate": round(promoted / n, 4) if n else 0.0,
    }
    by_reason = Counter(
        r["no_change_reason"] or "none" for r in rows if r["no_change"]
    )
    return {
        "scenes": n,
        "invalid": invalid,
        "actionable": actionable,
        "no_change": no_change,
        "revision_attempted": attempted,
        "revision_promoted": promoted,
        "bridge_failed": bridge_failed,
        **rates,
        "no_change_reason_counts": dict(sorted(by_reason.items())),
        "calls": sum(r.get("calls", 0) for r in rows),
        "tokens": sum(r.get("tokens", 0) for r in rows),
    }


def metric_delta(before: dict, after: dict) -> dict:
    keys = [
        "invalid_rate",
        "actionable_rate",
        "no_change_rate",
        "revision_attempted_rate",
        "revision_promoted_rate",
    ]
    return {
        key: round(after[key] - before[key], 4)
        for key in keys
    }


def build_initial_artifact(candidate: dict, work_dir: Path) -> Path:
    loop = read_json(candidate["loop_artifact_path"])
    final = read_json(candidate["artifact_path"])
    initial = dict(final)
    initial["text"] = loop["initial_text"]
    initial["text_sha256"] = loop["initial_text_sha256"]
    initial["receipt"] = loop["initial_receipt"]
    initial["accepted_text_sha256"] = loop["initial_text_sha256"]
    initial["artifact_id"] = loop["initial_draft_artifact_id"]
    initial["extraction"] = None
    initial["extraction_request"] = None
    initial["extraction_attempts"] = 0
    initial["extraction_failure"] = None
    initial["truncated"] = False
    path = work_dir / "initial_artifacts" / f"{candidate['brief_id']}_{candidate['generation_seed']}.artifact.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    write_json(path, initial)
    return path


def replay(args: argparse.Namespace) -> dict:
    candidates = read_json(args.candidates)
    if args.limit:
        candidates = candidates[: args.limit]
    briefs = {b["id"]: b for b in read_json(args.briefs)["briefs"]}
    work_dir = Path(args.work_dir or tempfile.mkdtemp(prefix="critic-quality-"))
    work_dir.mkdir(parents=True, exist_ok=True)
    (work_dir / "loops").mkdir(exist_ok=True)
    db_path = Path(args.db)
    db_path.parent.mkdir(parents=True, exist_ok=True)

    before_rows: list[dict] = []
    after_rows: list[dict] = []
    failures: list[dict] = []
    settings = None

    for idx, candidate in enumerate(candidates, 1):
        brief_id = candidate["brief_id"]
        seed = candidate["generation_seed"]
        scene_type = briefs.get(brief_id, {}).get("scene_type", "unknown")
        old_loop = read_json(candidate["loop_artifact_path"])
        before = loop_metrics(old_loop)
        before["brief_id"] = brief_id
        before["generation_seed"] = seed
        before["scene_type"] = scene_type
        before_rows.append(before)

        initial_path = build_initial_artifact(candidate, work_dir)
        loop_path = work_dir / "loops" / f"{brief_id}_{seed}.loop.json"
        cmd = [
            args.binary,
            "revise-artifact",
            args.config,
            str(db_path),
            str(initial_path),
            str(loop_path),
            "-",
            str(seed),
            "1",
        ]
        try:
            proc = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=args.timeout,
                check=False,
            )
        except subprocess.TimeoutExpired:
            proc = None
        if proc is None or proc.returncode != 0 or not loop_path.exists():
            after = {
                "status": "bridge_failed",
                "directives": 0,
                "actionable": False,
                "no_change": False,
                "invalid": False,
                "attempted": False,
                "promoted": False,
                "no_change_reason": None,
                "retention_reason": "bridge_failed",
                "calls": 0,
                "tokens": 0,
                "brief_id": brief_id,
                "generation_seed": seed,
                "scene_type": scene_type,
            }
            failures.append({"brief_id": brief_id, "generation_seed": seed})
        else:
            new_loop = read_json(loop_path)
            after = loop_metrics(new_loop)
            after["brief_id"] = brief_id
            after["generation_seed"] = seed
            after["scene_type"] = scene_type
            if settings is None:
                meta = new_loop.get("initial_receipt") or {}
                settings = {
                    "agent_id": meta.get("agent_id"),
                    "model": candidate.get("model"),
                    "tokenizer_id": candidate.get("tokenizer_id"),
                    "max_rounds": new_loop.get("max_rounds"),
                }
        after_rows.append(after)
        if idx % 10 == 0 or idx == len(candidates):
            print(f"replayed {idx}/{len(candidates)}", flush=True)

    scene_types = sorted({r["scene_type"] for r in before_rows})
    by_scene_type = {}
    for scene_type in scene_types:
        b = [r for r in before_rows if r["scene_type"] == scene_type]
        a = [r for r in after_rows if r["scene_type"] == scene_type]
        by_scene_type[scene_type] = {
            "before": aggregate(b),
            "after": aggregate(a),
            "delta": metric_delta(aggregate(b), aggregate(a)),
        }

    before = aggregate(before_rows)
    after = aggregate(after_rows)
    return {
        "schema_version": "drafting_critic_quality_v1",
        "description": "Offline critic/revision replay over the 90 saved initial drafts from the row-6 G run. Before = original G critic; after = branch critic with quote-first anchors, typed no-change, scene focus, and compact repair. No redrafting and no 3-family G judge panel.",
        "n_scenes": len(before_rows),
        "n_briefs": len({r["brief_id"] for r in before_rows}),
        "generation_seeds": sorted({r["generation_seed"] for r in before_rows}),
        "before": before,
        "after": after,
        "delta": metric_delta(before, after),
        "by_scene_type": by_scene_type,
        "settings": settings or {},
        "failures": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    replay_p = sub.add_parser("replay", help="Replay the critic/revision loop on saved G drafts")
    replay_p.add_argument("--candidates", required=True, help="candidates_loop.json from the G run")
    replay_p.add_argument("--briefs", default=str(Path(__file__).resolve().parent / "prose_judge" / "briefs.json"))
    replay_p.add_argument("--config", required=True, help="agents TOML with draft/review/revise/judge routes")
    replay_p.add_argument("--db", required=True, help="fresh SQLite usage/probe DB")
    replay_p.add_argument("--binary", required=True, help="spindle-tokenizer binary")
    replay_p.add_argument("--work-dir", default=None, help="directory for reconstructed initial artifacts and loop outputs")
    replay_p.add_argument("--limit", type=int, default=None)
    replay_p.add_argument("--timeout", type=int, default=900)
    replay_p.add_argument("--out", default=str(RESULTS_DIR / "drafting_critic_quality.json"))
    args = parser.parse_args()
    if args.command == "replay":
        result = replay(args)
        result = {k: v for k, v in result.items() if k != "work_dir"}
        write_json(args.out, result)
        print(
            "before: invalid={invalid_rate} actionable={actionable_rate} "
            "promoted={revision_promoted_rate}; after: invalid={after_invalid} "
            "actionable={after_actionable} promoted={after_promoted}".format(
                **result["before"],
                after_invalid=result["after"]["invalid_rate"],
                after_actionable=result["after"]["actionable_rate"],
                after_promoted=result["after"]["revision_promoted_rate"],
            ),
            flush=True,
        )
        print(f"wrote {args.out}", flush=True)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Semantic-on evaluation for the public continuity suites.

Seeds each corpus once, runs `check_consistency` in on mode (the pass that
dispatches the judge and emits findings), then reruns the same projects in
shadow mode against the validated-report cache. Each mode keeps its own
results, so repair/cost metrics are never borrowed across passes; shadow/on
would-be parity is measured from the two actual summaries. Reports
deterministic-vs-semantic recall, precision/control counts, evidence validity,
repairs, p95 tokens/latency, calls, cache hits, and totals per scene/chapter.
"""
from __future__ import annotations

import math
from pathlib import Path
import tempfile
from typing import Any

from continuity_recall import (
    McpClient,
    issue_matches_contradiction,
    score_manuscript,
    seed_ashfall_manuscript,
    seed_veridian_manuscript,
)
from continuity_held_out import MANUSCRIPTS as H1_SPECS, seed_held_out_manuscript
from continuity_held_out_2 import MANUSCRIPTS as H2_SPECS, _build_manuscript, _score as h2_score
from continuity_held_out_3 import (
    MANUSCRIPTS as H3_SPECS,
    _score_planted,
    planted_contradictions,
    seed_manuscript,
)

SET_COMMANDS = {
    "run": "baseline",
    "held-out": "held-out-72",
    "held-out-2": "held-out-2",
    "held-out-3": "held-out-3",
}


def _check(client: McpClient, project_id: str, mode: str) -> dict[str, Any]:
    return client.tool(
        "check_consistency",
        {
            "project_id": project_id,
            "scope": {"scope_type": "full"},
            "checks": [],
            "severity_filter": [],
            "deep_check": False,
            "subjects": [],
            "semantic_mode": mode,
            "semantic_host_reports": [],
        },
    )


def _semantic(issues: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [issue for issue in issues if "(semantic)" in issue.get("message", "")]


def _deterministic(issues: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [issue for issue in issues if "(semantic)" not in issue.get("message", "")]


def _p95(values: list[int | float]) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    return float(ordered[max(0, math.ceil(0.95 * len(ordered)) - 1)])


def _metrics(
    checks: list[dict[str, Any]],
    chapters: int,
    control_checks: list[dict[str, Any]],
) -> dict[str, Any]:
    summaries = [check.get("semantic_summary") or {} for check in checks]
    scenes = [
        scene
        for check in checks
        for scene in check.get("semantic_scene_summaries", [])
    ]
    issues = [issue for check in checks for issue in check.get("issues", [])]
    semantic = _semantic(issues)
    control_issues = [
        issue for check in control_checks for issue in check.get("issues", [])
    ]
    warnings: dict[str, int] = {}
    for issue in control_issues:
        if issue.get("severity") in ("warning", "error"):
            kind = issue.get("check_type", "unknown")
            warnings[kind] = warnings.get(kind, 0) + 1
    input_tokens = sum(int(summary.get("input_tokens", 0)) for summary in summaries)
    output_tokens = sum(int(summary.get("output_tokens", 0)) for summary in summaries)
    cache_hits = sum(int(summary.get("cache_hits", 0)) for summary in summaries)
    cache_misses = sum(int(summary.get("cache_misses", 0)) for summary in summaries)
    call_receipts = [
        call
        for summary in summaries
        for call in summary.get("calls", [])
    ]
    calls = len(call_receipts)
    judgments = sum(int(summary.get("judgments", 0)) for summary in summaries)
    candidate_scenes = sum(1 for scene in scenes if scene.get("candidates", 0) > 0)
    repairs = sum(1 for call in call_receipts if call.get("repaired"))
    if not call_receipts:
        # Backward-compatible fallback for older reports without receipts.
        calls = sum(int(summary.get("model_calls", 0)) for summary in summaries)
        repairs = sum(1 for scene in scenes if scene.get("repaired"))
    evidence_valid = sum(
        1
        for issue in semantic
        if issue.get("evidence")
        and issue["evidence"].get("start_byte", 0) < issue["evidence"].get("end_byte", 0)
        and issue["evidence"].get("exact_quote")
    )
    errors_by_kind: dict[str, int] = {}
    for summary in summaries:
        for error in summary.get("errors", []):
            kind = error.get("kind", "unknown")
            errors_by_kind[kind] = errors_by_kind.get(kind, 0) + 1
    verdicts: dict[str, int] = {}
    discard_reasons: dict[str, int] = {}
    stop_reasons: dict[str, int] = {}
    confidence_buckets = [0, 0, 0, 0, 0]
    for summary in summaries:
        for key, count in summary.get("verdicts", {}).items():
            verdicts[key] = verdicts.get(key, 0) + int(count)
        for key, count in summary.get("discard_reasons", {}).items():
            discard_reasons[key] = discard_reasons.get(key, 0) + int(count)
        for key, count in summary.get("stop_reasons", {}).items():
            stop_reasons[key] = stop_reasons.get(key, 0) + int(count)
        for index, count in enumerate(summary.get("confidence_buckets", [])):
            if index < len(confidence_buckets):
                confidence_buckets[index] += int(count)
    return {
        "issues": issues,
        "errors_by_kind": dict(sorted(errors_by_kind.items())),
        "verdicts": dict(sorted(verdicts.items())),
        "confidence_buckets": confidence_buckets,
        "discard_reasons": dict(sorted(discard_reasons.items())),
        "stop_reasons": dict(sorted(stop_reasons.items())),
        "semantic_issues": semantic,
        "semantic_warning_count": sum(
            1 for issue in semantic if issue.get("severity") == "warning"
        ),
        "control_warnings_by_check_type": dict(sorted(warnings.items())),
        "model_calls": calls,
        "judgments": judgments,
        "would_be_warnings": sum(int(summary.get("warnings", 0)) for summary in summaries),
        "would_be_infos": sum(int(summary.get("infos", 0)) for summary in summaries),
        "semantic_control_warnings": sum(
            1
            for issue in _semantic(control_issues)
            if issue.get("severity") in ("warning", "error")
        ),
        "candidate_scene_count": candidate_scenes,
        "calls_per_candidate_scene": round(calls / candidate_scenes, 4) if candidate_scenes else 0.0,
        "cache_hit_rate": round(cache_hits / (cache_hits + cache_misses), 4)
        if cache_hits + cache_misses
        else 0.0,
        "repair_rate": round(repairs / calls, 4) if calls else 0.0,
        "repaired_scene_rate": round(
            sum(1 for scene in scenes if scene.get("repaired")) / candidate_scenes, 4
        )
        if candidate_scenes
        else 0.0,
        "per_call_receipts": len(call_receipts),
        "p95_prompt_tokens": _p95(
            [call.get("input_tokens", 0) for call in call_receipts]
        )
        if call_receipts
        else _p95([scene.get("input_tokens", 0) for scene in scenes]),
        "p95_output_tokens": _p95(
            [call.get("output_tokens", 0) for call in call_receipts]
        )
        if call_receipts
        else _p95([scene.get("output_tokens", 0) for scene in scenes]),
        "p95_latency_ms": _p95([call.get("elapsed_ms", 0) for call in call_receipts])
        if call_receipts
        else _p95([scene.get("elapsed_ms", 0) for scene in scenes]),
        "p95_output_chars": _p95([call.get("output_chars", 0) for call in call_receipts])
        if call_receipts
        else 0.0,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "tokens_per_scene": round((input_tokens + output_tokens) / len(scenes), 2) if scenes else 0.0,
        "tokens_per_chapter": round((input_tokens + output_tokens) / chapters, 2) if chapters else 0.0,
        "evidence_anchor_validity": round(evidence_valid / len(semantic), 4) if semantic else 1.0,
    }


def _score_report(set_name: str, report: dict[str, Any], issues_key: str) -> dict[str, Any]:
    control = report["control"][issues_key]
    planted = report["planted"][issues_key]
    if set_name in ("baseline", "held-out-72"):
        score = score_manuscript(
            manuscript_id=report["name"],
            chapter_count=report["chapters"],
            control_issues=control,
            planted_issues=planted,
            planted_contradictions=report["items"],
        )
        return {
            "localized": score["detected_total"],
            "type_present": score["detected_total"],
            "planted": score["planted_total"],
            "control_count": score["control_false_positive_count"],
        }
    if set_name == "held-out-2":
        score = h2_score(planted, report["items"])
        return {
            "localized": score["localized_detected"],
            "type_present": score["type_detected"],
            "planted": score["planted"],
            "control_count": 0,
        }
    score = _score_planted(report["spec"], report["scene_ids"], control, planted)
    return {
        "localized": score["localized_total"],
        "type_present": score["type_present_total"],
        "planted": score["planted_total"],
        "control_count": score["control_false_positive_count"],
    }


def _seed_reports(client: McpClient, set_name: str) -> tuple[list[dict[str, Any]], int]:
    reports: list[dict[str, Any]] = []
    if set_name == "baseline":
        specs = [("ashfall_expedition", seed_ashfall_manuscript), ("veridian_station", seed_veridian_manuscript)]
        for name, seeder in specs:
            control_id, _ = seeder(client, False)
            planted_id, items = seeder(client, True)
            reports.append(
                {
                    "name": name,
                    "chapters": 20,
                    "items": items,
                    "control_project": control_id,
                    "planted_project": planted_id,
                }
            )
        return reports, 40
    if set_name == "held-out-72":
        for spec in H1_SPECS:
            control_id, _ = seed_held_out_manuscript(client, spec, False)
            planted_id, items = seed_held_out_manuscript(client, spec, True)
            reports.append(
                {
                    "name": spec["id"],
                    "chapters": 16,
                    "items": items,
                    "control_project": control_id,
                    "planted_project": planted_id,
                }
            )
        return reports, 32
    if set_name == "held-out-2":
        for spec in H2_SPECS:
            control = _build_manuscript(client, spec, False)
            planted = _build_manuscript(client, spec, True)
            reports.append(
                {
                    "name": spec["name"],
                    "chapters": 15,
                    "items": planted["plants"],
                    "control_project": control["project_id"],
                    "planted_project": planted["project_id"],
                }
            )
        return reports, 30
    for spec in H3_SPECS:
        control_id, _ = seed_manuscript(client, spec, False)
        planted_id, scene_ids = seed_manuscript(client, spec, True)
        reports.append(
            {
                "name": spec["title"],
                "chapters": 16,
                "spec": spec,
                "scene_ids": scene_ids,
                "items": planted_contradictions(spec, scene_ids),
                "control_project": control_id,
                "planted_project": planted_id,
            }
        )
    return reports, 32


def _precision_from_warning_matches(matches: list[bool]) -> float | None:
    """Precision over warning instances; empty means undefined (None)."""
    if not matches:
        return None
    return round(sum(1 for match in matches if match) / len(matches), 4)


def _shadow_on_parity(reports: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Compare would-be finding counts from the actual shadow and on runs.

    Both modes share the validated-report cache, so a mismatch means the shadow
    path produced different would-be findings or failed where on succeeded.
    """
    parity: list[dict[str, Any]] = []
    for report in reports:
        for variant in ("control", "planted"):
            on_summary = report[variant]["on"].get("semantic_summary") or {}
            shadow_summary = report[variant]["shadow"].get("semantic_summary") or {}
            on_counts = (
                int(on_summary.get("warnings", 0)),
                int(on_summary.get("infos", 0)),
            )
            shadow_counts = (
                int(shadow_summary.get("warnings", 0)),
                int(shadow_summary.get("infos", 0)),
            )
            on_judgments = int(on_summary.get("judgments", 0))
            shadow_judgments = int(shadow_summary.get("judgments", 0))
            on_verdicts = on_summary.get("verdicts") or {}
            shadow_verdicts = shadow_summary.get("verdicts") or {}
            on_discards = on_summary.get("discard_reasons") or {}
            shadow_discards = shadow_summary.get("discard_reasons") or {}
            on_errors = sorted(
                error.get("kind", "unknown") for error in on_summary.get("errors") or []
            )
            shadow_errors = sorted(
                error.get("kind", "unknown") for error in shadow_summary.get("errors") or []
            )
            parity.append(
                {
                    "name": report["name"],
                    "variant": variant,
                    "on_warnings": on_counts[0],
                    "on_infos": on_counts[1],
                    "shadow_warnings": shadow_counts[0],
                    "shadow_infos": shadow_counts[1],
                    "on_judgments": on_judgments,
                    "shadow_judgments": shadow_judgments,
                    "on_verdicts": on_verdicts,
                    "shadow_verdicts": shadow_verdicts,
                    "on_discard_reasons": on_discards,
                    "shadow_discard_reasons": shadow_discards,
                    "on_error_kinds": on_errors,
                    "shadow_error_kinds": shadow_errors,
                    # The gate is would-be finding parity. Verdict/discard/error
                    # differences are reported for diagnosis but can legitimately
                    # come from cache retries and the judge-only verifier.
                    "matches": on_counts == shadow_counts,
                    "summary_matches": (
                        on_judgments == shadow_judgments
                        and on_verdicts == shadow_verdicts
                        and on_discards == shadow_discards
                        and on_errors == shadow_errors
                    ),
                }
            )
    return parity


def semantic_eval_self_test() -> None:
    fake = {
        "semantic_summary": {
            "input_tokens": 3000,
            "output_tokens": 500,
            "cache_hits": 0,
            "cache_misses": 2,
            "model_calls": 2,
            "judgments": 2,
            "warnings": 0,
            "infos": 0,
            "calls": [
                {"scene_id": "s1", "input_tokens": 1000, "output_tokens": 200, "elapsed_ms": 100, "repaired": False},
                {"scene_id": "s1", "input_tokens": 1200, "output_tokens": 300, "elapsed_ms": 400, "repaired": True},
            ],
        },
        "semantic_scene_summaries": [
            {"scene_id": "s1", "candidates": 2, "repaired": True},
            {"scene_id": "s2", "candidates": 1, "repaired": False},
        ],
        "issues": [],
    }
    metrics = _metrics([fake], 1, [fake])
    assert metrics["per_call_receipts"] == 2
    assert metrics["repair_rate"] == 0.5
    assert metrics["p95_latency_ms"] == 400.0
    assert metrics["p95_output_tokens"] == 300.0
    assert _precision_from_warning_matches([]) is None
    assert _precision_from_warning_matches([True]) == 1.0
    assert _precision_from_warning_matches([True, True]) == 1.0
    assert _precision_from_warning_matches([True, False]) == 0.5
    # Info findings never enter this list, and one warning matching several
    # plants contributes exactly one boolean match.
    assert _precision_from_warning_matches([True, True, False]) == 0.6667
    parity = _shadow_on_parity(
        [
            {
                "name": "m",
                "control": {
                    "on": {"semantic_summary": {"warnings": 1, "infos": 0}},
                    "shadow": {"semantic_summary": {"warnings": 1, "infos": 0}},
                },
                "planted": {
                    "on": {"semantic_summary": {"warnings": 2, "infos": 1}},
                    "shadow": {"semantic_summary": {"warnings": 2, "infos": 0}},
                },
            }
        ]
    )
    assert parity[0]["matches"] is True
    assert parity[1]["matches"] is False
    verdict_parity = _shadow_on_parity(
        [
            {
                "name": "v",
                "control": {
                    "on": {
                        "semantic_summary": {
                            "warnings": 1,
                            "infos": 0,
                            "verdicts": {"contradiction": 1},
                        }
                    },
                    "shadow": {
                        "semantic_summary": {
                            "warnings": 1,
                            "infos": 0,
                            "verdicts": {"compatible": 1},
                        }
                    },
                },
                "planted": {"on": {"semantic_summary": {}}, "shadow": {"semantic_summary": {}}},
            }
        ]
    )
    assert verdict_parity[0]["matches"] is True
    assert verdict_parity[0]["summary_matches"] is False


def _warning_precision(
    set_name: str,
    reports: list[dict[str, Any]],
    mode: str,
) -> float | None:
    """TP/FP over semantic warning instances; info findings never count."""
    true_positive = 0
    false_positive = 0
    for report in reports:
        control_warnings = [
            issue
            for issue in report["control"][mode]["issues"]
            if issue.get("severity") == "warning" and "(semantic)" in issue.get("message", "")
        ]
        planted_warnings = [
            issue
            for issue in report["planted"][mode]["issues"]
            if issue.get("severity") == "warning" and "(semantic)" in issue.get("message", "")
        ]
        false_positive += len(control_warnings)
        for warning in planted_warnings:
            if set_name in ("baseline", "held-out-72"):
                matched = any(issue_matches_contradiction(warning, item) for item in report["items"])
            elif set_name == "held-out-2":
                matched = h2_score([warning], report["items"])["localized_detected"] > 0
            else:
                matched = (
                    _score_planted(report["spec"], report["scene_ids"], [], [warning])[
                        "localized_total"
                    ]
                    > 0
                )
            if matched:
                true_positive += 1
            else:
                false_positive += 1
    matches = [True] * true_positive + [False] * false_positive
    return _precision_from_warning_matches(matches)


def run_semantic_set(
    binary: Path,
    set_name: str,
    agent_config: Path,
    judge_identity: str = "configured-agent",
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix=f"semantic-eval-{set_name}-") as tmp:
        client = McpClient(binary, Path(tmp), agent_config=agent_config)
        try:
            reports, chapters = _seed_reports(client, set_name)
            # Run on first: this is the pass that dispatches the judge and
            # emits findings, so its repair/cost receipts describe real
            # on-mode calls. The shadow pass then runs against the same
            # projects and the validated-report cache; its results are kept
            # as its own, not copied from on.
            for report in reports:
                report["control"] = {
                    "on": _check(client, report["control_project"], "on")
                }
                report["planted"] = {
                    "on": _check(client, report["planted_project"], "on")
                }
            for report in reports:
                report["control"]["shadow"] = _check(
                    client, report["control_project"], "shadow"
                )
                report["planted"]["shadow"] = _check(
                    client, report["planted_project"], "shadow"
                )
        finally:
            client.close()

    parity = _shadow_on_parity(reports)
    result: dict[str, Any] = {
        "set": set_name,
        "judge_identity": judge_identity,
        "shadow_on_parity": parity,
        "shadow_on_parity_ok": all(entry["matches"] for entry in parity),
        "modes": {},
    }
    for mode in ("shadow", "on"):
        mode_checks = []
        deterministic_scores = {"localized": 0, "type_present": 0, "planted": 0, "control_count": 0}
        semantic_scores = {"localized": 0, "type_present": 0, "planted": 0, "control_count": 0}
        for report in reports:
            deterministic = dict(report)
            deterministic["control"] = {"issues": _deterministic(report["control"][mode]["issues"])}
            deterministic["planted"] = {"issues": _deterministic(report["planted"][mode]["issues"])}
            det = _score_report(set_name, deterministic, "issues")
            for key in deterministic_scores:
                deterministic_scores[key] += det[key]
            full = {
                **report,
                "control": {"issues": report["control"][mode]["issues"]},
                "planted": {"issues": report["planted"][mode]["issues"]},
            }
            semantic = _score_report(set_name, full, "issues")
            for key in semantic_scores:
                semantic_scores[key] += semantic[key]
            mode_checks.extend([report["control"][mode], report["planted"][mode]])
        control_checks = [report["control"][mode] for report in reports]
        metrics = _metrics(mode_checks, chapters, control_checks)
        # Semantic-only precision uses only issues carrying the semantic suffix.
        semantic_only = {"localized": 0, "type_present": 0, "planted": 0, "control_count": 0}
        for report in reports:
            filtered = dict(report)
            filtered["control"] = {"issues": _semantic(report["control"][mode]["issues"])}
            filtered["planted"] = {"issues": _semantic(report["planted"][mode]["issues"])}
            score = _score_report(set_name, filtered, "issues")
            for key in semantic_only:
                semantic_only[key] += score[key]
        result["modes"][mode] = {
            "deterministic": deterministic_scores,
            "semantic_on": semantic_scores,
            "semantic_only": semantic_only,
            "semantic_warning_precision": _warning_precision(set_name, reports, mode),
            **metrics,
        }
    return result

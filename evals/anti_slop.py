#!/usr/bin/env python3
"""Fiction anti-slop eval / CI harness (Phase 4).

Commands:
  self-test  Validate case schema, shelf-ID coverage, and preference-dim labels.
  drift      Fail if catalog MD, pack TOML, and scanner IDs diverge.
  regress    Run the Rust eval cases (`cargo test -p spindle-core --test antislop_eval`).
  score      Tally optional labeled preference dims (not a quality ranking).

Standard library only. Never dispatches a writing model.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CASES_PATH = HERE / "anti_slop" / "cases.json"
CATALOG = REPO / "references" / "anti-slop.md"
PACK = REPO / "references" / "anti-slop-shelf-pack.v0.toml"
SCANNER = REPO / "crates" / "spindle-core" / "src" / "style" / "antislop" / "scan.rs"

NON_PORTS = (
    "bluf",
    "tech_buzzlist",
    "em_dash_gate",
    "flesch",
    "detector_percent",
    "faq_ending",
    "delve_as_tech_gate",
)
DIM_VALUES = {"present", "absent", "n/a"}


def read_cases():
    return json.loads(CASES_PATH.read_text())


def catalog_shelf_ids(markdown: str) -> list[str]:
    return re.findall(r"(?m)^### `([a-z][a-z0-9_]+)`", markdown)


def pack_shelf_ids(toml_src: str) -> list[str]:
    ids = []
    in_shelf = False
    for line in toml_src.splitlines():
        if line.startswith("[[shelves]]"):
            in_shelf = True
            continue
        if line.startswith("[") and not line.startswith("[[shelves]]"):
            in_shelf = False
        if in_shelf:
            match = re.match(r'id = "([a-z][a-z0-9_]+)"', line)
            if match:
                ids.append(match.group(1))
    return ids


def scanner_shelf_ids(rust_src: str) -> list[str]:
    block = re.search(
        r"pub const SCANNER_SHELF_IDS: &\[&str\] = &\[(.*?)\];",
        rust_src,
        re.S,
    )
    if not block:
        raise ValueError("SCANNER_SHELF_IDS constant missing from scan.rs")
    return re.findall(r'"([a-z][a-z0-9_]+)"', block.group(1))


def drift() -> dict:
    catalog = catalog_shelf_ids(CATALOG.read_text())
    pack = pack_shelf_ids(PACK.read_text())
    scanner = scanner_shelf_ids(SCANNER.read_text())
    declared = list(read_cases()["shelf_ids"])
    sets = {
        "catalog": catalog,
        "pack": pack,
        "scanner": scanner,
        "eval_manifest": declared,
    }
    first = catalog
    ok = all(ids == first for ids in sets.values())
    if "solitary_fade" not in catalog:
        ok = False
    if any(port in catalog or port in pack or port in scanner for port in NON_PORTS):
        ok = False
    report = {
        "ok": ok,
        "counts": {name: len(ids) for name, ids in sets.items()},
        "ids": sets,
    }
    if not ok:
        report["error"] = (
            "shelf ID drift: catalog MD, pack TOML, scanner, and eval manifest must match"
        )
    return report


def validate_cases(bundle: dict) -> list[str]:
    errors = []
    shelf_ids = bundle.get("shelf_ids") or []
    dims = set(bundle.get("preference_dims") or [])
    cases = bundle.get("cases") or []
    if len(shelf_ids) != 12:
        errors.append(f"eval manifest must list 12 shelves, got {len(shelf_ids)}")
    if "solitary_fade" not in shelf_ids:
        errors.append("solitary_fade must stay in the eval manifest")
    seen_ids = []
    covered = set()
    for case in cases:
        cid = case.get("id")
        if not cid:
            errors.append("case missing id")
            continue
        seen_ids.append(cid)
        if not case.get("prose"):
            errors.append(f"{cid}: prose required")
        expect = case.get("expect") or {}
        if expect.get("skipped"):
            errors.append(f"{cid}: eval cases are fiction; skipped must be false")
        for hit in expect.get("must_include") or []:
            shelf = hit.get("shelf_id")
            severity = hit.get("severity")
            if shelf not in shelf_ids:
                errors.append(f"{cid}: unknown must_include shelf {shelf}")
            if severity not in ("hard", "soft"):
                errors.append(f"{cid}: severity must be hard|soft")
            if shelf:
                covered.add(shelf)
        for excluded in expect.get("must_exclude") or []:
            if excluded in shelf_ids and excluded in {
                hit.get("shelf_id") for hit in (expect.get("must_include") or [])
            }:
                errors.append(f"{cid}: {excluded} cannot be both included and excluded")
        labels = case.get("preference_dims") or {}
        for dim, value in labels.items():
            if dim not in dims:
                errors.append(f"{cid}: unknown preference dim {dim}")
            if value not in DIM_VALUES:
                errors.append(
                    f"{cid}: preference dim {dim} must be present|absent|n/a (not a quality score)"
                )
    if len(seen_ids) != len(set(seen_ids)):
        errors.append("duplicate case ids")
    missing = [shelf for shelf in shelf_ids if shelf not in covered]
    if missing:
        errors.append(f"eval cases must cover every shelf; missing {missing}")
    if bundle.get("interpretation") and "not a quality" not in bundle["interpretation"].lower():
        errors.append("preference-dim interpretation must refuse a quality superiority claim")
    return errors


def self_test() -> None:
    bundle = read_cases()
    errors = validate_cases(bundle)
    report = drift()
    if not report["ok"]:
        errors.append(report.get("error") or "drift failed")
    if errors:
        print("self-test failed:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        raise SystemExit(1)
    ratings = [
        {
            "case_id": case["id"],
            "preference_dims": case.get("preference_dims") or {},
        }
        for case in bundle["cases"]
    ]
    scored = score(ratings)
    assert scored["interpretation"].lower().find("not a quality") >= 0
    assert "superiority" in scored["interpretation"].lower()
    print(
        "Case schema, twelve-shelf coverage, solitary_fade lock, drift, "
        "and labeled preference dims (no quality claim) passed."
    )


def score(ratings: list[dict]) -> dict:
    bundle = read_cases()
    dims = bundle["preference_dims"]
    tallies = {dim: Counter() for dim in dims}
    labeled = 0
    for row in ratings:
        labels = row.get("preference_dims") or {}
        if labels:
            labeled += 1
        for dim, value in labels.items():
            if dim not in tallies:
                raise ValueError(f"unknown preference dim {dim}")
            if value not in DIM_VALUES:
                raise ValueError(
                    f"{dim} must be present|absent|n/a; numeric scores are not a quality ranking"
                )
            tallies[dim][value] += 1
    return {
        "cases_labeled": labeled,
        "cases_unlabeled": len(ratings) - labeled,
        "dims": {dim: dict(counter) for dim, counter in tallies.items()},
        "interpretation": bundle["interpretation"],
    }


def regress() -> None:
    cmd = [
        "cargo",
        "test",
        "-p",
        "spindle-core",
        "--test",
        "antislop_eval",
        "--",
        "--nocapture",
    ]
    print(" ".join(cmd))
    raise SystemExit(subprocess.call(cmd, cwd=REPO))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("self-test")
    sub.add_parser("drift")
    sub.add_parser("regress")
    score_p = sub.add_parser("score")
    score_p.add_argument(
        "--ratings",
        help="JSON list of {case_id, preference_dims}. Default: labels baked into cases.json",
    )
    args = parser.parse_args()
    if args.command == "self-test":
        self_test()
    elif args.command == "drift":
        report = drift()
        print(json.dumps(report, indent=2))
        if not report["ok"]:
            raise SystemExit(1)
    elif args.command == "regress":
        regress()
    else:
        if args.ratings:
            ratings = json.loads(Path(args.ratings).read_text())
        else:
            ratings = [
                {
                    "case_id": case["id"],
                    "preference_dims": case.get("preference_dims") or {},
                }
                for case in read_cases()["cases"]
            ]
        print(json.dumps(score(ratings), indent=2))


if __name__ == "__main__":
    main()

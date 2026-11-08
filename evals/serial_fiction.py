#!/usr/bin/env python3
"""Serial-fiction comparison kit. Standard library; never dispatches a writing model."""
import argparse
import hashlib
import itertools
import json
import math
from pathlib import Path
import random
import statistics
import tempfile
import urllib.request

CONDITIONS = ("compact", "spindle_context", "full_workflow")
PAIRWISE_CONDITIONS = ("baseline", "candidate")
HERE = Path(__file__).resolve().parent


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False).encode()).hexdigest()


class Mcp:
    """Small synchronous client for local fixture setup, not browser automation."""
    def __init__(self, endpoint):
        self.endpoint, self.session, self.counter = endpoint, None, 0
        self.version = "2025-03-26"
        result = self.rpc("initialize", {"protocolVersion": self.version, "capabilities": {},
                          "clientInfo": {"name": "spindle-serial-eval", "version": "1"}})
        self.version = result["protocolVersion"]
        self.rpc("notifications/initialized", notification=True)

    def rpc(self, method, params=None, notification=False):
        self.counter += 1
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notification:
            message["id"] = self.counter
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "MCP-Protocol-Version": self.version}
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.endpoint, json.dumps(message).encode(), headers)
        with urllib.request.urlopen(request, timeout=300) as response:
            self.session = response.headers.get("Mcp-Session-Id", self.session)
            if notification:
                return None
            if "text/event-stream" in response.headers.get("Content-Type", ""):
                data, result = [], None
                for raw in response:
                    line = raw.decode().rstrip("\r\n")
                    if line.startswith("data:"):
                        data.append(line[5:].lstrip())
                    elif not line and data:
                        payload = "\n".join(data); data = []
                        if not payload.strip():
                            continue
                        parsed = json.loads(payload)
                        if parsed.get("id") == self.counter:
                            result = parsed
                            break
                if result is None:
                    raise ValueError("MCP stream ended without a result")
            else:
                result = json.load(response)
        if "error" in result:
            raise ValueError(result["error"])
        return result["result"]

    def call(self, name, arguments):
        result = self.rpc("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise ValueError(result["content"])
        if "structuredContent" in result:
            return result["structuredContent"]
        return json.loads(next(c["text"] for c in result["content"] if c["type"] == "text"))


def cases():
    data = read(HERE / "cases.json")
    for case in data:
        if case.get("history_chapters"):
            # Original synopsis fixture, not a claim of 200 chapters of human prose.
            episodes = []
            for n in range(1, case["history_chapters"] + 1):
                book, chapter = (n - 1) // 100 + 1, (n - 1) % 100 + 1
                summary = f"Mara delivers ledger {n} to the next river station. The ferry toll rises by one copper, and she records the village's objections before moving downstream."
                if n == 1:
                    summary = "Mara promises the ferryman she will bring back his daughter's brass compass. She leaves without finding it."
                if n == 199:
                    summary = "The ferry is burned. Mara takes the southern footbridge; her left boot is torn and her companion Ivo carries the only food."
                if n == 200:
                    summary = "At Southwatch, Ivo gives their last food to a stranded child. Mara recognizes the ferryman's brass compass in the child's hand."
                episodes.append({"book": book, "chapter": chapter, "summary": summary})
            case["episodes"] = episodes
    return data


def collect(endpoint, output):
    """Mutates only new, clearly named fixture projects on the supplied server."""
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    if (output / "captures.json").exists():
        raise ValueError("captures.json already exists; choose a new directory to preserve provenance")
    client, captures = Mcp(endpoint), []
    for case in cases():
        project = client.call("create_project", {"name": f"EVAL {case['id']} — {case['name']}",
            "project_type": "webserial", "genre": case.get("genre", "fantasy"),
            "reader_contract": {"promise": case["contract"], "style_notes": [], "boundaries": []}})["project_id"]
        loc = client.call("create_location", {"project_id": project, "name": case.get("location", "River station"),
            "summary": "The location of the next scene.", "kind": "settlement"})["location_id"]
        character_ids = []
        for name in case.get("characters", ["Mara", "Ivo"]):
            character_ids.append(client.call("create_character", {"project_id": project, "name": name,
                "summary": "A traveller whose knowledge is limited to witnessed events.", "role": "protagonist"})["character_id"])
        target = tuple(case["target"])
        max_book = max([target[0]] + [e["book"] for e in case["episodes"]])
        for book in range(2, max_book + 1):
            client.call("create_book", {"project_id": project, "title": f"Book {book}"})
        for episode in case["episodes"]:
            position = {"project_id": project, "book_number": episode["book"], "chapter_number": episode["chapter"]}
            client.call("create_chapter", position)
            client.call("save_summary", {**position, "summary": episode["summary"]})
        for rule in case.get("rules", []):
            client.call("create_world_rule", {"project_id": project, "rule_name": rule[0], "rule_type": "world", "description": rule[1]})
        for promise in case.get("promises", []):
            pid = client.call("create_narrative_promise", {"project_id": project, "promise_type": "foreshadowing", "description": promise["description"],
                "planted_at": {"book_number": promise["planted"][0], "chapter_number": promise["planted"][1]},
                "planned_payoff": {"book_number": promise["planned"][0], "chapter_number": promise["planned"][1]} if promise.get("planned") else None})["narrative_promise_id"]
            for event in promise.get("events", []):
                client.call("update_promise_status", {"narrative_promise_id": pid, "status": event["status"],
                    "at": {"book_number": event["at"][0], "chapter_number": event["at"][1]} if event.get("at") else None})
        position = {"project_id": project, "book_number": target[0], "chapter_number": target[1]}
        client.call("create_chapter", position)
        client.call("plan_chapter", {**position, "synopsis": case["task"], "scenes": [{"scene_order": 1,
            "summary": case["task"], "purpose": case["task"], "location_id": loc, "character_ids": character_ids, "content_rating": "general"}]})
        context_input = {**position, "scene_order": 1, "location_id": loc, "character_ids": character_ids, "format": "markdown", "budget_tokens": 5000}
        context = client.call("get_scene_context", context_input)
        captures.append({"case_id": case["id"], "case_hash": digest(case), "project_id": project,
                         "context_input": context_input, "context_output": context})
        write(output / "captures.json", captures)  # preserve completed setup if interrupted
        print(f"Captured {case['id']}", flush=True)


def prepare(captures_path, output):
    output = Path(output); output.mkdir(parents=True, exist_ok=True)
    captures = {c["case_id"]: c for c in read(captures_path)}
    requests = []
    for case in cases():
        capture = captures[case["id"]]
        if capture["case_hash"] != digest(case):
            raise ValueError(f"Stale case capture: {case['id']}")
        prior = [e for e in case["episodes"] if (e["book"], e["chapter"]) < tuple(case["target"])]
        common = f"Write 500–700 words for this episode. Contract: {case['contract']}\nTask: {case['task']}\nDo not add facts from future episodes to this viewpoint. Output prose only."
        for condition in CONDITIONS:
            request = {"case_id": case["id"], "condition": condition, "case_hash": digest(case), "task": common}
            if condition == "compact":
                request["context"] = [e["summary"] for e in prior[-3:]]
                request["workflow"] = "One generation from this compact context, with no tools or revision pass."
            elif condition == "spindle_context":
                request["context"] = capture["context_output"]
                request["workflow"] = "One generation from this captured Spindle context, with no additional tools or revision pass."
            else:
                request["project_id"] = capture["project_id"]
                request["context_input"] = capture["context_input"]
                request["workflow"] = "Use the scene-writer workflow in this fixture project: get_scene_context, draft with authorship assistant, check_consistency, read_episode and inspect editorial concerns. At most one revision. Return final prose. Record every model call's actual usage, including review and revision; unknowns stay null. Never release an evaluation episode."
            requests.append(request)
    write(output / "requests.json", requests)
    write(output / "candidate-template.json", [{"case_id": r["case_id"], "condition": r["condition"], "text": "",
        "model": "", "model_version": "", "input_tokens": None, "output_tokens": None, "elapsed_seconds": None} for r in requests])


def valid_number(value, name, integer=False):
    if value is None:
        return
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0 or (integer and not isinstance(value, int)):
        raise ValueError(f"{name} must be a nonnegative {'integer' if integer else 'number'} or null")


def normalize_pairwise_input(source, briefs_by_id=None):
    """Normalize either pair-record list or flat (pair_id/case_id, condition) candidate list into pair records."""
    if not isinstance(source, list) or not source:
        raise ValueError("Pairwise input must be a non-empty list")
    briefs_by_id = briefs_by_id or {}
    if "baseline" in source[0] and "candidate" in source[0]:
        pairs = []
        for idx, item in enumerate(source, start=1):
            case_id = item.get("case_id") or item.get("brief_id")
            if not case_id:
                raise ValueError("Each pair record requires case_id or brief_id")
            pair_id = item.get("pair_id") or f"P{idx:03d}"
            brief = briefs_by_id.get(case_id, {})
            base_dict = dict(item["baseline"])
            cand_dict = dict(item["candidate"])
            base_dict.setdefault("case_id", case_id)
            cand_dict.setdefault("case_id", case_id)
            pair_entry = {
                "pair_id": pair_id,
                "case_id": case_id,
                "task": item.get("task") or brief.get("task", ""),
                "contract": item.get("contract") or brief.get("contract", ""),
                "scene_purpose": item.get("scene_purpose") or brief.get("scene_purpose", ""),
                "continuity_checklist": item.get("continuity_checklist") or brief.get("criteria", []),
                "baseline": base_dict,
                "candidate": cand_dict,
            }
            pairs.append(pair_entry)
    else:
        grouped = {}
        order_keys = []
        for row in source:
            cid = row.get("case_id") or row.get("brief_id")
            pid = row.get("pair_id") or cid
            cond = row.get("condition")
            if not pid or cond not in PAIRWISE_CONDITIONS:
                raise ValueError("Flat pairwise candidates require pair_id/case_id and condition in ('baseline', 'candidate')")
            if pid not in grouped:
                grouped[pid] = {"pair_id": pid, "case_id": cid or pid}
                order_keys.append(pid)
            if cond in grouped[pid]:
                raise ValueError(f"Duplicate condition {cond} for pair {pid}")
            grouped[pid][cond] = dict(row)
        pairs = []
        for pid in order_keys:
            entry = grouped[pid]
            if "baseline" not in entry or "candidate" not in entry:
                raise ValueError(f"Pair {pid} requires both baseline and candidate entries")
            brief = briefs_by_id.get(entry["case_id"], {})
            pairs.append({
                "pair_id": pid,
                "case_id": entry["case_id"],
                "task": entry["baseline"].get("task") or brief.get("task", ""),
                "contract": entry["baseline"].get("contract") or brief.get("contract", ""),
                "scene_purpose": entry["baseline"].get("scene_purpose") or brief.get("scene_purpose", ""),
                "continuity_checklist": entry["baseline"].get("continuity_checklist") or brief.get("criteria", []),
                "baseline": entry["baseline"],
                "candidate": entry["candidate"],
            })
    seen_pids = set()
    flat_candidates = []
    for pair in pairs:
        pid = pair["pair_id"]
        if pid in seen_pids:
            raise ValueError(f"Duplicate pair_id: {pid}")
        seen_pids.add(pid)
        for cond in PAIRWISE_CONDITIONS:
            cand = pair[cond]
            if not str(cand.get("text", "")).strip() or not str(cand.get("model", "")).strip() or not str(cand.get("model_version", "")).strip():
                raise ValueError(f"Complete prose, model and model_version are required for {pid}/{cond}; never score placeholders")
            for field in ["input_tokens", "output_tokens", "elapsed_seconds"]:
                valid_number(cand.get(field), field, integer=field.endswith("tokens"))
            flat_candidates.append({
                "pair_id": pid,
                "case_id": pair["case_id"],
                "condition": cond,
                "text": cand["text"],
                "model": cand["model"],
                "model_version": cand["model_version"],
                "prompt_variant": cand.get("prompt_variant"),
                "input_tokens": cand.get("input_tokens"),
                "output_tokens": cand.get("output_tokens"),
                "elapsed_seconds": cand.get("elapsed_seconds"),
            })
    return pairs, flat_candidates


def blind_pairs(candidates_path, output, seed, briefs_path=None):
    """Pairwise (A/B) blinding workflow for human calibration (~100 pairs or arbitrary N)."""
    source = read(candidates_path) if not isinstance(candidates_path, list) else candidates_path
    briefs_by_id = {}
    if briefs_path and Path(briefs_path).exists():
        raw_briefs = read(briefs_path)
        briefs_list = raw_briefs.get("briefs", raw_briefs) if isinstance(raw_briefs, dict) else raw_briefs
        briefs_by_id = {b["id"]: b for b in briefs_list}
    else:
        default_briefs = HERE / "prose_judge" / "briefs.json"
        if default_briefs.exists():
            raw_briefs = read(default_briefs)
            briefs_list = raw_briefs.get("briefs", raw_briefs) if isinstance(raw_briefs, dict) else raw_briefs
            briefs_by_id = {b["id"]: b for b in briefs_list}
        for c in cases():
            briefs_by_id.setdefault(c["id"], c)
    pairs, flat_candidates = normalize_pairwise_input(source, briefs_by_id)
    output = Path(output)
    if output.exists() and any(output.iterdir()):
        raise ValueError("Use an empty output directory; existing blinded evaluations are immutable inputs")
    output.mkdir(parents=True, exist_ok=True)
    rng = random.Random(seed)
    base_orders = list(itertools.permutations(PAIRWISE_CONDITIONS))
    orders = (base_orders * math.ceil(len(pairs) / len(base_orders)))[:len(pairs)]
    rng.shuffle(orders)
    shuffled_pairs = list(pairs)
    rng.shuffle(shuffled_pairs)
    key, packet, ratings = [], [], []
    for pair, order in zip(shuffled_pairs, orders):
        mapping = dict(zip("AB", order))
        key.append({"pair_id": pair["pair_id"], "case_id": pair["case_id"], "labels": mapping})
        packet.append({
            "pair_id": pair["pair_id"],
            "case_id": pair["case_id"],
            "task": pair["task"],
            "contract": pair["contract"],
            "scene_purpose": pair["scene_purpose"],
            "continuity_checklist": pair["continuity_checklist"],
            "candidates": {label: pair[condition]["text"] for label, condition in mapping.items()},
        })
        ratings.append({
            "pair_id": pair["pair_id"],
            "case_id": pair["case_id"],
            "preferred": [],
            "rationale": "",
            "candidates": {
                label: {
                    "continuity_errors": None,
                    "voice": None,
                    "engagement": None,
                    "revision_minutes": None,
                    "anti_slop_dims": None,
                    "rubric": None,
                }
                for label in "AB"
            },
        })
    write(output / "reviewer-packet.json", packet)
    write(output / "ratings.json", ratings)
    write(output / "private-key.json", {
        "mode": "pairwise",
        "seed": seed,
        "source_hash": digest(flat_candidates),
        "mapping": key,
        "candidates": flat_candidates,
    })


def score_pairs(key_path, ratings_path):
    """Score a pairwise (A/B) blinded human evaluation and return de-blinded pair verdicts."""
    key = read(key_path) if not isinstance(key_path, dict) else key_path
    ratings = read(ratings_path) if not isinstance(ratings_path, list) else ratings_path
    if digest(key["candidates"]) != key["source_hash"]:
        raise ValueError("Candidate provenance hash changed")
    mappings = {row["pair_id"]: row for row in key["mapping"]}
    if len({r.get("pair_id") for r in ratings}) != len(ratings) or {r.get("pair_id") for r in ratings} != set(mappings):
        raise ValueError("Rating pairs must match the blinded packet exactly")
    result = {
        condition: {
            "preference_credit": 0.0,
            "outright_wins": 0,
            "tied_top": 0,
            "continuity_errors": [],
            "voice": [],
            "engagement": [],
            "revision_minutes": [],
            "anti_slop_dims": [],
            "rubric_scores": {},
        }
        for condition in PAIRWISE_CONDITIONS
    }
    ANTI_SLOP_DIM_VALUES = {"present", "absent", "n/a"}
    judged = 0
    deblinded_pairs = []
    for row in ratings:
        pid = row["pair_id"]
        mapping_row = mappings[pid]
        mapping = mapping_row["labels"]
        preferred = row["preferred"]
        if len(set(preferred)) != len(preferred) or not set(preferred) <= set("AB") or set(row["candidates"]) != set("AB"):
            raise ValueError("Pairwise candidate labels must be A, B; preferences cannot repeat")
        human_verdict = None
        if preferred:
            judged += 1
            if len(preferred) == 1:
                human_verdict = mapping[preferred[0]]
            else:
                human_verdict = "tie"
        pair_rubric = {}
        for label, condition in mapping.items():
            if label in preferred:
                result[condition]["preference_credit"] += 1 / len(preferred)
                result[condition]["outright_wins" if len(preferred) == 1 else "tied_top"] += 1
            for metric in ["continuity_errors", "voice", "engagement", "revision_minutes"]:
                value = row["candidates"][label].get(metric)
                valid_number(value, metric, integer=metric != "revision_minutes")
                if metric in ("voice", "engagement") and value is not None and not 1 <= value <= 5:
                    raise ValueError(f"{metric} must be 1–5 or null")
                if value is not None:
                    result[condition][metric].append(value)
            dims = row["candidates"][label].get("anti_slop_dims")
            if dims is not None:
                if not isinstance(dims, dict):
                    raise ValueError("anti_slop_dims must be an object of labeled observations or null")
                for dim, value in dims.items():
                    if value not in ANTI_SLOP_DIM_VALUES:
                        raise ValueError(
                            "anti_slop_dims values must be present|absent|n/a; "
                            "they are editorial labels, not a quality score"
                        )
                result[condition]["anti_slop_dims"].append(dims)
            rubric = row["candidates"][label].get("rubric")
            if rubric is not None:
                if not isinstance(rubric, dict):
                    raise ValueError("rubric must be an object of 1-5 dimension scores or null")
                pair_rubric[condition] = {}
                for dim_name, score_val in rubric.items():
                    valid_number(score_val, f"rubric.{dim_name}", integer=True)
                    if score_val is not None:
                        if not 1 <= score_val <= 5:
                            raise ValueError(f"rubric.{dim_name} must be 1–5 or null")
                        result[condition]["rubric_scores"].setdefault(dim_name, []).append(score_val)
                        pair_rubric[condition][dim_name] = score_val
        deblinded_pairs.append({
            "pair_id": pid,
            "case_id": mapping_row["case_id"],
            "human_verdict": human_verdict,
            "rationale": row.get("rationale", ""),
            "rubric": pair_rubric if pair_rubric else None,
        })
    for condition, data in result.items():
        for metric in ["continuity_errors", "voice", "engagement", "revision_minutes"]:
            values = data[metric]
            data[metric] = {"rated": len(values), "mean": statistics.mean(values) if values else None}
        labeled = data.pop("anti_slop_dims")
        data["anti_slop_dims"] = {
            "labeled": len(labeled),
            "note": "Labeled editorial observations only. Not a quality superiority claim.",
        }
        rubric_raw = data.pop("rubric_scores")
        data["rubric"] = {
            dim_name: {"rated": len(vals), "mean": statistics.mean(vals) if vals else None}
            for dim_name, vals in sorted(rubric_raw.items())
        }
        costs = [c for c in key["candidates"] if c["condition"] == condition]
        for metric in ["input_tokens", "output_tokens", "elapsed_seconds"]:
            values = [c.get(metric) for c in costs if c.get(metric) is not None]
            data[metric] = {"known_total": sum(values), "unknown_candidates": len(costs) - len(values)}
    return {
        "mode": "pairwise",
        "preference_pairs_rated": judged,
        "preference_pairs_unrated": len(ratings) - judged,
        "conditions": result,
        "deblinded_pairs": deblinded_pairs,
        "interpretation": (
            "Descriptive pairwise human results only. Report missing judgments, model versions "
            "and human-vs-model rater provenance; this is not proof of universal superiority."
        ),
    }


def blind(candidates_path, output, seed, pairwise=False, briefs_path=None):
    source = read(candidates_path)
    if pairwise or (isinstance(source, list) and source and ("baseline" in source[0] or source[0].get("condition") in PAIRWISE_CONDITIONS)):
        return blind_pairs(source, output, seed, briefs_path=briefs_path)
    all_cases = cases()
    expected = {(c["id"], condition) for c in all_cases for condition in CONDITIONS}
    candidates = {(c["case_id"], c["condition"]): c for c in source}
    if len(candidates) != len(source) or set(candidates) != expected:
        raise ValueError("Exactly one candidate for every case and condition is required")
    for candidate in source:
        if not candidate["text"].strip() or not candidate["model"].strip() or not candidate["model_version"].strip():
            raise ValueError("Complete prose, model and model_version are required; never score placeholders")
        for field in ["input_tokens", "output_tokens", "elapsed_seconds"]:
            valid_number(candidate.get(field), field, integer=field.endswith("tokens"))
    output = Path(output)
    if output.exists() and any(output.iterdir()):
        raise ValueError("Use an empty output directory; existing blinded evaluations are immutable inputs")
    output.mkdir(parents=True, exist_ok=True)
    rng = random.Random(seed)
    orders = list(itertools.permutations(CONDITIONS)) * math.ceil(len(all_cases) / 6)
    rng.shuffle(orders)
    rng.shuffle(all_cases)
    key, packet, ratings = [], [], []
    for case, order in zip(all_cases, orders):
        mapping = dict(zip("ABC", order))
        key.append({"case_id": case["id"], "labels": mapping})
        packet.append({"case_id": case["id"], "task": case["task"], "contract": case["contract"],
            "continuity_checklist": case["criteria"], "candidates": {label: candidates[case["id"], condition]["text"] for label, condition in mapping.items()}})
        ratings.append({"case_id": case["id"], "preferred": [], "candidates": {label: {"continuity_errors": None,
            "voice": None, "engagement": None, "revision_minutes": None,
            "anti_slop_dims": None} for label in "ABC"}})
    write(output / "reviewer-packet.json", packet)
    write(output / "ratings.json", ratings)
    write(output / "private-key.json", {"seed": seed, "source_hash": digest(source), "mapping": key, "candidates": source})


def score(key_path, ratings_path):
    key, ratings = read(key_path), read(ratings_path)
    if key.get("mode") == "pairwise":
        return score_pairs(key, ratings)
    if digest(key["candidates"]) != key["source_hash"]:
        raise ValueError("Candidate provenance hash changed")
    mappings = {row["case_id"]: row["labels"] for row in key["mapping"]}
    if len({r["case_id"] for r in ratings}) != len(ratings) or {r["case_id"] for r in ratings} != set(mappings):
        raise ValueError("Rating cases must match the blinded packet exactly")
    result = {condition: {"preference_credit": 0.0, "outright_wins": 0, "tied_top": 0,
                         "continuity_errors": [], "voice": [], "engagement": [], "revision_minutes": [],
                         "anti_slop_dims": []} for condition in CONDITIONS}
    ANTI_SLOP_DIM_VALUES = {"present", "absent", "n/a"}
    judged = 0
    for row in ratings:
        mapping, preferred = mappings[row["case_id"]], row["preferred"]
        if len(set(preferred)) != len(preferred) or not set(preferred) <= set("ABC") or set(row["candidates"]) != set("ABC"):
            raise ValueError("Candidate labels must be A, B, C; preferences cannot repeat")
        if preferred:
            judged += 1
        for label, condition in mapping.items():
            if label in preferred:
                result[condition]["preference_credit"] += 1 / len(preferred)
                result[condition]["outright_wins" if len(preferred) == 1 else "tied_top"] += 1
            for metric in ["continuity_errors", "voice", "engagement", "revision_minutes"]:
                value = row["candidates"][label].get(metric)
                valid_number(value, metric, integer=metric != "revision_minutes")
                if metric in ("voice", "engagement") and value is not None and not 1 <= value <= 5:
                    raise ValueError(f"{metric} must be 1–5 or null")
                if value is not None:
                    result[condition][metric].append(value)
            dims = row["candidates"][label].get("anti_slop_dims")
            if dims is not None:
                if not isinstance(dims, dict):
                    raise ValueError("anti_slop_dims must be an object of labeled observations or null")
                for dim, value in dims.items():
                    if value not in ANTI_SLOP_DIM_VALUES:
                        raise ValueError(
                            "anti_slop_dims values must be present|absent|n/a; "
                            "they are editorial labels, not a quality score"
                        )
                result[condition]["anti_slop_dims"].append(dims)
    for condition, data in result.items():
        for metric in ["continuity_errors", "voice", "engagement", "revision_minutes"]:
            values = data[metric]
            data[metric] = {"rated": len(values), "mean": statistics.mean(values) if values else None}
        labeled = data.pop("anti_slop_dims")
        data["anti_slop_dims"] = {
            "labeled": len(labeled),
            "note": "Labeled editorial observations only. Not a quality superiority claim.",
        }
        costs = [c for c in key["candidates"] if c["condition"] == condition]
        for metric in ["input_tokens", "output_tokens", "elapsed_seconds"]:
            values = [c.get(metric) for c in costs if c.get(metric) is not None]
            data[metric] = {"known_total": sum(values), "unknown_candidates": len(costs) - len(values)}
    return {"preference_cases_rated": judged, "preference_cases_unrated": len(ratings) - judged,
            "conditions": result, "interpretation": "Descriptive results only. Report missing judgments, model versions and human-vs-model rater provenance; this is not proof of superiority. Optional anti_slop_dims are labeled observations (present/absent/n/a), not a quality ranking."}


def self_test():
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        candidates = [{"case_id": c["id"], "condition": condition, "text": f"Original test passage {c['id']}.",
            "model": "test-only", "model_version": "fixture", "input_tokens": 0, "output_tokens": None, "elapsed_seconds": None}
            for c in cases() for condition in CONDITIONS]
        write(root / "candidates.json", candidates)
        blind(root / "candidates.json", root / "one", 19)
        blind(root / "candidates.json", root / "two", 19)
        key = read(root / "one/private-key.json")
        assert key == read(root / "two/private-key.json")
        counts = {(label, condition): 0 for label in "ABC" for condition in CONDITIONS}
        for mapping in key["mapping"]:
            for label, condition in mapping["labels"].items():
                counts[label, condition] += 1
        assert len(set(counts.values())) == 1, counts
        packet = (root / "one/reviewer-packet.json").read_text()
        assert all(condition not in packet for condition in CONDITIONS) and "test-only" not in packet
        ratings = read(root / "one/ratings.json")
        ratings[0]["preferred"] = ["A", "B"]
        ratings[0]["candidates"]["A"]["continuity_errors"] = 0
        write(root / "one/ratings.json", ratings)
        result = score(root / "one/private-key.json", root / "one/ratings.json")
        assert result["preference_cases_rated"] == 1 and result["preference_cases_unrated"] == 11
        assert sum(v["preference_credit"] for v in result["conditions"].values()) == 1
        assert all(v["input_tokens"]["unknown_candidates"] == 0 and v["output_tokens"]["unknown_candidates"] == 12 for v in result["conditions"].values())
        assert sum(v["continuity_errors"]["rated"] for v in result["conditions"].values()) == 1
        for bad in [-1, float("nan"), True, "0"]:
            try:
                valid_number(bad, "metric")
                raise AssertionError("accepted invalid metric")
            except ValueError:
                pass
        # Pairwise (A/B) 100-pair calibration workflow check
        pair_candidates = [
            {
                "pair_id": f"P{i:03d}",
                "case_id": f"S{(i - 1) % 12 + 1:02d}",
                "condition": cond,
                "text": f"Pairwise passage {i} ({cond}).",
                "model": "test-only",
                "model_version": "fixture",
                "input_tokens": 100,
                "output_tokens": None,
                "elapsed_seconds": None,
            }
            for i in range(1, 101)
            for cond in PAIRWISE_CONDITIONS
        ]
        write(root / "pair_candidates.json", pair_candidates)
        blind(root / "pair_candidates.json", root / "pairs_blind", 42, pairwise=True)
        pkey = read(root / "pairs_blind/private-key.json")
        assert pkey["mode"] == "pairwise" and len(pkey["mapping"]) == 100
        pcounts = {(label, cond): 0 for label in "AB" for cond in PAIRWISE_CONDITIONS}
        for m in pkey["mapping"]:
            for label, cond in m["labels"].items():
                pcounts[label, cond] += 1
        assert all(v == 50 for v in pcounts.values()), pcounts
        ppacket = (root / "pairs_blind/reviewer-packet.json").read_text()
        assert "test-only" not in ppacket and '"baseline"' not in ppacket
        pratings = read(root / "pairs_blind/ratings.json")
        pratings[0]["preferred"] = ["A"]
        pratings[1]["preferred"] = ["A", "B"]
        write(root / "pairs_blind/ratings.json", pratings)
        presult = score(root / "pairs_blind/private-key.json", root / "pairs_blind/ratings.json")
        assert presult["preference_pairs_rated"] == 2 and presult["preference_pairs_unrated"] == 98
        assert sum(v["preference_credit"] for v in presult["conditions"].values()) == 2.0
    print("Blinding, balance, ties, missing ratings, zero/unknown usage and validation checks passed.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("collect", help="Create new EVAL fixture projects and capture real context; use a disposable server")
    p.add_argument("--server", required=True); p.add_argument("--out", required=True)
    p = sub.add_parser("prepare"); p.add_argument("--captures", required=True); p.add_argument("--out", required=True)
    p = sub.add_parser("blind"); p.add_argument("--candidates", required=True); p.add_argument("--out", required=True); p.add_argument("--seed", type=int, default=20260905); p.add_argument("--pairwise", action="store_true"); p.add_argument("--briefs", default=None)
    p = sub.add_parser("score"); p.add_argument("--key", required=True); p.add_argument("--ratings", required=True)
    sub.add_parser("self-test")
    args = parser.parse_args()
    if args.command == "collect": collect(args.server, args.out)
    elif args.command == "prepare": prepare(args.captures, args.out)
    elif args.command == "blind": blind(args.candidates, args.out, args.seed, pairwise=args.pairwise, briefs_path=args.briefs)
    elif args.command == "score": print(json.dumps(score(args.key, args.ratings), indent=2))
    else: self_test()


if __name__ == "__main__":
    main()

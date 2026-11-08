#!/usr/bin/env python3
"""Pairwise LLM-judge evaluation kit for Spindle prose quality (Task F).

Commands:
  self-test     Run offline unit tests (30-brief suite, double-judging, position bias,
                bootstrap CIs, inter-judge kappa, generation adapter, 100-pair human
                calibration workflow, and API key redaction).
  check-keys    Report which environment API keys are set or missing (never prints values).
  generate      Produce candidate prose outputs from saved files or by calling a configured
                model with a prompt variant (e.g. compact_baseline vs spindle_full_context).
  judge         Run randomized + swapped-order double judging across multiple judge families.
  blind         Create a blinded human-rating packet (~100 pairs) reusing serial_fiction.py.
  calibrate     Compute judge-human agreement against a scored human calibration packet.

Standard library only. Never prints or logs API keys.
"""
from __future__ import annotations

import argparse
from collections import Counter
import concurrent.futures
import hashlib
import itertools
import json
import math
import os
from pathlib import Path
import random
import re
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

import serial_fiction  # noqa: E402

BRIEFS_PATH = HERE / "prose_judge" / "briefs.json"
FIXTURE_PAIRS_PATH = HERE / "prose_judge" / "fixture_pairs.json"
JUDGES_PATH = HERE / "prose_judge" / "judges.json"
VARIANTS_PATH = HERE / "prose_judge" / "prompt_variants.json"
RESULTS_DIR = HERE / "results"

SCENE_TYPES = (
    "action",
    "dialogue-heavy",
    "quiet-interior",
    "reveal",
    "chapter-closer",
)

RUBRIC_DIMENSIONS = (
    "scene_purpose_fulfilled",
    "tension_turn",
    "pov_discipline",
    "voice_distinctiveness",
    "dialogue",
    "prose_freshness",
    "continuity_with_brief",
)

VERDICT_LABELS = ("baseline", "candidate", "tie")
PASSAGE_LABELS = ("passage_1", "passage_2", "tie")

SLOP_PATTERNS = (
    re.compile(r"\bit wasn't\b[^.?!]{1,60}\bit was\b", re.I),
    re.compile(r"\b(?:a\s+)?(?:mix|cocktail|blend)\s+of\s+\w+\s+and\s+\w+", re.I),
    re.compile(r"\bdidn't know what tomorrow would bring\b", re.I),
    re.compile(r"\bhours passed\b", re.I),
    re.compile(r'\b(?:hissed|gasped|breathed)\b', re.I),
    re.compile(r"\b(?:jaw tightened|stomach dropped|heart hammered)\b", re.I),
)


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


def redact_secrets(text: str, env_names: list[str] | tuple[str, ...] = ()) -> str:
    """Scrub any environment secret values that might appear in an exception or message."""
    if not text:
        return text
    cleaned = str(text)
    check_names = set(env_names) | {
        k for k in os.environ if k.endswith("_KEY") or k.endswith("_TOKEN") or k.endswith("_SECRET")
    }
    for name in check_names:
        val = os.environ.get(name)
        if val and len(val) >= 4 and val in cleaned:
            cleaned = cleaned.replace(val, "[REDACTED]")
    return cleaned


def load_briefs(path: Path | str = BRIEFS_PATH) -> list[dict]:
    data = read_json(path)
    return data["briefs"] if isinstance(data, dict) and "briefs" in data else data


def validate_brief_suite(briefs: list[dict]) -> list[str]:
    """Validate the 30-brief suite schema, coverage, and drafter context."""
    errors: list[str] = []
    if len(briefs) < 25 or len(briefs) > 35:
        errors.append(f"Expected ~30 scene briefs, got {len(briefs)}")
    seen_ids: set[str] = set()
    scene_type_counts: Counter[str] = Counter()
    genres: set[str] = set()
    pov_perspectives: set[str] = set()

    for b in briefs:
        bid = b.get("id")
        if not bid:
            errors.append("Brief missing 'id'")
            continue
        if bid in seen_ids:
            errors.append(f"Duplicate brief id: {bid}")
        seen_ids.add(bid)

        stype = b.get("scene_type")
        if stype not in SCENE_TYPES:
            errors.append(f"{bid}: invalid scene_type {stype!r}, expected one of {SCENE_TYPES}")
        else:
            scene_type_counts[stype] += 1

        genre = b.get("genre")
        if not genre:
            errors.append(f"{bid}: missing genre")
        else:
            genres.add(genre)

        pov = b.get("pov") or {}
        if not pov.get("character") or not pov.get("perspective") or not pov.get("voice_profile"):
            errors.append(f"{bid}: pov requires character, perspective, and voice_profile")
        else:
            pov_perspectives.add(pov["perspective"])

        for req_field in ("name", "project_id", "contract", "scene_purpose", "tension_turn", "task"):
            if not str(b.get(req_field, "")).strip():
                errors.append(f"{bid}: missing required field {req_field!r}")

        criteria = b.get("criteria") or []
        if not isinstance(criteria, list) or len(criteria) < 3:
            errors.append(f"{bid}: criteria must be a list of at least 3 items")

        ctx = b.get("drafter_context") or {}
        for ctx_field in (
            "location",
            "in_world_time",
            "hard_constraints",
            "characters_present",
            "previous_scene_tail",
            "context_markdown",
        ):
            if not ctx.get(ctx_field):
                errors.append(f"{bid}: drafter_context missing {ctx_field!r}")

    for stype in SCENE_TYPES:
        if scene_type_counts[stype] < 3:
            errors.append(f"Insufficient coverage for scene_type {stype!r}: {scene_type_counts[stype]}")
    if len(genres) < 4:
        errors.append(f"Suite must span varied genres, found only {sorted(genres)}")
    if len(pov_perspectives) < 2:
        errors.append(f"Suite must span varied POV perspectives, found only {sorted(pov_perspectives)}")
    return errors


def check_keys(judges_path: Path | str = JUDGES_PATH, variants_path: Path | str = VARIANTS_PATH) -> dict:
    """Inspect required API key env vars without ever reading or printing their values."""
    jcfg = read_json(judges_path)
    required_keys: dict[str, list[str]] = {}
    for group in ("live_judges", "alternate_provider_judges"):
        for judge in jcfg.get(group, []):
            key_env = judge.get("api_key_env")
            if key_env:
                required_keys.setdefault(key_env, []).append(f"judge:{judge['id']}")
    if Path(variants_path).exists():
        vcfg = read_json(variants_path)
        gen = vcfg.get("default_generator") or {}
        if gen.get("api_key_env"):
            required_keys.setdefault(gen["api_key_env"], []).append("generator:default")

    primary_live_keys = sorted({
        j["api_key_env"]
        for j in jcfg.get("live_judges", [])
        if j.get("api_key_env")
    })
    status = {}
    for key_env, users in sorted(required_keys.items()):
        is_set = bool(os.environ.get(key_env))
        status[key_env] = {
            "set": is_set,
            "used_by": users,
        }
    missing_primary = [k for k in primary_live_keys if not status[k]["set"]]
    key_arg = ",".join(primary_live_keys) if primary_live_keys else "OLLAMA_API_KEY"
    return {
        "primary_live_keys": primary_live_keys,
        "missing_primary_keys": missing_primary,
        "all_primary_keys_set": len(missing_primary) == 0,
        "keys": status,
        "recommended_invocation": (
            f"ai run {key_arg} -- python3 evals/prose_judge.py judge "
            f"--live --out evals/results/prose_judge_live.json"
        ),
    }


def extract_json_object(raw_text: str) -> dict:
    """Extract and parse a JSON object from model output, stripping <think> blocks and fences."""
    cleaned = re.sub(r"<think>.*?</think>", "", raw_text, flags=re.S | re.I).strip()
    if "</think>" in cleaned.lower():
        cleaned = re.split(r"</think>", cleaned, flags=re.I)[-1].strip()
    fence_match = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", cleaned, flags=re.S | re.I)
    if fence_match:
        cleaned = fence_match.group(1).strip()
    else:
        start = cleaned.find("{")
        end = cleaned.rfind("}")
        if start != -1 and end != -1 and end > start:
            cleaned = cleaned[start : end + 1]
    # Remove trailing commas before } or ] that some LLMs emit
    cleaned = re.sub(r",\s*([}\]])", r"\1", cleaned)
    return json.loads(cleaned)


def normalize_preference_token(token: str) -> str:
    t = str(token).strip().lower().replace("-", "_").replace(" ", "_")
    if t in ("passage_1", "passage1", "1", "a", "option_1", "first"):
        return "passage_1"
    if t in ("passage_2", "passage2", "2", "b", "option_2", "second"):
        return "passage_2"
    if t in ("tie", "tied", "equal", "neither", "draw"):
        return "tie"
    raise ValueError(f"Invalid preference token: {token!r}; expected passage_1, passage_2, or tie")


def validate_judge_response(payload: dict) -> dict:
    """Validate and normalize structured rubric output from a judge model."""
    if not isinstance(payload, dict):
        raise ValueError("Judge output must be a JSON object")
    raw_rubric = payload.get("rubric")
    if not isinstance(raw_rubric, dict):
        raise ValueError("Judge output missing 'rubric' object")

    normalized_rubric = {}
    for dim in RUBRIC_DIMENSIONS:
        if dim not in raw_rubric:
            raise ValueError(f"Judge rubric missing dimension {dim!r}")
        entry = raw_rubric[dim]
        if not isinstance(entry, dict):
            raise ValueError(f"Rubric dimension {dim!r} must be an object")
        def _coerce_score(val, field_label: str) -> int:
            if val is None or (isinstance(val, str) and val.strip().upper() in ("N/A", "NA", "NONE")):
                if dim == "dialogue":
                    return 3
                raise ValueError(f"{field_label} must be a number 1..5, got {val!r}")
            if isinstance(val, bool):
                raise ValueError(f"{field_label} must be a number 1..5, got {val!r}")
            if isinstance(val, str):
                try:
                    val = float(val.strip())
                except ValueError:
                    raise ValueError(f"{field_label} must be a number 1..5, got {val!r}") from None
            if not isinstance(val, (int, float)) or not (0 <= float(val) <= 5):
                raise ValueError(f"{field_label} must be a number 1..5, got {val!r}")
            return max(1, min(5, int(round(float(val)))))

        s1 = _coerce_score(
            entry.get("passage_1_score", entry.get("score_1")),
            f"{dim}.passage_1_score",
        )
        s2 = _coerce_score(
            entry.get("passage_2_score", entry.get("score_2")),
            f"{dim}.passage_2_score",
        )
        winner_raw = entry.get("winner")
        if winner_raw is None:
            winner = "passage_1" if s1 > s2 else ("passage_2" if s2 > s1 else "tie")
        else:
            winner = normalize_preference_token(winner_raw)
        note = str(entry.get("note", "")).strip()
        if not note:
            raise ValueError(f"{dim}.note must be a non-empty string")
        normalized_rubric[dim] = {
            "passage_1_score": s1,
            "passage_2_score": s2,
            "winner": winner,
            "note": note,
        }

    overall_raw = payload.get("overall_preference")
    if overall_raw is None:
        raise ValueError("Judge output missing 'overall_preference'")
    overall = normalize_preference_token(overall_raw)
    rationale = str(payload.get("rationale", "")).strip()
    if not rationale:
        raise ValueError("Judge output missing non-empty 'rationale'")

    return {
        "rubric": normalized_rubric,
        "overall_preference": overall,
        "rationale": rationale,
    }


def build_judge_prompts(brief: dict, passage_1_text: str, passage_2_text: str) -> tuple[str, str]:
    """Build system and user prompts for blinded pairwise judging (Passage 1 vs Passage 2)."""
    system_prompt = (
        "You are an expert literary editor and continuity auditor evaluating two blinded "
        "scene drafts (Passage 1 and Passage 2) written for the same scene brief.\n"
        "Evaluate both passages strictly against the supplied brief, reader contract, POV "
        "discipline, voice profile, hard continuity constraints, and prose craft.\n"
        "Do NOT favor a passage merely because it is longer or uses melodramatic clichés. "
        "Penalize prefab AI slop phrases ('It wasn't X, it was Y', 'a mix/cocktail of X and Y', "
        "'didn't know what tomorrow would bring', 'hours passed', said-bookism clusters like "
        "'hissed/gasped/breathed'), POV violations, future-knowledge leaks, and broken world rules.\n"
        "Return ONLY a single valid JSON object matching the exact schema requested, with no "
        "markdown commentary outside the JSON. Every rubric dimension MUST include integer "
        "passage_1_score and passage_2_score in 1..5 (never null or N/A; if a brief forbids "
        "spoken dialogue, score dialogue 5 when silence is respected and 1 when violated)."
    )
    ctx_md = brief.get("drafter_context", {}).get("context_markdown", "")
    criteria_lines = "\n".join(f"- {c}" for c in brief.get("criteria", []))
    user_prompt = (
        f"## Scene Brief ({brief['id']}: {brief['name']})\n"
        f"- Genre: {brief['genre']}\n"
        f"- Scene Type: {brief['scene_type']}\n"
        f"- POV: {brief['pov']['character']} ({brief['pov']['perspective']}) — {brief['pov']['voice_profile']}\n"
        f"- Reader Contract: {brief['contract']}\n"
        f"- Scene Purpose: {brief['scene_purpose']}\n"
        f"- Required Dramatic Turn: {brief['tension_turn']}\n"
        f"- Task: {brief['task']}\n\n"
        f"### Continuity & Craft Checklist\n{criteria_lines}\n\n"
        f"### Drafter Context Packet\n{ctx_md}\n\n"
        f"---\n"
        f"## Passage 1\n{passage_1_text}\n\n"
        f"---\n"
        f"## Passage 2\n{passage_2_text}\n\n"
        f"---\n"
        "Return ONLY a JSON object with this exact structure:\n"
        "{\n"
        '  "rubric": {\n'
        '    "scene_purpose_fulfilled": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "tension_turn": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "pov_discipline": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "voice_distinctiveness": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "dialogue": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "prose_freshness": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"},\n'
        '    "continuity_with_brief": {"passage_1_score": <1-5>, "passage_2_score": <1-5>, "winner": "passage_1"|"passage_2"|"tie", "note": "<brief specific reason>"}\n'
        "  },\n"
        '  "overall_preference": "passage_1" | "passage_2" | "tie",\n'
        '  "rationale": "<2-3 sentences citing concrete textual evidence for the preference>"\n'
        "}"
    )
    return system_prompt, user_prompt


def _score_passage_heuristically(brief: dict, text: str, profile: str = "structural") -> dict[str, int]:
    """Deterministic feature scorer used by the offline fake judge."""
    slop_hits = sum(1 for pat in SLOP_PATTERNS if pat.search(text))
    words = text.split()
    word_count = len(words)
    paragraphs = [p.strip() for p in text.split("\n\n") if p.strip()]

    # Check POV discipline
    perspective = brief.get("pov", {}).get("perspective", "third_limited")
    first_person_pronouns = len(re.findall(r"(?:^|\s)(?:I|my|me|myself)(?:\s|[,.?!'])", re.sub(r"'.*?'|\".*?\"", "", text)))
    omniscient_spoilers = bool(re.search(r"\b(?:narrator|in Chapter \d+|would later confess|secretly unknown to)\b", text, re.I))
    pov_ok = True
    if perspective == "first_person" and first_person_pronouns == 0:
        pov_ok = False
    elif perspective in ("third_limited", "close_third") and first_person_pronouns > 2:
        pov_ok = False
    if omniscient_spoilers:
        pov_ok = False

    # Check continuity anchors from brief criteria, location, and constraints
    loc_obj = brief.get("drafter_context", {}).get("location", {})
    loc_words = [
        w.lower()
        for w in re.findall(r"[A-Za-z]{3,}", f"{loc_obj.get('name', '')} {loc_obj.get('summary', '')}")
    ]
    crit_text = " ".join(brief.get("criteria", [])) + " " + brief.get("scene_purpose", "")
    key_terms = {
        w.lower()
        for w in re.findall(r"[A-Za-z]{5,}", crit_text)
        if w.lower() not in {"scene", "write", "chapter", "character", "without", "through", "their", "before", "after", "while"}
    }
    text_lower = text.lower()
    term_hits = sum(1 for w in key_terms if w in text_lower)
    loc_hit = any(w in text_lower for w in loc_words) if loc_words else True

    # Violations of explicit negative constraints
    rule_violation = bool(
        re.search(
            r"\b(?:giant sea monster|laser pistol|million credits|ten thousand gold|Gold-9 tier|muahaha|poisoned crossbows)\b",
            text,
            re.I,
        )
    )
    if brief["id"] == "PJ03" and re.search(r"""['"][^'"]+['"]\s*Mara\s+(?:said|replied|asked)""", text):
        rule_violation = True

    freshness = max(1, min(5, 5 - slop_hits * 2))
    pov_score = 5 if (pov_ok and not slop_hits) else (4 if pov_ok else 1)
    continuity = 5 if (not rule_violation and term_hits >= 3 and loc_hit) else (3 if not rule_violation and term_hits >= 1 else 1)
    purpose = 5 if (continuity >= 4 and word_count >= 90 and len(paragraphs) >= 2 and slop_hits == 0) else (2 if slop_hits >= 2 or rule_violation else 3)
    tension = 5 if (purpose >= 4 and freshness >= 4) else (2 if slop_hits >= 2 or rule_violation else 3)
    dialogue_quotes = len(re.findall(r"['\"].+?['\"]", text))
    if brief["scene_type"] == "dialogue-heavy":
        dialogue = 5 if (dialogue_quotes >= 4 and slop_hits == 0 and not rule_violation) else 2
    else:
        dialogue = 4 if (slop_hits == 0 and not rule_violation) else 2
    voice = 5 if (pov_score >= 4 and freshness >= 4 and not rule_violation) else 2

    if profile == "voice":
        voice = min(5, voice + (1 if freshness == 5 and len(paragraphs) >= 3 else 0))
    elif profile == "continuity":
        continuity = min(5, continuity + (1 if term_hits >= 5 and not rule_violation else 0))

    return {
        "scene_purpose_fulfilled": purpose,
        "tension_turn": tension,
        "pov_discipline": pov_score,
        "voice_distinctiveness": voice,
        "dialogue": dialogue,
        "prose_freshness": freshness,
        "continuity_with_brief": continuity,
    }


def run_fake_judge(
    judge_cfg: dict,
    brief: dict,
    passage_1_text: str,
    passage_2_text: str,
) -> dict:
    """Offline deterministic judge supporting unbiased, family-weighted, and position-biased profiles."""
    profile = judge_cfg.get("profile", "structural")
    s1 = _score_passage_heuristically(brief, passage_1_text, profile=profile)
    s2 = _score_passage_heuristically(brief, passage_2_text, profile=profile)

    if profile == "position_biased_first":
        for d in RUBRIC_DIMENSIONS:
            s1[d] = min(5, s1[d] + 2)
            s2[d] = max(1, s2[d] - 1)
    elif profile == "position_biased_second":
        for d in RUBRIC_DIMENSIONS:
            s2[d] = min(5, s2[d] + 2)
            s1[d] = max(1, s1[d] - 1)

    rubric = {}
    weights = {d: 1.0 for d in RUBRIC_DIMENSIONS}
    if profile == "structural":
        weights["scene_purpose_fulfilled"] = 1.5
        weights["tension_turn"] = 1.5
    elif profile == "voice":
        weights["voice_distinctiveness"] = 1.5
        weights["prose_freshness"] = 1.5
        weights["dialogue"] = 1.3
    elif profile == "continuity":
        weights["continuity_with_brief"] = 1.8
        weights["pov_discipline"] = 1.4

    total_1 = 0.0
    total_2 = 0.0
    for dim in RUBRIC_DIMENSIONS:
        v1, v2 = s1[dim], s2[dim]
        w = "passage_1" if v1 > v2 else ("passage_2" if v2 > v1 else "tie")
        rubric[dim] = {
            "passage_1_score": v1,
            "passage_2_score": v2,
            "winner": w,
            "note": f"Offline {profile} evaluation on {dim}: P1={v1}, P2={v2}.",
        }
        total_1 += v1 * weights[dim]
        total_2 += v2 * weights[dim]

    if profile == "position_biased_first":
        overall = "passage_1"
    elif profile == "position_biased_second":
        overall = "passage_2"
    elif abs(total_1 - total_2) <= 1.25:
        overall = "tie"
    elif total_1 > total_2:
        overall = "passage_1"
    else:
        overall = "passage_2"

    return {
        "rubric": rubric,
        "overall_preference": overall,
        "rationale": (
            f"Offline judge ({judge_cfg['id']}, family={judge_cfg.get('family', 'synthetic')}) "
            f"weighted score P1={total_1:.1f} vs P2={total_2:.1f} -> {overall}."
        ),
    }


def _http_post_json(url: str, payload: dict, headers: dict, timeout: int, key_env: str | None) -> dict:
    body = json.dumps(payload).encode("utf-8")
    last_err: str | None = None
    for attempt in range(4):
        req = urllib.request.Request(url, data=body, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                raw = resp.read().decode("utf-8")
                return json.loads(raw)
        except urllib.error.HTTPError as exc:
            err_body = ""
            try:
                err_body = exc.read().decode("utf-8", errors="replace")[:600]
            except Exception:
                pass
            last_err = redact_secrets(f"HTTP {exc.code} from {url}: {err_body}", [key_env] if key_env else [])
            if exc.code in (429, 500, 502, 503, 504) and attempt < 3:
                time.sleep(1.5 * (2 ** attempt))
                continue
            raise RuntimeError(last_err) from None
        except Exception as exc:
            last_err = redact_secrets(f"Request failed for {url}: {exc}", [key_env] if key_env else [])
            if attempt < 3:
                time.sleep(1.5 * (2 ** attempt))
                continue
            raise RuntimeError(last_err) from None
    raise RuntimeError(last_err or f"Request failed for {url}")


def call_model_provider(
    agent_cfg: dict,
    system_prompt: str,
    user_prompt: str,
    timeout: int = 180,
) -> dict:
    """Dispatch a completion request to ollama, anthropic, openai-compatible, or fake provider."""
    provider = str(agent_cfg.get("provider", "fake")).lower()
    model = agent_cfg.get("model", "unknown-model")
    endpoint = str(agent_cfg.get("endpoint", "")).rstrip("/")
    key_env = agent_cfg.get("api_key_env")
    temperature = float(agent_cfg.get("temperature", 0.0))
    max_tokens = int(agent_cfg.get("max_tokens", 2048))

    if provider == "fake":
        raise ValueError("call_model_provider should not be called directly for fake judge; use run_fake_judge")

    api_key = os.environ.get(key_env, "") if key_env else ""
    if key_env and not api_key:
        raise RuntimeError(
            f"Missing required environment variable {key_env} for {agent_cfg.get('id', model)}. "
            f"Run with: ai run {key_env} -- python3 evals/prose_judge.py ..."
        )

    t0 = time.monotonic()

    if provider == "ollama":
        # Ollama Cloud (https://ollama.com) and local Ollama support /api/chat, /v1/chat/completions, and /v1/messages.
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "spindle-prose-judge/1.0",
        }
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"
            headers["x-api-key"] = api_key

        if endpoint.endswith("/v1/messages"):
            url = endpoint
            mode = "anthropic"
        elif endpoint.endswith("/chat/completions"):
            url = endpoint
            mode = "openai"
        elif endpoint.endswith("/api/chat"):
            url = endpoint
            mode = "ollama_native"
        else:
            url = f"{endpoint}/api/chat"
            mode = "ollama_native"

        if mode == "ollama_native":
            options = {
                "temperature": temperature,
                "num_predict": max(max_tokens, 12288 if agent_cfg.get("json_mode", True) else 4096),
            }
            if agent_cfg.get("seed") is not None:
                options["seed"] = int(agent_cfg["seed"])
            payload = {
                "model": model,
                "stream": False,
                "think": False,
                "format": "json" if agent_cfg.get("json_mode", True) else None,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt},
                ],
                "options": options,
            }
            payload = {k: v for k, v in payload.items() if v is not None}
            try:
                resp = _http_post_json(url, payload, headers, timeout, key_env)
                elapsed = round(time.monotonic() - t0, 3)
                msg = resp.get("message") or {}
                text = msg.get("content") or resp.get("response") or ""
                in_tok = resp.get("prompt_eval_count")
                out_tok = resp.get("eval_count")
                return {
                    "text": text,
                    "model": resp.get("model", model),
                    "input_tokens": in_tok if isinstance(in_tok, int) else None,
                    "output_tokens": out_tok if isinstance(out_tok, int) else None,
                    "elapsed_seconds": elapsed,
                }
            except RuntimeError:
                # Fallback to Anthropic-compatible /v1/messages on Ollama Cloud
                url = f"{endpoint}/v1/messages"
                mode = "anthropic"

        if mode == "anthropic":
            headers["anthropic-version"] = "2023-06-01"
            payload = {
                "model": model,
                "max_tokens": max_tokens,
                "temperature": temperature,
                "system": system_prompt,
                "messages": [{"role": "user", "content": user_prompt}],
            }
            resp = _http_post_json(url, payload, headers, timeout, key_env)
            elapsed = round(time.monotonic() - t0, 3)
            blocks = resp.get("content") or []
            text_parts = [b.get("text", "") for b in blocks if isinstance(b, dict) and b.get("type") == "text"]
            usage = resp.get("usage") or {}
            return {
                "text": "\n".join(text_parts).strip(),
                "model": resp.get("model", model),
                "input_tokens": usage.get("input_tokens"),
                "output_tokens": usage.get("output_tokens"),
                "elapsed_seconds": elapsed,
            }

    if provider == "anthropic":
        url = endpoint if endpoint.endswith("/messages") else f"{endpoint}/v1/messages"
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json",
            "anthropic-version": "2023-06-01",
            "User-Agent": "spindle-prose-judge/1.0",
        }
        if api_key:
            headers["x-api-key"] = api_key
            headers["Authorization"] = f"Bearer {api_key}"
        payload = {
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "system": system_prompt,
            "messages": [{"role": "user", "content": user_prompt}],
        }
        resp = _http_post_json(url, payload, headers, timeout, key_env)
        elapsed = round(time.monotonic() - t0, 3)
        blocks = resp.get("content") or []
        text_parts = [b.get("text", "") for b in blocks if isinstance(b, dict) and b.get("type") == "text"]
        usage = resp.get("usage") or {}
        return {
            "text": "\n".join(text_parts).strip(),
            "model": resp.get("model", model),
            "input_tokens": usage.get("input_tokens"),
            "output_tokens": usage.get("output_tokens"),
            "elapsed_seconds": elapsed,
        }

    if provider in ("openai-compatible", "openai"):
        if endpoint.endswith("/chat/completions"):
            url = endpoint
        elif endpoint.endswith("/v1"):
            url = f"{endpoint}/chat/completions"
        else:
            url = f"{endpoint}/v1/chat/completions"
        headers = {
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "spindle-prose-judge/1.0",
        }
        if api_key:
            headers["Authorization"] = f"Bearer {api_key}"
        payload = {
            "model": model,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
        }
        resp = _http_post_json(url, payload, headers, timeout, key_env)
        elapsed = round(time.monotonic() - t0, 3)
        choices = resp.get("choices") or []
        msg = choices[0].get("message", {}) if choices else {}
        usage = resp.get("usage") or {}
        return {
            "text": str(msg.get("content", "")).strip(),
            "model": resp.get("model", model),
            "input_tokens": usage.get("prompt_tokens"),
            "output_tokens": usage.get("completion_tokens"),
            "elapsed_seconds": elapsed,
        }

    raise ValueError(f"Unsupported provider: {provider!r}")


# =============================================================================
# Generation Adapter (Saved files, model + prompt-variant, OR Rust bridge)
# =============================================================================

def _slug(value: str) -> str:
    return re.sub(r"[^a-z0-9]+", "_", str(value).lower()).strip("_") or "item"


def brief_to_drafting_packet_v1(brief: dict) -> dict:
    """Convert an eval brief from briefs.json into a valid DraftingPacketV1 JSON object."""
    bid = str(brief["id"])
    raw_pid = str(brief["project_id"])
    project_id = raw_pid if raw_pid.startswith("project:") else f"project:{raw_pid}"
    branch_id = f"bible_branch:{project_id}:main"
    scene_id = f"scene:{bid}"
    run_id = f"run:{bid}"
    packet_id = f"packet:{bid}"

    pov = brief.get("pov") or {}
    pov_name = str(pov.get("character", "Protagonist"))
    pov_char_id = f"character:{bid}:pov:{_slug(pov_name)}"
    persp = str(pov.get("perspective", "third_limited")).lower()
    person = "first" if "first" in persp else "third"
    tense = "present" if "present" in persp else "past"

    target = brief.get("target") or []
    episodes = brief.get("episodes") or []
    if len(target) >= 3:
        book_num = max(1, int(target[0]))
        chapter_num = max(1, int(target[1]))
        scene_num = max(1, int(target[2]))
    elif episodes:
        latest_ep = episodes[-1]
        book_num = max(1, int(latest_ep.get("book", 1)))
        chapter_num = max(1, int(latest_ep.get("chapter", 1)))
        scene_num = 1
    else:
        book_num = 1
        chapter_num = 1
        scene_num = 1

    ctx = brief.get("drafter_context") or {}
    loc = ctx.get("location") or {}
    raw_hc = ctx.get("hard_constraints") or []
    chars_present = ctx.get("characters_present") or []
    iwt = ctx.get("in_world_time") or {}

    source_versions: list[dict] = []
    seen_sources: set[tuple[str, str, str]] = set()

    def _make_source(source_id: str, payload_for_hash) -> dict:
        rev_hash = digest({"brief_id": bid, "source_id": source_id, "payload": payload_for_hash})[:16]
        src = {
            "branch_id": branch_id,
            "source_id": source_id,
            "revision_hash": rev_hash,
        }
        key = (src["branch_id"], src["source_id"], src["revision_hash"])
        if key not in seen_sources:
            seen_sources.add(key)
            source_versions.append(src)
        return src

    def _make_item(source_id: str, proposition_id: str, text: str, relevance: int = 100, recency: int = 1) -> dict:
        src = _make_source(source_id, text)
        return {
            "source": src,
            "proposition_id": proposition_id,
            "text": text,
            "relevance": int(relevance),
            "recency": int(recency),
        }

    if isinstance(iwt, dict):
        day_index = int(iwt["day"]) if isinstance(iwt.get("day"), int) else None
        tod = str(iwt.get("time_of_day", "")).strip()
        anchor = str(iwt.get("anchor_note", "")).strip()
        story_clock = f"Day {day_index}, {tod}" if day_index is not None else tod
        if anchor:
            story_clock = f"{story_clock} ({anchor})" if story_clock else anchor
    else:
        story_clock = str(iwt).strip() or "Day 1"

    participants = []
    character_states = []
    knowledge_boundaries = []
    voice_rules = [
        _make_item(
            f"voice:pov:{bid}",
            f"prop:voice:pov:{bid}",
            f"POV character & perspective: {pov_name} ({persp}) — {pov.get('voice_profile', '')}",
            relevance=100,
            recency=10,
        ),
        _make_item(
            f"voice:antislop:{bid}",
            f"prop:voice:antislop:{bid}",
            "Avoid prefab AI prose cliches ('It wasn't X, it was Y' contrast hinges, 'a mix/cocktail of X and Y', 'didn't know what tomorrow would bring', 'hours passed', and said-bookism clusters like 'hissed/gasped/breathed').",
            relevance=95,
            recency=9,
        ),
    ]
    withheld_constraints: list[tuple[str, str]] = []
    kb_counter = 0

    for idx, cp in enumerate(chars_present, start=1):
        cname = str(cp.get("name", f"Participant {idx}"))
        cid = pov_char_id if cname == pov_name else f"character:{bid}:{idx}:{_slug(cname)}"
        role = str(cp.get("role", "participant"))
        cstate = str(cp.get("current_state", "")).strip()
        speech = str(cp.get("voice") or cp.get("speech_style") or "").strip()
        knows_list = [str(x).strip() for x in (cp.get("knows") or []) if str(x).strip()]
        dnk_list = [str(x).strip() for x in (cp.get("does_not_know") or []) if str(x).strip()]

        participants.append({
            "id": cid,
            "display_name": cname,
            "role": role,
        })
        if speech:
            voice_rules.append(
                _make_item(
                    f"voice:char:{bid}:{idx}",
                    f"prop:voice:char:{bid}:{idx}",
                    f"{cname} ({role}) voice: {speech}",
                    relevance=90 - idx,
                    recency=8,
                )
            )

        state_parts = [f"{cname} ({role}): {cstate}"]
        if speech:
            state_parts.append(f"Voice: {speech}")
        if knows_list:
            state_parts.append(f"Knows: {'; '.join(knows_list)}")
        if dnk_list:
            state_parts.append(f"Withheld/Does NOT know: {'; '.join(dnk_list)}")
        character_states.append(
            _make_item(
                f"character_state:{bid}:{idx}",
                f"prop:char_state:{bid}:{idx}",
                " | ".join(state_parts),
                relevance=100 - idx,
                recency=10,
            )
        )

        for k_item in knows_list:
            kb_counter += 1
            knowledge_boundaries.append(
                _make_item(
                    f"knowledge:{bid}:{kb_counter}",
                    f"prop:kb:{bid}:{kb_counter}",
                    f"{cname} knows: {k_item}",
                    relevance=100 - kb_counter,
                    recency=10,
                )
            )
        for dnk_item in dnk_list:
            kb_counter += 1
            knowledge_boundaries.append(
                _make_item(
                    f"knowledge:{bid}:{kb_counter}",
                    f"prop:kb:{bid}:{kb_counter}",
                    f"{cname} does NOT know (withheld): {dnk_item}",
                    relevance=100 - kb_counter,
                    recency=10,
                )
            )
            withheld_constraints.append((cname, dnk_item))

    if not any(p["id"] == pov_char_id for p in participants):
        participants.insert(0, {
            "id": pov_char_id,
            "display_name": pov_name,
            "role": "protagonist",
        })

    promises_due = [
        f"Reader contract ({brief['genre']}): {brief['contract']}",
    ]
    for prom in brief.get("promises") or []:
        if isinstance(prom, dict) and prom.get("description"):
            planted = prom.get("planted") or []
            planted_tag = f" (planted B{planted[0]}C{planted[1]})" if len(planted) >= 2 else ""
            promises_due.append(f"{prom['description']}{planted_tag}")

    criteria = [str(c).strip() for c in (brief.get("criteria") or []) if str(c).strip()]
    prohibited_outcomes = [
        "Do not use prefab AI prose cliches ('It wasn't X, it was Y', 'a mix/cocktail of X and Y', 'didn't know what tomorrow would bring', 'hours passed', or said-bookism clusters like 'hissed/gasped/breathed').",
    ]
    for crit in criteria:
        if re.search(r"\b(?:do not|don't|never|must not|without)\b", crit, re.I):
            prohibited_outcomes.append(crit)

    def _map_constraint_kind(raw_label: str) -> str:
        low = raw_label.lower()
        if "time" in low or "clock" in low:
            return "time"
        if "loc" in low:
            return "location"
        if "know" in low or "secret" in low or "gate" in low:
            return "knowledge"
        if "lock" in low and "author" in low:
            return "author_lock"
        return "canon"

    hard_constraints = []
    facts = []
    fact_counter = 0

    if isinstance(raw_hc, list):
        for hc_idx, entry in enumerate(raw_hc, start=1):
            if isinstance(entry, dict):
                raw_id = str(entry.get("id") or f"constraint_{hc_idx}").strip()
                stmt = str(entry.get("statement") or "").strip()
            else:
                raw_id = f"constraint_{hc_idx}"
                stmt = str(entry).strip()
            if stmt:
                full_stmt = f"[{raw_id}] {stmt}"
                hc_src = _make_source(f"constraint:{bid}:{hc_idx}", full_stmt)
                hard_constraints.append({
                    "id": f"hc:{bid}:{hc_idx}:{_slug(raw_id)}",
                    "kind": _map_constraint_kind(raw_id),
                    "statement": full_stmt,
                    "source": hc_src,
                    "scope": "scene",
                })
                fact_counter += 1
                facts.append(
                    _make_item(
                        f"fact:{bid}:{fact_counter}",
                        f"prop:fact:{bid}:{fact_counter}",
                        full_stmt,
                        relevance=100 - fact_counter,
                        recency=10,
                    )
                )

    loc_name = str(loc.get("name", "Scene Location")).strip()
    loc_summary = str(loc.get("summary", "")).strip()
    loc_kind = str(loc.get("kind", "")).strip()
    if loc_name:
        loc_stmt = f"[LOCATION] {loc_name}" + (f" ({loc_kind})" if loc_kind else "") + (f" — {loc_summary}" if loc_summary else "")
        loc_src = _make_source(f"location:{bid}:primary", loc_stmt)
        hard_constraints.append({
            "id": f"hc:{bid}:location",
            "kind": "location",
            "statement": loc_stmt,
            "source": loc_src,
            "scope": "scene",
        })
        fact_counter += 1
        facts.append(
            _make_item(
                f"fact:{bid}:location",
                f"prop:fact:{bid}:location",
                loc_stmt,
                relevance=95,
                recency=10,
            )
        )

    for r_idx, rule_pair in enumerate(brief.get("rules") or [], start=1):
        if isinstance(rule_pair, (list, tuple)) and len(rule_pair) >= 2:
            r_name, r_stmt = str(rule_pair[0]).strip(), str(rule_pair[1]).strip()
            if r_stmt:
                full_rule = f"[WORLD RULE: {r_name}] {r_stmt}"
                r_src = _make_source(f"world_rule:{bid}:{r_idx}", full_rule)
                hard_constraints.append({
                    "id": f"hc:{bid}:rule:{r_idx}",
                    "kind": "canon",
                    "statement": full_rule,
                    "source": r_src,
                    "scope": "project",
                })
                fact_counter += 1
                facts.append(
                    _make_item(
                        f"fact:{bid}:rule:{r_idx}",
                        f"prop:fact:{bid}:rule:{r_idx}",
                        full_rule,
                        relevance=98,
                        recency=10,
                    )
                )

    for w_idx, (cname, dnk_stmt) in enumerate(withheld_constraints, start=1):
        w_text = f"[KNOWLEDGE GATE] {cname} does NOT know: {dnk_stmt}"
        w_src = _make_source(f"withheld:{bid}:{w_idx}", w_text)
        hard_constraints.append({
            "id": f"hc:{bid}:withheld:{w_idx}",
            "kind": "knowledge",
            "statement": w_text,
            "source": w_src,
            "scope": "scene",
        })

    prior_tail_text = str(ctx.get("previous_scene_tail") or "").strip()
    prior_scene_tail = (
        _make_item(
            f"prior_scene_tail:{bid}",
            f"prop:tail:{bid}",
            prior_tail_text,
            relevance=100,
            recency=10,
        )
        if prior_tail_text
        else None
    )

    story_so_far = []
    for ep_idx, ep in enumerate(episodes, start=1):
        ep_summary = str(ep.get("summary") or "").strip()
        if ep_summary:
            story_so_far.append(
                _make_item(
                    f"episode:{bid}:{ep_idx}",
                    f"prop:episode:{bid}:{ep_idx}",
                    f"Book {ep.get('book', 1)} Ch {ep.get('chapter', ep_idx)}: {ep_summary}",
                    relevance=80 + ep_idx,
                    recency=ep_idx,
                )
            )

    open_threads = [
        _make_item(
            f"thread:{bid}:task",
            f"prop:thread:{bid}:task",
            f"Drafting task ({brief['scene_type']}): {brief['task']}",
            relevance=100,
            recency=10,
        )
    ]

    scene_card_src = _make_source(f"scene_card:{bid}", brief["scene_purpose"])
    pov_src = _make_source(f"pov:{bid}", pov)

    scene_card = {
        "source": scene_card_src,
        "pov": {
            "character_id": pov_char_id,
            "display_name": pov_name,
            "person": person,
            "tense": tense,
            "source": pov_src,
        },
        "goal": str(brief["scene_purpose"]),
        "opposition": f"Reader contract ({brief['genre']}): {brief['contract']}",
        "turn": str(brief["tension_turn"]),
        "intended_outcome": str(brief["task"]),
        "value_shift": {
            "axis": str(brief["scene_type"]),
            "before": "opening equilibrium under constraint",
            "after": str(brief["tension_turn"]),
        },
        "emotion": {
            "entry": "tense focus on immediate objective",
            "exit": "consequence of the dramatic turn",
        },
        "chapter": {
            "book_number": book_num,
            "number": chapter_num,
            "scene_index": scene_num,
            "planned_scene_count": max(scene_num, 1),
            "is_closer": bool(brief.get("scene_type") == "chapter-closer"),
        },
        "target_words": {
            "min": 350,
            "preferred": 450,
            "max": 550,
        },
        "location": {
            "id": f"location:{bid}:primary",
            "name": loc_name,
        },
        "story_clock": story_clock,
        "rating": "general",
        "tone": str(brief["contract"]),
        "participants": participants,
        "required_beats": criteria,
        "promises_due": promises_due,
        "prohibited_outcomes": prohibited_outcomes,
    }

    canon_hash = digest({"brief_id": bid, "hard_constraints": hard_constraints, "facts": facts})[:16]
    plan_rev = digest({"brief_id": bid, "scene_card": scene_card})[:16]

    return {
        "schema_version": "drafting_packet_v1",
        "packet_id": packet_id,
        "project_id": project_id,
        "branch_id": branch_id,
        "run_id": run_id,
        "scene_id": scene_id,
        "scene_revision": "rev-1",
        "canon_snapshot_hash": canon_hash,
        "plan_revision": plan_rev,
        "source_versions": source_versions,
        "scene_card": scene_card,
        "hard_constraints": hard_constraints,
        "continuity": {
            "character_states": character_states,
            "knowledge_boundaries": knowledge_boundaries,
            "facts": facts,
            "prior_scene_tail": prior_scene_tail,
            "story_so_far": story_so_far,
            "open_threads": open_threads,
        },
        "voice": {
            "rules": voice_rules,
            "exemplars": [],
        },
        "editorial_memory": {
            "recent_openings": [],
            "recent_endings": [],
            "recurring_images": [],
            "repeated_phrases": [],
            "pacing_history": [],
        },
        "craft_rubric": {
            "version": "scene-craft-v1",
            "text": "",
        },
        "research_evidence": [],
        "assembly": None,
    }


def resolve_tokenizer_binary(explicit: Path | str | None = None) -> Path:
    """Resolve the spindle-tokenizer Rust binary path for the generation bridge."""
    candidates = []
    if explicit:
        candidates.append(Path(explicit))
    env_bin = os.environ.get("SPINDLE_TOKENIZER_BINARY")
    if env_bin:
        candidates.append(Path(env_bin))
    cargo_target = os.environ.get("CARGO_TARGET_DIR")
    if cargo_target:
        candidates.append(Path(cargo_target) / "debug" / "spindle-tokenizer")
        candidates.append(Path(cargo_target) / "release" / "spindle-tokenizer")
    candidates.extend([
        Path("/tmp/spindle-34-target/debug/spindle-tokenizer"),
        Path("/tmp/spindle-34-target/release/spindle-tokenizer"),
        REPO / "target" / "debug" / "spindle-tokenizer",
        REPO / "target" / "release" / "spindle-tokenizer",
    ])
    for cand in candidates:
        if cand.is_file() and os.access(cand, os.X_OK):
            return cand
    raise RuntimeError(
        "spindle-tokenizer binary not found. Build it with "
        "`cargo build -p spindle-adapters --bin spindle-tokenizer` or pass --tokenizer-binary."
    )


def parse_generation_seeds(
    seed: int | None = None,
    seeds_csv: str | list[int] | tuple[int, ...] | None = None,
) -> list[int | None]:
    """Parse single or comma-separated generation seeds, preserving order and deduplicating."""
    if seeds_csv is not None:
        if isinstance(seeds_csv, (list, tuple)):
            raw_items = [int(x) for x in seeds_csv if x is not None]
        else:
            raw_items = [int(part.strip()) for part in str(seeds_csv).split(",") if part.strip()]
        deduped: list[int | None] = []
        for item in raw_items:
            if item not in deduped:
                deduped.append(item)
        if deduped:
            return deduped
    if seed is not None:
        return [int(seed)]
    return [None]


def render_variant_prompt(brief: dict, variant_cfg: dict) -> tuple[str, str]:
    """Render system and user prompts for a drafter prompt variant."""
    sys_prompt = variant_cfg.get(
        "system_prompt",
        "You are a fiction drafting assistant. Write the requested scene and output prose only.",
    )
    episodes = brief.get("episodes") or []
    compact_lines = [f"- Book {e['book']} Ch {e['chapter']}: {e['summary']}" for e in episodes[-3:]]
    compact_summaries = "\n".join(compact_lines) if compact_lines else "- None."
    ctx_md = brief.get("drafter_context", {}).get("context_markdown", "")
    pov = brief.get("pov") or {}
    tmpl = variant_cfg.get("user_template") or "{context_markdown}\n\nTask: {task}"
    user_prompt = tmpl.format(
        contract=brief.get("contract", ""),
        pov_character=pov.get("character", ""),
        pov_perspective=pov.get("perspective", ""),
        scene_type=brief.get("scene_type", ""),
        scene_purpose=brief.get("scene_purpose", ""),
        tension_turn=brief.get("tension_turn", ""),
        task=brief.get("task", ""),
        compact_summaries=compact_summaries,
        context_markdown=ctx_md,
    )
    return sys_prompt, user_prompt


def call_rust_bridge_for_brief(
    brief: dict,
    variant_cfg: dict,
    generator_cfg: dict,
    condition_label: str = "candidate",
    generation_seed: int | None = None,
) -> dict:
    """Invoke the Rust ModelRouter / SqliteSpindleService bridge via spindle-tokenizer."""
    rust_config = generator_cfg.get("rust_config")
    rust_db = generator_cfg.get("rust_db")
    if not rust_config or not rust_db:
        raise ValueError(
            "Rust generation bridge requires both 'rust_config' (--rust-config) and 'rust_db' (--rust-db)"
        )
    bin_path = resolve_tokenizer_binary(generator_cfg.get("tokenizer_binary"))
    route = str(generator_cfg.get("route") or "draft")
    rating = str(generator_cfg.get("rating") or "general")
    variant_id = str(variant_cfg.get("id", "custom_variant"))
    context_mode = str(variant_cfg.get("context_mode", "full_drafter_context"))
    artifact_root = Path(
        generator_cfg.get("artifact_dir")
        or (Path(rust_db).resolve().parent / "prose_judge_bridge_artifacts")
    )
    artifact_root.mkdir(parents=True, exist_ok=True)
    seed_tag = f"seed_{generation_seed}" if generation_seed is not None else "seed_none"

    max_attempts = 3
    proc = None
    elapsed = 0.0
    for attempt in range(max_attempts):
        if context_mode == "drafting_packet_v1":
            packet = brief_to_drafting_packet_v1(brief)
            packet_path = artifact_root / "packets" / f"{brief['id']}.packet.json"
            # Preserve any existing recovery .lock files untouched; if a previous attempt left a
            # recovery lock on the primary artifact path, use a fresh attempt-scoped artifact path.
            candidate_art_path = artifact_root / "artifacts" / f"{brief['id']}_{seed_tag}.artifact.json"
            attempt_idx = attempt
            while Path(f"{candidate_art_path}.lock").exists() and not candidate_art_path.exists():
                attempt_idx += 1
                candidate_art_path = (
                    artifact_root / "artifacts" / f"{brief['id']}_{seed_tag}_attempt{attempt_idx}.artifact.json"
                )
            artifact_path = candidate_art_path
            write_json(packet_path, packet)
            cmd = [
                str(bin_path),
                "draft-packet",
                str(rust_config),
                str(rust_db),
                str(packet_path),
                str(artifact_path),
            ]
            if generation_seed is not None:
                cmd.append(str(generation_seed))
        else:
            sys_prompt, user_prompt = render_variant_prompt(brief, variant_cfg)
            messages_path = artifact_root / "messages" / f"{brief['id']}_{variant_id}.messages.json"
            write_json(messages_path, {"system": sys_prompt, "user": user_prompt})
            cmd = [
                str(bin_path),
                "draft-messages",
                str(rust_config),
                str(rust_db),
                route,
                rating,
                str(messages_path),
            ]
            if generation_seed is not None:
                cmd.append(str(generation_seed))

        t0 = time.monotonic()
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        elapsed = round(time.monotonic() - t0, 3)
        if proc.returncode == 0:
            break
        if attempt + 1 < max_attempts:
            time.sleep(2.5 * (attempt + 1))
    assert proc is not None
    if proc.returncode != 0:
        err_msg = redact_secrets((proc.stderr or proc.stdout or "").strip())
        raise RuntimeError(
            f"spindle-tokenizer bridge failed (exit {proc.returncode}) for {brief['id']} ({variant_id}, seed={generation_seed}): {err_msg}"
        )
    try:
        bridge_out = json.loads(proc.stdout)
    except Exception as exc:
        raise RuntimeError(
            f"Failed to parse spindle-tokenizer JSON output for {brief['id']} ({variant_id}): {exc}"
        ) from None

    settings = bridge_out.get("settings") or {}
    receipt = bridge_out.get("receipt") or {}
    model_name = (
        bridge_out.get("model")
        or receipt.get("model")
        or settings.get("model")
        or generator_cfg.get("model", "unknown")
    )
    model_rev = settings.get("model_revision") or "unpinned"
    truncated = bool(bridge_out.get("truncated", False))
    if truncated:
        raise RuntimeError(
            f"Incomplete/truncated Rust bridge output for {brief['id']} ({variant_id}, seed={generation_seed}): "
            "truncated=True; incomplete drafts cannot enter evaluation or release-gate statistics"
        )
    text = bridge_out.get("text") or bridge_out.get("accepted_text") or ""
    if not isinstance(text, str) or not text.strip():
        raise RuntimeError(
            f"Empty Rust bridge output for {brief['id']} ({variant_id}, seed={generation_seed})"
        )
    computed_sha = hashlib.sha256(text.encode("utf-8")).hexdigest()
    if context_mode == "drafting_packet_v1":
        accepted_sha = bridge_out.get("accepted_text_sha256")
        if not accepted_sha:
            raise RuntimeError(
                f"Packet draft artifact for {brief['id']} ({variant_id}, seed={generation_seed}) "
                "was not accepted (missing accepted_text_sha256)"
            )
        raw_text_sha = bridge_out.get("text_sha256") or bridge_out.get("raw_text_sha256")
        if raw_text_sha and accepted_sha != raw_text_sha:
            raise RuntimeError(
                f"Packet draft artifact acceptance hash mismatch for {brief['id']} ({variant_id}, seed={generation_seed}): "
                f"accepted_text_sha256={accepted_sha} != text_sha256={raw_text_sha}"
            )
        if accepted_sha != computed_sha:
            raise RuntimeError(
                f"Packet draft artifact text SHA256 mismatch for {brief['id']} ({variant_id}, seed={generation_seed}): "
                f"accepted_text_sha256={accepted_sha} != computed={computed_sha}"
            )
        text_sha = accepted_sha
        raw_text_sha = raw_text_sha or accepted_sha
    else:
        accepted_sha = None
        text_sha = bridge_out.get("text_sha256") or computed_sha
        if text_sha != computed_sha:
            raise RuntimeError(
                f"Message draft text SHA256 mismatch for {brief['id']} ({variant_id}, seed={generation_seed}): "
                f"text_sha256={text_sha} != computed={computed_sha}"
            )
        raw_text_sha = bridge_out.get("raw_text_sha256") or text_sha

    model_call_id = (
        bridge_out.get("model_call_id")
        or receipt.get("generation_id")
        or receipt.get("call_id")
    )

    return {
        "case_id": brief["id"],
        "brief_id": brief["id"],
        "condition": condition_label,
        "prompt_variant": variant_id,
        "generation_seed": bridge_out.get("generation_seed", generation_seed),
        "model": model_name,
        "model_version": f"rust-bridge:{model_name}@{model_rev}",
        "text": text,
        "truncated": truncated,
        "text_sha256": text_sha,
        "raw_text_sha256": raw_text_sha,
        "accepted_text_sha256": accepted_sha,
        "packet_hash": bridge_out.get("packet_hash"),
        "source_packet_sha256": bridge_out.get("source_packet_sha256"),
        "prompt_hash": bridge_out.get("prompt_hash"),
        "tokenizer_id": bridge_out.get("tokenizer_id") or settings.get("tokenizer_id"),
        "tokenizer_version": bridge_out.get("tokenizer_version") or settings.get("tokenizer_version"),
        "settings": settings,
        "artifact_id": bridge_out.get("artifact_id"),
        "artifact_path": bridge_out.get("artifact_path"),
        "model_call_id": model_call_id,
        "input_tokens": bridge_out.get("input_tokens"),
        "output_tokens": bridge_out.get("output_tokens"),
        "elapsed_seconds": elapsed,
    }


def compute_generation_input_fingerprint(
    brief: dict,
    variant_cfg: dict,
    generator_cfg: dict,
    condition_label: str = "candidate",
    generation_seed: int | None = None,
) -> str:
    """Compute a deterministic fingerprint of the effective brief, variant, and generator config."""
    effective_seed = (
        generation_seed
        if generation_seed is not None
        else (int(generator_cfg["seed"]) if generator_cfg.get("seed") is not None else None)
    )
    provider = str(generator_cfg.get("provider", "fake")).lower()
    if provider != "fake" and generator_cfg.get("rust_config"):
        provider = "rust-bridge"
    rust_cfg_path = generator_cfg.get("rust_config")
    rust_cfg_hash = None
    if rust_cfg_path:
        p = Path(rust_cfg_path)
        if p.exists():
            try:
                rust_cfg_hash = hashlib.sha256(p.read_bytes()).hexdigest()
            except Exception:
                rust_cfg_hash = str(rust_cfg_path)
        else:
            rust_cfg_hash = str(rust_cfg_path)
    normalized_gen = {
        "provider": provider,
        "model": generator_cfg.get("model"),
        "endpoint": generator_cfg.get("endpoint"),
        "api_key_env": generator_cfg.get("api_key_env"),
        "temperature": generator_cfg.get("temperature"),
        "max_tokens": generator_cfg.get("max_tokens"),
        "route": str(generator_cfg.get("route") or "draft") if provider == "rust-bridge" else generator_cfg.get("route"),
        "rating": str(generator_cfg.get("rating") or "general") if provider == "rust-bridge" else generator_cfg.get("rating"),
        "rust_config_hash": rust_cfg_hash,
        "tokenizer_binary": str(generator_cfg.get("tokenizer_binary")) if generator_cfg.get("tokenizer_binary") else None,
    }
    return digest(
        {
            "brief": brief,
            "variant": variant_cfg,
            "condition": condition_label,
            "generation_seed": effective_seed,
            "generator_cfg": normalized_gen,
        }
    )


def generate_candidate_for_brief(
    brief: dict,
    variant_cfg: dict,
    generator_cfg: dict,
    condition_label: str = "candidate",
    generation_seed: int | None = None,
) -> dict:
    """Produce a single candidate output for a brief via fake, Rust bridge, or live model adapter."""
    effective_seed = (
        generation_seed
        if generation_seed is not None
        else (int(generator_cfg["seed"]) if generator_cfg.get("seed") is not None else None)
    )
    gen_input_fp = compute_generation_input_fingerprint(
        brief,
        variant_cfg,
        generator_cfg,
        condition_label=condition_label,
        generation_seed=effective_seed,
    )
    provider = str(generator_cfg.get("provider", "fake")).lower()
    if provider == "rust-bridge" or (provider != "fake" and generator_cfg.get("rust_config")):
        out = call_rust_bridge_for_brief(
            brief,
            variant_cfg,
            generator_cfg,
            condition_label=condition_label,
            generation_seed=effective_seed,
        )
        out["generation_input_fingerprint"] = gen_input_fp
        return out

    sys_prompt, user_prompt = render_variant_prompt(brief, variant_cfg)
    variant_id = variant_cfg.get("id", "custom_variant")
    context_mode = variant_cfg.get("context_mode", "full_drafter_context")
    packet_obj = brief_to_drafting_packet_v1(brief) if context_mode == "drafting_packet_v1" else None
    source_packet_sha256 = digest(packet_obj) if packet_obj is not None else None
    prompt_hash = digest({"system": sys_prompt, "user": json.dumps(packet_obj, sort_keys=True) if packet_obj else user_prompt})
    packet_hash = prompt_hash if packet_obj is not None else None

    if provider == "fake":
        # Offline deterministic generator: full_drafter_context and drafting_packet_v1 honor all brief constraints;
        # compact mode produces a shorter, slop-prone draft.
        loc_name = brief.get("drafter_context", {}).get("location", {}).get("name", "the room")
        pov_char = brief.get("pov", {}).get("character", "The protagonist")
        persp = brief.get("pov", {}).get("perspective", "third_limited")
        subj = "I" if persp == "first_person" else pov_char
        if context_mode == "compact":
            text = (
                f"{subj} stood in {loc_name}. It wasn't fear. It was determination. "
                f"A mix of relief and dread washed over the moment.\n\n"
                f"'We must finish this,' {subj} hissed, not knowing what tomorrow would bring."
            )
        else:
            crit_summary = " ".join(brief.get("criteria", []))
            tail = brief.get("drafter_context", {}).get("previous_scene_tail", "")
            seed_note = f" [seed={effective_seed}]" if effective_seed is not None else ""
            text = (
                f"{tail}\n\n"
                f"At {loc_name}, {subj} acted on the immediate constraint: {brief.get('scene_purpose', '')}{seed_note}\n\n"
                f"'Hold the line steady,' {subj} said, keeping to the terms of the brief ({crit_summary}).\n\n"
                f"The turn completed cleanly as {brief.get('tension_turn', '')}"
            )
        clean_text = text.strip()
        text_sha = digest(clean_text)
        return {
            "case_id": brief["id"],
            "brief_id": brief["id"],
            "condition": condition_label,
            "prompt_variant": variant_id,
            "generation_seed": effective_seed,
            "generation_input_fingerprint": gen_input_fp,
            "model": generator_cfg.get("model", "spindle-fake-drafter-v1"),
            "model_version": "offline-v1",
            "text": clean_text,
            "truncated": False,
            "text_sha256": text_sha,
            "accepted_text_sha256": text_sha if packet_obj is not None else None,
            "packet_hash": packet_hash,
            "source_packet_sha256": source_packet_sha256,
            "prompt_hash": prompt_hash,
            "settings": {
                "transport": "fake",
                "temperature": float(generator_cfg.get("temperature", 0.4)),
                "max_tokens": int(generator_cfg.get("max_tokens", 1200)),
                "seed": effective_seed,
            },
            "input_tokens": len(user_prompt.split()),
            "output_tokens": len(clean_text.split()),
            "elapsed_seconds": 0.001,
        }

    gen_call_cfg = {**generator_cfg, "json_mode": False}
    if effective_seed is not None:
        gen_call_cfg["seed"] = effective_seed
    resp = call_model_provider(gen_call_cfg, sys_prompt, user_prompt)
    text_sha = digest(resp["text"])
    return {
        "case_id": brief["id"],
        "brief_id": brief["id"],
        "condition": condition_label,
        "prompt_variant": variant_id,
        "generation_seed": effective_seed,
        "generation_input_fingerprint": gen_input_fp,
        "model": resp.get("model", generator_cfg.get("model", "unknown")),
        "model_version": f"{provider}:{generator_cfg.get('model', 'unknown')}",
        "text": resp["text"],
        "truncated": bool(resp.get("truncated", False)),
        "text_sha256": text_sha,
        "accepted_text_sha256": text_sha if packet_obj is not None and not resp.get("truncated", False) else None,
        "packet_hash": packet_hash,
        "source_packet_sha256": source_packet_sha256,
        "prompt_hash": prompt_hash,
        "settings": {
            "transport": provider,
            "temperature": float(generator_cfg.get("temperature", 0.4)),
            "max_tokens": int(generator_cfg.get("max_tokens", 1200)),
            "seed": effective_seed,
        },
        "input_tokens": resp.get("input_tokens"),
        "output_tokens": resp.get("output_tokens"),
        "elapsed_seconds": resp.get("elapsed_seconds"),
    }


def generate_candidates_with_checkpoint(
    briefs: list[dict],
    variant_cfg: dict,
    generator_cfg: dict,
    out_path: Path | str,
    condition_label: str = "candidate",
    seeds: list[int | None] | None = None,
) -> list[dict]:
    """Generate candidates across briefs and seeds, reusing checkpoint entries only when their generation_input_fingerprint and completeness match."""
    effective_seeds = seeds if seeds else [None]
    out_file = Path(out_path)
    existing_by_key: dict[tuple[str, int | None], dict] = {}
    if out_file.exists():
        try:
            prev = read_json(out_file)
            prev_list = prev.get("candidates", prev) if isinstance(prev, dict) else prev
            if isinstance(prev_list, list):
                for item in prev_list:
                    if isinstance(item, dict) and item.get("text"):
                        try:
                            validate_candidate_completeness(item, arm_label=condition_label)
                        except ValueError:
                            continue
                        bid = item.get("case_id") or item.get("brief_id")
                        existing_by_key[(bid, item.get("generation_seed"))] = item
        except Exception:
            pass

    outputs: list[dict] = []
    for b in briefs:
        for gseed in effective_seeds:
            expected_fp = compute_generation_input_fingerprint(
                b,
                variant_cfg,
                generator_cfg,
                condition_label=condition_label,
                generation_seed=gseed,
            )
            key = (b["id"], gseed)
            cached_item = existing_by_key.get(key)
            if cached_item and cached_item.get("generation_input_fingerprint") == expected_fp:
                outputs.append(cached_item)
                continue
            cand = generate_candidate_for_brief(
                b,
                variant_cfg,
                generator_cfg,
                condition_label=condition_label,
                generation_seed=gseed,
            )
            outputs.append(cand)
            write_json(out_file, outputs)
    write_json(out_file, outputs)
    return outputs


def validate_candidate_completeness(record: dict, arm_label: str = "candidate") -> None:
    """Reject truncated outputs or unaccepted packet-arm artifacts before checkpointing or judging."""
    if not isinstance(record, dict):
        raise ValueError(f"{arm_label} record must be a dict")
    bid = record.get("brief_id") or record.get("case_id") or "<unknown>"
    gseed = record.get("generation_seed")
    if record.get("truncated") is True:
        raise ValueError(
            f"Incomplete/truncated {arm_label} output for {bid} (seed={gseed}): truncated=True"
        )
    text = record.get("text")
    if not isinstance(text, str) or not text.strip():
        raise ValueError(f"Empty {arm_label} text for {bid} (seed={gseed})")
    is_packet_arm = (
        record.get("prompt_variant") == "drafting_packet_v1"
        or record.get("context_mode") == "drafting_packet_v1"
        or record.get("artifact_id") is not None
        or record.get("source_packet_sha256") is not None
    )
    if is_packet_arm:
        acc_sha = record.get("accepted_text_sha256")
        if not acc_sha:
            raise ValueError(
                f"Unaccepted packet {arm_label} output for {bid} (seed={gseed}): missing accepted_text_sha256"
            )
        raw_sha = record.get("text_sha256") or record.get("raw_text_sha256")
        if raw_sha and acc_sha != raw_sha:
            raise ValueError(
                f"Stale or mismatched acceptance hash on {arm_label} output for {bid} (seed={gseed}): "
                f"accepted_text_sha256={acc_sha} != text_sha256={raw_sha}"
            )


def load_or_generate_pairs(
    briefs: list[dict],
    pairs_path: Path | str | None = None,
    baseline_path: Path | str | None = None,
    candidate_path: Path | str | None = None,
    variant_a: str | None = None,
    variant_b: str | None = None,
    variants_file: Path | str = VARIANTS_PATH,
    generator_override: dict | None = None,
    limit: int | None = None,
    generation_seeds: list[int | None] | None = None,
) -> list[dict]:
    """Load candidate pairs from saved files OR generate them from two prompt variants."""
    selected_briefs = briefs[:limit] if limit else briefs
    brief_map = {b["id"]: b for b in selected_briefs}
    seeds = generation_seeds if generation_seeds else [None]
    multi_seed_mode = len(seeds) > 1 or seeds[0] is not None

    if variant_a and variant_b:
        vfile = read_json(variants_file)
        variants = vfile.get("variants", vfile)
        if variant_a not in variants or variant_b not in variants:
            raise ValueError(f"Variants {variant_a!r} and {variant_b!r} must exist in {variants_file}")
        gen_cfg = dict(vfile.get("default_generator") or {"provider": "fake", "model": "spindle-fake-drafter-v1"})
        if generator_override:
            gen_cfg.update(generator_override)
        pairs = []
        for idx, b in enumerate(selected_briefs, start=1):
            for gseed in seeds:
                base_cand = generate_candidate_for_brief(
                    b,
                    variants[variant_a],
                    gen_cfg,
                    condition_label="baseline",
                    generation_seed=gseed,
                )
                cand_cand = generate_candidate_for_brief(
                    b,
                    variants[variant_b],
                    gen_cfg,
                    condition_label="candidate",
                    generation_seed=gseed,
                )
                pid = f"P{idx:03d}-S{gseed}" if multi_seed_mode else f"P{idx:03d}"
                pairs.append({
                    "pair_id": pid,
                    "case_id": b["id"],
                    "brief_id": b["id"],
                    "generation_seed": gseed,
                    "scene_type": b["scene_type"],
                    "genre": b["genre"],
                    "baseline": base_cand,
                    "candidate": cand_cand,
                })
        return pairs

    if baseline_path and candidate_path:
        base_raw = read_json(baseline_path)
        cand_raw = read_json(candidate_path)
        base_list = base_raw.get("candidates", base_raw) if isinstance(base_raw, dict) else base_raw
        cand_list = cand_raw.get("candidates", cand_raw) if isinstance(cand_raw, dict) else cand_raw

        base_by_id_seed: dict[tuple[str, int | None], dict] = {}
        cand_by_id_seed: dict[tuple[str, int | None], dict] = {}
        seeds_by_brief: dict[str, list[int | None]] = {}
        for r in base_list:
            bid = r.get("case_id") or r.get("brief_id")
            gseed = r.get("generation_seed")
            base_by_id_seed[(bid, gseed)] = r
            seeds_by_brief.setdefault(bid, [])
            if gseed not in seeds_by_brief[bid]:
                seeds_by_brief[bid].append(gseed)
        for r in cand_list:
            bid = r.get("case_id") or r.get("brief_id")
            gseed = r.get("generation_seed")
            cand_by_id_seed[(bid, gseed)] = r

        pairs = []
        for idx, b in enumerate(selected_briefs, start=1):
            bid = b["id"]
            brief_seeds = seeds if multi_seed_mode else (seeds_by_brief.get(bid) or [None])
            use_seed_suffix = multi_seed_mode or len(brief_seeds) > 1 or (brief_seeds and brief_seeds[0] is not None)
            for gseed in brief_seeds:
                if (bid, gseed) not in base_by_id_seed or (bid, gseed) not in cand_by_id_seed:
                    raise ValueError(f"Missing brief {bid} (seed={gseed}) in baseline or candidate file")
                base_rec = base_by_id_seed[(bid, gseed)]
                cand_rec = cand_by_id_seed[(bid, gseed)]
                validate_candidate_completeness(base_rec, arm_label="baseline")
                validate_candidate_completeness(cand_rec, arm_label="candidate")
                pid = f"P{idx:03d}-S{gseed}" if use_seed_suffix else f"P{idx:03d}"
                pairs.append({
                    "pair_id": pid,
                    "case_id": bid,
                    "brief_id": bid,
                    "generation_seed": gseed,
                    "scene_type": b["scene_type"],
                    "genre": b["genre"],
                    "baseline": base_rec,
                    "candidate": cand_rec,
                })
        return pairs

    source_path = pairs_path or FIXTURE_PAIRS_PATH
    raw = read_json(source_path)
    raw_pairs = raw.get("pairs", raw) if isinstance(raw, dict) else raw
    norm_pairs, _ = serial_fiction.normalize_pairwise_input(raw_pairs, brief_map)
    filtered = [p for p in norm_pairs if p["case_id"] in brief_map]
    if limit:
        filtered = filtered[:limit]
    for p in filtered:
        b = brief_map[p["case_id"]]
        p["brief_id"] = b["id"]
        p["scene_type"] = b["scene_type"]
        p["genre"] = b["genre"]
    return filtered


# =============================================================================
# Pairwise Swapped-Order Double Judging & Statistical Reporting
# =============================================================================

def evaluate_single_pass(
    judge_cfg: dict,
    brief: dict,
    first_cond: str,
    second_cond: str,
    first_text: str,
    second_text: str,
) -> dict:
    """Run one directional pass (Passage 1 = first_cond, Passage 2 = second_cond) and de-blind."""
    provider = str(judge_cfg.get("provider", "fake")).lower()
    if provider == "fake":
        raw_eval = run_fake_judge(judge_cfg, brief, first_text, second_text)
        usage_meta = {"input_tokens": None, "output_tokens": None, "elapsed_seconds": 0.0}
    else:
        sys_p, user_p = build_judge_prompts(brief, first_text, second_text)
        last_err = None
        for attempt in range(3):
            call_cfg = dict(judge_cfg)
            if attempt > 0:
                call_cfg["temperature"] = 0.1 * attempt
            resp = call_model_provider(call_cfg, sys_p, user_p)
            try:
                parsed = extract_json_object(resp["text"])
                raw_eval = validate_judge_response(parsed)
                usage_meta = {
                    "input_tokens": resp.get("input_tokens"),
                    "output_tokens": resp.get("output_tokens"),
                    "elapsed_seconds": resp.get("elapsed_seconds"),
                }
                break
            except Exception as exc:
                last_err = exc
        else:
            raise RuntimeError(f"Judge {judge_cfg.get('id')} failed schema validation after 3 attempts: {last_err}")

    validated = validate_judge_response(raw_eval)
    slot_map = {"passage_1": first_cond, "passage_2": second_cond, "tie": "tie"}
    deblinded_winner = slot_map[validated["overall_preference"]]

    deblinded_rubric = {}
    for dim, dval in validated["rubric"].items():
        if first_cond == "baseline":
            b_score, c_score = dval["passage_1_score"], dval["passage_2_score"]
        else:
            b_score, c_score = dval["passage_2_score"], dval["passage_1_score"]
        deblinded_rubric[dim] = {
            "baseline_score": b_score,
            "candidate_score": c_score,
            "winner": slot_map[dval["winner"]],
            "note": dval["note"],
        }

    return {
        "order": [first_cond, second_cond],
        "raw_overall_preference": validated["overall_preference"],
        "deblinded_preference": deblinded_winner,
        "rationale": validated["rationale"],
        "rubric": deblinded_rubric,
        "usage": usage_meta,
    }


def compute_judge_config_fingerprint(judge_cfg: dict) -> str:
    """Compute a deterministic fingerprint of a single judge's configuration."""
    return digest(judge_cfg)


def compute_pair_prompt_seed_fingerprint(brief: dict, pair: dict, seed: int) -> str:
    """Compute a deterministic fingerprint of the judge prompt inputs, rubric, texts, and randomization seed."""
    base_text = pair["baseline"]["text"]
    cand_text = pair["candidate"]["text"]
    sys_p, user_p = build_judge_prompts(brief, base_text, cand_text)
    return digest(
        {
            "pair_id": str(pair["pair_id"]),
            "brief_id": brief["id"],
            "seed": seed,
            "rubric_dimensions": list(RUBRIC_DIMENSIONS),
            "baseline_text_hash": digest(base_text),
            "candidate_text_hash": digest(cand_text),
            "system_prompt": sys_p,
            "user_prompt": user_p,
        }
    )


def compute_judge_pair_input_fingerprint(judge_cfg: dict, brief: dict, pair: dict, seed: int) -> str:
    """Compute a deterministic fingerprint of a judge's config and prompt/randomization inputs for a pair."""
    return digest(
        {
            "judge_config_fingerprint": compute_judge_config_fingerprint(judge_cfg),
            "pair_prompt_seed_fingerprint": compute_pair_prompt_seed_fingerprint(brief, pair, seed),
        }
    )


def compute_pair_judgment_fingerprint(brief: dict, pair: dict, judges: list[dict], seed: int) -> str:
    """Compute a deterministic fingerprint of the full judge panel and pair prompt/randomization inputs."""
    return digest(
        {
            "pair_prompt_seed_fingerprint": compute_pair_prompt_seed_fingerprint(brief, pair, seed),
            "panel": [
                {
                    "id": j["id"],
                    "judge_config_fingerprint": compute_judge_config_fingerprint(j),
                }
                for j in judges
            ],
        }
    )


def judge_pair_double(
    judge_cfg: dict,
    brief: dict,
    pair: dict,
    seed: int,
) -> dict:
    """Evaluate a pair twice: Pass 1 (randomized order) and Pass 2 (swapped order)."""
    rng = random.Random(f"{seed}:{pair['pair_id']}:{judge_cfg['id']}")
    initial_order = ("baseline", "candidate") if rng.random() < 0.5 else ("candidate", "baseline")
    swapped_order = (initial_order[1], initial_order[0])

    if str(judge_cfg.get("provider", "fake")).lower() != "fake":
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            f1 = pool.submit(
                evaluate_single_pass,
                judge_cfg,
                brief,
                initial_order[0],
                initial_order[1],
                pair[initial_order[0]]["text"],
                pair[initial_order[1]]["text"],
            )
            f2 = pool.submit(
                evaluate_single_pass,
                judge_cfg,
                brief,
                swapped_order[0],
                swapped_order[1],
                pair[swapped_order[0]]["text"],
                pair[swapped_order[1]]["text"],
            )
            pass_1 = f1.result()
            pass_2 = f2.result()
    else:
        pass_1 = evaluate_single_pass(
            judge_cfg,
            brief,
            initial_order[0],
            initial_order[1],
            pair[initial_order[0]]["text"],
            pair[initial_order[1]]["text"],
        )
        pass_2 = evaluate_single_pass(
            judge_cfg,
            brief,
            swapped_order[0],
            swapped_order[1],
            pair[swapped_order[0]]["text"],
            pair[swapped_order[1]]["text"],
        )

    w1 = pass_1["deblinded_preference"]
    w2 = pass_2["deblinded_preference"]
    raw1 = pass_1["raw_overall_preference"]
    raw2 = pass_2["raw_overall_preference"]

    if w1 == w2:
        final_verdict = w1
        order_consistent = True
        position_bias_kind = "none"
    else:
        final_verdict = "tie"
        order_consistent = False
        if raw1 == "passage_1" and raw2 == "passage_1":
            position_bias_kind = "first_position"
        elif raw1 == "passage_2" and raw2 == "passage_2":
            position_bias_kind = "second_position"
        else:
            position_bias_kind = "tie_flip"

    combined_rubric = {}
    for dim in RUBRIC_DIMENSIONS:
        r1 = pass_1["rubric"][dim]
        r2 = pass_2["rubric"][dim]
        b_mean = round((r1["baseline_score"] + r2["baseline_score"]) / 2.0, 2)
        c_mean = round((r1["candidate_score"] + r2["candidate_score"]) / 2.0, 2)
        if r1["winner"] == r2["winner"]:
            dim_winner = r1["winner"]
        elif c_mean > b_mean:
            dim_winner = "candidate"
        elif b_mean > c_mean:
            dim_winner = "baseline"
        else:
            dim_winner = "tie"
        combined_rubric[dim] = {
            "baseline_score": b_mean,
            "candidate_score": c_mean,
            "winner": dim_winner,
            "pass_1_note": r1["note"],
            "pass_2_note": r2["note"],
        }

    return {
        "pair_id": pair["pair_id"],
        "brief_id": brief["id"],
        "judge_id": judge_cfg["id"],
        "judge_family": judge_cfg.get("family", "unknown"),
        "judge_config_fingerprint": compute_judge_config_fingerprint(judge_cfg),
        "judgment_input_fingerprint": compute_judge_pair_input_fingerprint(judge_cfg, brief, pair, seed),
        "verdict": final_verdict,
        "order_consistent": order_consistent,
        "position_bias_kind": position_bias_kind,
        "pass_1": pass_1,
        "pass_2": pass_2,
        "rubric": combined_rubric,
    }


def bootstrap_ci(
    values: list[float],
    seed: int = 20261107,
    n_resamples: int = 2000,
    alpha: float = 0.05,
) -> dict:
    """Compute non-parametric bootstrap mean and (1-alpha) confidence interval."""
    n = len(values)
    if n == 0:
        return {"estimate": None, "ci95_low": None, "ci95_high": None, "n": 0, "n_resamples": 0}
    estimate = statistics.mean(values)
    if n == 1 or len(set(values)) == 1:
        val = round(estimate, 4)
        return {"estimate": val, "ci95_low": val, "ci95_high": val, "n": n, "n_resamples": n_resamples}
    rng = random.Random(seed)
    means = []
    for _ in range(n_resamples):
        sample = [values[rng.randrange(n)] for _ in range(n)]
        means.append(sum(sample) / n)
    means.sort()
    low_idx = max(0, int(math.floor((alpha / 2.0) * n_resamples)))
    high_idx = min(n_resamples - 1, int(math.ceil((1.0 - alpha / 2.0) * n_resamples)) - 1)
    return {
        "estimate": round(estimate, 4),
        "ci95_low": round(means[low_idx], 4),
        "ci95_high": round(means[high_idx], 4),
        "n": n,
        "n_resamples": n_resamples,
    }


def cluster_bootstrap_ci(
    values: list[float],
    cluster_ids: list[str] | None = None,
    seed: int = 20261107,
    n_resamples: int = 2000,
    alpha: float = 0.05,
) -> dict:
    """Compute cluster-bootstrap mean and (1-alpha) CI resampled by cluster (brief_id) across seeds."""
    n = len(values)
    if n == 0:
        return {
            "estimate": None,
            "ci95_low": None,
            "ci95_high": None,
            "n": 0,
            "n_clusters": 0,
            "clustered_by": "brief_id",
            "n_resamples": 0,
        }
    if not cluster_ids or len(cluster_ids) != n:
        base = bootstrap_ci(values, seed=seed, n_resamples=n_resamples, alpha=alpha)
        return {**base, "n_clusters": n, "clustered_by": "pair"}

    unique_clusters = list(dict.fromkeys(cluster_ids))
    k = len(unique_clusters)
    if k == n:
        base = bootstrap_ci(values, seed=seed, n_resamples=n_resamples, alpha=alpha)
        return {**base, "n_clusters": k, "clustered_by": "brief_id"}

    estimate = statistics.mean(values)
    if k <= 1 or len(set(values)) == 1:
        val = round(estimate, 4)
        return {
            "estimate": val,
            "ci95_low": val,
            "ci95_high": val,
            "n": n,
            "n_clusters": k,
            "clustered_by": "brief_id",
            "n_resamples": n_resamples,
        }

    by_cluster: dict[str, list[float]] = {cid: [] for cid in unique_clusters}
    for val, cid in zip(values, cluster_ids):
        by_cluster[cid].append(val)

    rng = random.Random(seed)
    means = []
    for _ in range(n_resamples):
        sampled_vals: list[float] = []
        for _ in range(k):
            sampled_vals.extend(by_cluster[unique_clusters[rng.randrange(k)]])
        means.append(sum(sampled_vals) / len(sampled_vals))
    means.sort()
    low_idx = max(0, int(math.floor((alpha / 2.0) * n_resamples)))
    high_idx = min(n_resamples - 1, int(math.ceil((1.0 - alpha / 2.0) * n_resamples)) - 1)
    return {
        "estimate": round(estimate, 4),
        "ci95_low": round(means[low_idx], 4),
        "ci95_high": round(means[high_idx], 4),
        "n": n,
        "n_clusters": k,
        "clustered_by": "brief_id",
        "n_resamples": n_resamples,
    }


def cohens_kappa(labels_a: list[str], labels_b: list[str], categories: tuple[str, ...] = VERDICT_LABELS) -> float | None:
    """Compute Cohen's kappa for two raters over fixed categories."""
    if len(labels_a) != len(labels_b) or not labels_a:
        return None
    n = len(labels_a)
    observed = sum(1 for a, b in zip(labels_a, labels_b) if a == b) / n
    counts_a = Counter(labels_a)
    counts_b = Counter(labels_b)
    expected = sum((counts_a[cat] / n) * (counts_b[cat] / n) for cat in categories)
    if abs(1.0 - expected) < 1e-12:
        return 1.0 if abs(1.0 - observed) < 1e-12 else 0.0
    return round((observed - expected) / (1.0 - expected), 4)


def fleiss_kappa(ratings_per_item: list[list[str]], categories: tuple[str, ...] = VERDICT_LABELS) -> float | None:
    """Compute Fleiss' kappa across multiple raters per item."""
    if not ratings_per_item:
        return None
    n_items = len(ratings_per_item)
    n_raters = len(ratings_per_item[0])
    if n_raters < 2 or any(len(r) != n_raters for r in ratings_per_item):
        return None
    total_assignments = n_items * n_raters
    cat_totals = Counter()
    p_items = []
    for row in ratings_per_item:
        c = Counter(row)
        for cat in categories:
            cat_totals[cat] += c[cat]
        num = sum(c[cat] * (c[cat] - 1) for cat in categories)
        p_items.append(num / (n_raters * (n_raters - 1)))
    p_bar = sum(p_items) / n_items
    p_e = sum((cat_totals[cat] / total_assignments) ** 2 for cat in categories)
    if abs(1.0 - p_e) < 1e-12:
        return 1.0 if abs(1.0 - p_bar) < 1e-12 else 0.0
    return round((p_bar - p_e) / (1.0 - p_e), 4)


def panel_consensus(verdicts: list[str]) -> str:
    """Compute majority/plurality verdict across a panel of judges, breaking ties to 'tie'."""
    c = Counter(verdicts)
    if c["candidate"] > c["baseline"] and c["candidate"] > c["tie"]:
        return "candidate"
    if c["baseline"] > c["candidate"] and c["baseline"] > c["tie"]:
        return "baseline"
    return "tie"


def summarize_verdicts(
    verdicts: list[str],
    seed: int = 20261107,
    cluster_ids: list[str] | None = None,
) -> dict:
    """Compute win rates and 95% (cluster) bootstrap confidence intervals from a list of verdicts."""
    n = len(verdicts)
    c_wins = sum(1 for v in verdicts if v == "candidate")
    b_wins = sum(1 for v in verdicts if v == "baseline")
    ties = sum(1 for v in verdicts if v == "tie")

    tie_half_scores = [1.0 if v == "candidate" else (0.5 if v == "tie" else 0.0) for v in verdicts]
    c_outright_scores = [1.0 if v == "candidate" else 0.0 for v in verdicts]
    b_outright_scores = [1.0 if v == "baseline" else 0.0 for v in verdicts]
    n_clusters = len(dict.fromkeys(cluster_ids)) if cluster_ids else n

    return {
        "n_pairs": n,
        "n_clusters": n_clusters,
        "candidate_wins": c_wins,
        "baseline_wins": b_wins,
        "ties": ties,
        "candidate_win_rate_tie_half": cluster_bootstrap_ci(tie_half_scores, cluster_ids=cluster_ids, seed=seed),
        "candidate_outright_win_rate": cluster_bootstrap_ci(c_outright_scores, cluster_ids=cluster_ids, seed=seed + 1),
        "baseline_outright_win_rate": cluster_bootstrap_ci(b_outright_scores, cluster_ids=cluster_ids, seed=seed + 2),
        "tie_rate": round(ties / n, 4) if n else None,
    }


def summarize_position_bias(double_judgments: list[dict]) -> dict:
    """Compute position-bias and order-consistency metrics across double-judged pairs."""
    n = len(double_judgments)
    if n == 0:
        return {"n_double_judged": 0, "position_bias_rate": None}
    inconsistent = [j for j in double_judgments if not j["order_consistent"]]
    first_bias = sum(1 for j in inconsistent if j["position_bias_kind"] == "first_position")
    second_bias = sum(1 for j in inconsistent if j["position_bias_kind"] == "second_position")
    tie_flip = sum(1 for j in inconsistent if j["position_bias_kind"] == "tie_flip")

    total_passes = 2 * n
    p1_picks = sum(
        (1 if j["pass_1"]["raw_overall_preference"] == "passage_1" else 0)
        + (1 if j["pass_2"]["raw_overall_preference"] == "passage_1" else 0)
        for j in double_judgments
    )
    p2_picks = sum(
        (1 if j["pass_1"]["raw_overall_preference"] == "passage_2" else 0)
        + (1 if j["pass_2"]["raw_overall_preference"] == "passage_2" else 0)
        for j in double_judgments
    )

    return {
        "n_double_judged": n,
        "order_consistent_count": n - len(inconsistent),
        "order_consistency_rate": round((n - len(inconsistent)) / n, 4),
        "position_biased_count": len(inconsistent),
        "position_bias_rate": round(len(inconsistent) / n, 4),
        "both_picked_first_count": first_bias,
        "both_picked_second_count": second_bias,
        "tie_flip_count": tie_flip,
        "first_position_preference_rate": round(p1_picks / total_passes, 4),
        "second_position_preference_rate": round(p2_picks / total_passes, 4),
    }


def compute_inter_judge_agreement(judge_ids: list[str], by_judge_verdicts: dict[str, list[str]]) -> dict:
    """Compute pairwise agreement, Cohen's kappa, and multi-rater Fleiss' kappa across judges."""
    pairwise = []
    for ja, jb in itertools.combinations(judge_ids, 2):
        va = by_judge_verdicts[ja]
        vb = by_judge_verdicts[jb]
        n = len(va)
        raw_agree = sum(1 for a, b in zip(va, vb) if a == b) / n if n else None
        non_tie = [(a, b) for a, b in zip(va, vb) if a != "tie" and b != "tie"]
        dir_agree = (sum(1 for a, b in non_tie if a == b) / len(non_tie)) if non_tie else None
        pairwise.append({
            "judge_a": ja,
            "judge_b": jb,
            "n_pairs": n,
            "raw_agreement_rate": round(raw_agree, 4) if raw_agree is not None else None,
            "directional_agreement_rate": round(dir_agree, 4) if dir_agree is not None else None,
            "cohens_kappa": cohens_kappa(va, vb),
        })

    if not judge_ids:
        return {"n_judges": 0, "pairwise": [], "fleiss_kappa": None, "unanimous_rate": None}

    n_items = len(by_judge_verdicts[judge_ids[0]])
    per_item = [[by_judge_verdicts[jid][i] for jid in judge_ids] for i in range(n_items)]
    unanimous = sum(1 for row in per_item if len(set(row)) == 1) / n_items if n_items else None
    return {
        "n_judges": len(judge_ids),
        "pairwise": pairwise,
        "fleiss_kappa": fleiss_kappa(per_item) if len(judge_ids) >= 2 else None,
        "unanimous_rate": round(unanimous, 4) if unanimous is not None else None,
    }


# =============================================================================
# Calibration Workflow (~100 pairs human blinding + judge-human agreement)
# =============================================================================

def expand_pairs_for_calibration(
    base_pairs: list[dict],
    briefs_by_id: dict[str, dict],
    target_count: int = 100,
    seed: int = 20261107,
) -> list[dict]:
    """Build a calibration set of ~100 pairs drawn across the scene briefs while preserving pair provenance."""
    if not base_pairs or target_count <= 0:
        return []

    rng = random.Random(seed)
    expanded: list[dict] = []
    used_pair_ids: set[str] = set()
    variant_round = 0
    while len(expanded) < target_count:
        variant_round += 1
        cycle_pairs = list(base_pairs)
        if variant_round > 1:
            rng.shuffle(cycle_pairs)
        for p in cycle_pairs:
            if len(expanded) >= target_count:
                break
            cid = p["case_id"]
            brief = briefs_by_id.get(cid, {})
            orig_pid = str(p["pair_id"])
            existing_spid = p.get("source_pair_id")
            if existing_spid and orig_pid.startswith(f"{existing_spid}#cal"):
                source_pid = str(existing_spid)
            else:
                source_pid = orig_pid
            if variant_round == 1 and orig_pid not in used_pair_ids:
                cal_pid = orig_pid
            else:
                cal_pid = f"{orig_pid}#cal{variant_round}"
                suffix = 2
                while cal_pid in used_pair_ids:
                    cal_pid = f"{orig_pid}#cal{variant_round}_{suffix}"
                    suffix += 1
            used_pair_ids.add(cal_pid)

            base_copy = dict(p["baseline"])
            cand_copy = dict(p["candidate"])
            if variant_round > 1:
                # Distinct pair sampling tag so each of the ~100 calibration pairs has unique provenance
                base_copy["model_version"] = f"{base_copy.get('model_version', 'v1')}-cal-r{variant_round}"
                cand_copy["model_version"] = f"{cand_copy.get('model_version', 'v1')}-cal-r{variant_round}"
            expanded.append({
                "pair_id": cal_pid,
                "source_pair_id": source_pid,
                "case_id": cid,
                "brief_id": cid,
                "task": p.get("task") or brief.get("task", ""),
                "contract": p.get("contract") or brief.get("contract", ""),
                "scene_purpose": p.get("scene_purpose") or brief.get("scene_purpose", ""),
                "continuity_checklist": p.get("continuity_checklist") or brief.get("criteria", []),
                "baseline_text_hash": digest(base_copy["text"]),
                "candidate_text_hash": digest(cand_copy["text"]),
                "baseline": base_copy,
                "candidate": cand_copy,
            })
    return expanded


def compute_calibration_agreement(
    key_path: Path | str,
    ratings_path: Path | str,
    judge_results: dict,
) -> dict:
    """Compute judge-human agreement metrics from a scored pairwise human packet and judge results."""
    human_scored = serial_fiction.score_pairs(key_path, ratings_path)

    pair_records = judge_results.get("pairs", [])
    judge_ids = [j["id"] for j in judge_results.get("judges", [])]

    aligned_human: list[str] = []
    aligned_consensus: list[str] = []
    aligned_by_judge: dict[str, list[str]] = {jid: [] for jid in judge_ids}

    judge_by_id: dict[str, dict] = {}
    judge_by_content: dict[tuple[str, str, str], list[dict]] = {}
    for pr in pair_records:
        pid = pr.get("pair_id")
        spid = pr.get("source_pair_id") or pid
        bid = pr.get("brief_id") or pr.get("case_id")
        b_hash = pr.get("baseline_text_hash")
        c_hash = pr.get("candidate_text_hash")
        if not pid or not bid or not b_hash or not c_hash:
            raise ValueError(
                f"Judge pair record {pid!r} is missing required provenance fields "
                "(brief_id, baseline_text_hash, candidate_text_hash)"
            )
        if pid in judge_by_id:
            raise ValueError(f"Duplicate pair_id {pid!r} in judge_results")
        judge_by_id[pid] = pr
        if spid and spid not in judge_by_id:
            judge_by_id[spid] = pr
        judge_by_content.setdefault((bid, b_hash, c_hash), []).append(pr)

    for row in human_scored.get("deblinded_pairs", []):
        h_verdict = row.get("human_verdict")
        if h_verdict not in VERDICT_LABELS:
            continue
        h_pid = row["pair_id"]
        h_spid = row.get("source_pair_id") or (h_pid.split("#cal", 1)[0] if "#cal" in h_pid else h_pid)
        h_cid = row["case_id"]
        h_b_hash = row.get("baseline_text_hash")
        h_c_hash = row.get("candidate_text_hash")
        if not h_b_hash or not h_c_hash:
            raise ValueError(
                f"Human calibration row {h_pid!r} is missing baseline_text_hash or candidate_text_hash"
            )

        pr = judge_by_id.get(h_pid) or judge_by_id.get(h_spid)
        if pr is not None:
            pr_bid = pr.get("brief_id") or pr.get("case_id")
            if (
                pr_bid != h_cid
                or pr.get("baseline_text_hash") != h_b_hash
                or pr.get("candidate_text_hash") != h_c_hash
            ):
                raise ValueError(
                    f"Calibration pair {h_pid!r} (source={h_spid!r}, case={h_cid!r}) does not match "
                    f"judged pair {pr.get('pair_id')!r} (brief={pr_bid!r}) prose content hashes"
                )
        else:
            content_matches = judge_by_content.get((h_cid, h_b_hash, h_c_hash), [])
            if len(content_matches) == 1:
                pr = content_matches[0]
            elif len(content_matches) > 1:
                raise ValueError(
                    f"Ambiguous calibration match for {h_pid!r} on brief {h_cid!r}: "
                    "multiple judged pairs share identical hashes without matching source_pair_id"
                )
            else:
                raise ValueError(
                    f"No matching judged prose pair found for calibration row {h_pid!r} "
                    f"(source={h_spid!r}, case={h_cid!r})"
                )

        aligned_human.append(h_verdict)
        aligned_consensus.append(pr["panel_consensus"])
        per_j = {jr["judge_id"]: jr["verdict"] for jr in pr.get("judge_evaluations", [])}
        for jid in judge_ids:
            if jid in per_j:
                aligned_by_judge[jid].append(per_j[jid])

    def _agreement_block(h_list: list[str], j_list: list[str], seed_offset: int = 0) -> dict:
        n = len(h_list)
        if n == 0 or len(j_list) != n:
            return {
                "n_aligned": 0,
                "raw_agreement_rate": None,
                "directional_agreement_rate": None,
                "cohens_kappa": None,
                "confusion_matrix": {},
            }
        matches = [1.0 if h == j else 0.0 for h, j in zip(h_list, j_list)]
        non_tie = [(h, j) for h, j in zip(h_list, j_list) if h != "tie" and j != "tie"]
        dir_rate = round(sum(1 for h, j in non_tie if h == j) / len(non_tie), 4) if non_tie else None
        matrix = {
            f"human_{hc}": {
                f"judge_{jc}": sum(1 for h, j in zip(h_list, j_list) if h == hc and j == jc)
                for jc in VERDICT_LABELS
            }
            for hc in VERDICT_LABELS
        }
        h_cand_rate = sum(1 for h in h_list if h == "candidate") / n
        j_cand_rate = sum(1 for j in j_list if j == "candidate") / n
        h_tie_rate = sum(1 for h in h_list if h == "tie") / n
        j_tie_rate = sum(1 for j in j_list if j == "tie") / n
        return {
            "n_aligned": n,
            "raw_agreement_rate": bootstrap_ci(matches, seed=20261107 + seed_offset),
            "directional_agreement_rate": dir_rate,
            "cohens_kappa": cohens_kappa(h_list, j_list),
            "confusion_matrix": matrix,
            "bias_vs_human": {
                "judge_minus_human_candidate_rate": round(j_cand_rate - h_cand_rate, 4),
                "judge_minus_human_tie_rate": round(j_tie_rate - h_tie_rate, 4),
            },
        }

    per_judge_cal = {
        jid: _agreement_block(aligned_human, aligned_by_judge[jid], idx + 1)
        for idx, jid in enumerate(judge_ids)
    }

    return {
        "total_calibration_pairs": len(human_scored.get("deblinded_pairs", [])),
        "human_pairs_rated": human_scored["preference_pairs_rated"],
        "human_pairs_unrated": human_scored["preference_pairs_unrated"],
        "human_summary": human_scored["conditions"],
        "panel_consensus_vs_human": _agreement_block(aligned_human, aligned_consensus, 0),
        "per_judge_vs_human": per_judge_cal,
        "interpretation": (
            "Judge-human agreement measures how closely automated rubric judgments track blinded "
            "human ratings on the labeled subset. Unrated pairs are reported honestly and excluded "
            "from agreement denominators."
        ),
    }


def run_evaluation(
    briefs: list[dict],
    pairs: list[dict],
    judges: list[dict],
    seed: int = 20261107,
    calibration_key: Path | str | None = None,
    calibration_ratings: Path | str | None = None,
    checkpoint_path: Path | str | None = None,
) -> dict:
    """Execute full randomized + swapped-order double judging across the judge panel."""
    if not judges:
        raise ValueError("At least one judge configuration is required")
    families = {j.get("family", j["id"]) for j in judges}
    if len(judges) > 1 and len(families) < 2:
        raise ValueError("Multi-judge evaluation requires judges from at least 2 distinct model families")

    brief_map = {b["id"]: b for b in briefs}
    all_double_judgments: list[dict] = []
    by_judge_judgments: dict[str, list[dict]] = {j["id"]: [] for j in judges}
    by_judge_verdicts: dict[str, list[str]] = {j["id"]: [] for j in judges}
    consensus_verdicts: list[str] = []
    pair_reports: list[dict] = []

    cached_pairs: dict[str, dict] = {}
    ckpt_file = Path(checkpoint_path) if checkpoint_path else None
    if ckpt_file and ckpt_file.exists():
        try:
            raw_ckpt = read_json(ckpt_file)
            if isinstance(raw_ckpt, list):
                for row in raw_ckpt:
                    if isinstance(row, dict) and row.get("pair_id") and isinstance(row.get("judge_evaluations"), list):
                        cached_pairs[str(row["pair_id"])] = row
        except Exception:
            pass

    use_parallel_judges = any(str(j.get("provider", "fake")).lower() != "fake" for j in judges) and len(judges) > 1

    for pair in pairs:
        validate_candidate_completeness(pair["baseline"], arm_label="baseline")
        validate_candidate_completeness(pair["candidate"], arm_label="candidate")
        bid = pair.get("brief_id") or pair["case_id"]
        brief = brief_map[bid]
        pid_str = str(pair["pair_id"])
        base_hash = digest(pair["baseline"]["text"])
        cand_hash = digest(pair["candidate"]["text"])
        prompt_seed_fp = compute_pair_prompt_seed_fingerprint(brief, pair, seed)
        pair_judgment_fp = compute_pair_judgment_fingerprint(brief, pair, judges, seed)
        cached_row = cached_pairs.get(pid_str)
        reused_evals: list[dict] | None = None
        if (
            cached_row
            and cached_row.get("baseline_text_hash") == base_hash
            and cached_row.get("candidate_text_hash") == cand_hash
            and cached_row.get("judgment_prompt_seed_fingerprint") == prompt_seed_fp
        ):
            cached_eval_by_id = {
                dj.get("judge_id"): dj
                for dj in cached_row.get("judge_evaluations", [])
                if isinstance(dj, dict) and dj.get("judge_id")
            }
            if set(cached_eval_by_id.keys()) == {j["id"] for j in judges}:
                matched_all = True
                ordered_evals: list[dict] = []
                for judge_cfg in judges:
                    jid = judge_cfg["id"]
                    dj = cached_eval_by_id.get(jid)
                    expected_j_fp = compute_judge_pair_input_fingerprint(judge_cfg, brief, pair, seed)
                    if not dj or dj.get("judgment_input_fingerprint") != expected_j_fp:
                        matched_all = False
                        break
                    ordered_evals.append(dj)
                if matched_all:
                    reused_evals = ordered_evals

        if reused_evals is not None and cached_row is not None:
            evals_for_pair = reused_evals
            for dj in evals_for_pair:
                jid = dj["judge_id"]
                all_double_judgments.append(dj)
                by_judge_judgments[jid].append(dj)
                by_judge_verdicts[jid].append(dj["verdict"])
            consensus = panel_consensus([e["verdict"] for e in evals_for_pair])
            consensus_verdicts.append(consensus)
            updated_row = {
                **cached_row,
                "judgment_prompt_seed_fingerprint": prompt_seed_fp,
                "judgment_checkpoint_fingerprint": pair_judgment_fp,
                "panel_consensus": consensus,
                "judge_evaluations": evals_for_pair,
            }
            pair_reports.append(updated_row)
            continue

        if use_parallel_judges:
            with concurrent.futures.ThreadPoolExecutor(max_workers=len(judges)) as pool:
                futures = [
                    pool.submit(judge_pair_double, judge_cfg, brief, pair, seed)
                    for judge_cfg in judges
                ]
                evals_for_pair = [f.result() for f in futures]
        else:
            evals_for_pair = [
                judge_pair_double(judge_cfg, brief, pair, seed=seed)
                for judge_cfg in judges
            ]
        for dj in evals_for_pair:
            jid = dj["judge_id"]
            all_double_judgments.append(dj)
            by_judge_judgments[jid].append(dj)
            by_judge_verdicts[jid].append(dj["verdict"])

        consensus = panel_consensus([e["verdict"] for e in evals_for_pair])
        consensus_verdicts.append(consensus)
        existing_spid = pair.get("source_pair_id")
        source_pid = (
            str(existing_spid)
            if (existing_spid and pid_str.startswith(f"{existing_spid}#cal"))
            else pid_str
        )
        gseed = pair.get("generation_seed")
        if gseed is None:
            gseed = pair["candidate"].get("generation_seed", pair["baseline"].get("generation_seed"))

        def _extract_meta(arm: dict) -> dict:
            meta = {
                "model": arm.get("model"),
                "model_version": arm.get("model_version"),
                "prompt_variant": arm.get("prompt_variant"),
                "generation_seed": arm.get("generation_seed", gseed),
                "generation_input_fingerprint": arm.get("generation_input_fingerprint"),
                "truncated": bool(arm.get("truncated", False)),
                "text_sha256": arm.get("text_sha256"),
                "accepted_text_sha256": arm.get("accepted_text_sha256"),
                "packet_hash": arm.get("packet_hash"),
                "source_packet_sha256": arm.get("source_packet_sha256"),
                "prompt_hash": arm.get("prompt_hash"),
                "tokenizer_id": arm.get("tokenizer_id"),
                "tokenizer_version": arm.get("tokenizer_version"),
                "settings": arm.get("settings"),
                "artifact_id": arm.get("artifact_id"),
                "artifact_path": arm.get("artifact_path"),
                "input_tokens": arm.get("input_tokens"),
                "output_tokens": arm.get("output_tokens"),
            }
            return {k: v for k, v in meta.items() if v is not None}

        pair_reports.append({
            "pair_id": pair["pair_id"],
            "source_pair_id": source_pid,
            "brief_id": bid,
            "generation_seed": gseed,
            "name": brief["name"],
            "project_id": brief["project_id"],
            "genre": brief["genre"],
            "scene_type": brief["scene_type"],
            "pov": brief["pov"],
            "baseline_text_hash": base_hash,
            "candidate_text_hash": cand_hash,
            "judgment_prompt_seed_fingerprint": prompt_seed_fp,
            "judgment_checkpoint_fingerprint": pair_judgment_fp,
            "baseline_meta": _extract_meta(pair["baseline"]),
            "candidate_meta": _extract_meta(pair["candidate"]),
            "panel_consensus": consensus,
            "judge_evaluations": evals_for_pair,
        })
        if ckpt_file:
            write_json(ckpt_file, pair_reports)

    all_cluster_ids = [pr["brief_id"] for pr in pair_reports]

    # Rubric dimension aggregation
    rubric_summary = {}
    for dim in RUBRIC_DIMENSIONS:
        b_scores = [dj["rubric"][dim]["baseline_score"] for dj in all_double_judgments]
        c_scores = [dj["rubric"][dim]["candidate_score"] for dj in all_double_judgments]
        dim_verdicts = [dj["rubric"][dim]["winner"] for dj in all_double_judgments]
        b_mean = round(statistics.mean(b_scores), 4) if b_scores else None
        c_mean = round(statistics.mean(c_scores), 4) if c_scores else None
        c_wins = sum(1 for w in dim_verdicts if w == "candidate")
        b_wins = sum(1 for w in dim_verdicts if w == "baseline")
        ties = sum(1 for w in dim_verdicts if w == "tie")
        rubric_summary[dim] = {
            "baseline_mean_score": b_mean,
            "candidate_mean_score": c_mean,
            "mean_score_delta": round(c_mean - b_mean, 4) if (b_mean is not None and c_mean is not None) else None,
            "candidate_dim_win_rate_tie_half": round((c_wins + 0.5 * ties) / len(dim_verdicts), 4) if dim_verdicts else None,
            "candidate_wins": c_wins,
            "baseline_wins": b_wins,
            "ties": ties,
        }

    # Breakdown by scene_type, genre, and generation_seed
    by_scene_type = {}
    for stype in SCENE_TYPES:
        st_rows = [pr for pr in pair_reports if pr["scene_type"] == stype]
        if st_rows:
            by_scene_type[stype] = summarize_verdicts(
                [pr["panel_consensus"] for pr in st_rows],
                seed=seed,
                cluster_ids=[pr["brief_id"] for pr in st_rows],
            )

    by_genre = {}
    for genre in sorted({pr["genre"] for pr in pair_reports}):
        g_rows = [pr for pr in pair_reports if pr["genre"] == genre]
        by_genre[genre] = summarize_verdicts(
            [pr["panel_consensus"] for pr in g_rows],
            seed=seed,
            cluster_ids=[pr["brief_id"] for pr in g_rows],
        )

    by_generation_seed = {}
    present_seeds = [s for s in dict.fromkeys(pr.get("generation_seed") for pr in pair_reports) if s is not None]
    for gseed in present_seeds:
        s_rows = [pr for pr in pair_reports if pr.get("generation_seed") == gseed]
        by_generation_seed[str(gseed)] = summarize_verdicts(
            [pr["panel_consensus"] for pr in s_rows],
            seed=seed + int(gseed),
            cluster_ids=[pr["brief_id"] for pr in s_rows],
        )

    per_judge_summary = {}
    for j in judges:
        jid = j["id"]
        per_judge_summary[jid] = {
            "family": j.get("family", "unknown"),
            "provider": j.get("provider", "fake"),
            "model": j.get("model", "unknown"),
            "win_rates": summarize_verdicts(by_judge_verdicts[jid], seed=seed, cluster_ids=all_cluster_ids),
            "position_bias": summarize_position_bias(by_judge_judgments[jid]),
        }

    report = {
        "schema_version": 1,
        "suite_id": "spindle.prose_judge.v1",
        "seed": seed,
        "generation_seeds": present_seeds,
        "n_briefs_evaluated": len(pair_reports),
        "n_unique_briefs": len(dict.fromkeys(all_cluster_ids)),
        "judges": [
            {
                "id": j["id"],
                "family": j.get("family", "unknown"),
                "provider": j.get("provider", "fake"),
                "model": j.get("model", "unknown"),
            }
            for j in judges
        ],
        "panel_summary": summarize_verdicts(consensus_verdicts, seed=seed, cluster_ids=all_cluster_ids),
        "position_bias": summarize_position_bias(all_double_judgments),
        "inter_judge_agreement": compute_inter_judge_agreement([j["id"] for j in judges], by_judge_verdicts),
        "rubric_summary": rubric_summary,
        "by_scene_type": by_scene_type,
        "by_genre": by_genre,
        "by_generation_seed": by_generation_seed,
        "per_judge": per_judge_summary,
        "calibration": None,
        "pairs": pair_reports,
        "interpretation": (
            "Pairwise LLM-judge evaluation with randomized initial order and swapped-order double judging. "
            "Order-inconsistent pairs are recorded as position-biased ties rather than awarded as wins. "
            "Confidence intervals are cluster-bootstrapped by brief_id across generation seeds. "
            "LLM-judge win rates are an automated proxy and do not establish human author preference "
            "without blinded human calibration (`prose_judge.py blind` / `calibrate`)."
        ),
    }

    if calibration_key and calibration_ratings:
        report["calibration"] = compute_calibration_agreement(calibration_key, calibration_ratings, report)

    return report


# =============================================================================
# Offline Self-Test & CLI Entrypoint
# =============================================================================

def self_test() -> None:
    briefs = load_briefs()
    errors = validate_brief_suite(briefs)
    if errors:
        print("Brief suite validation failed:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        raise SystemExit(1)
    assert len(briefs) == 30, f"Expected 30 briefs, got {len(briefs)}"

    # 1. Verify fixture pairs & offline multi-family double-judging
    jcfg = read_json(JUDGES_PATH)
    offline_judges = jcfg["offline_judges"]
    pairs = load_or_generate_pairs(briefs, pairs_path=FIXTURE_PAIRS_PATH)
    assert len(pairs) == 30

    report = run_evaluation(briefs, pairs, offline_judges, seed=20261107)
    ps = report["panel_summary"]
    assert ps["n_pairs"] == 30
    assert ps["candidate_wins"] == 22, f"Expected 22 candidate wins on fixture suite, got {ps['candidate_wins']}"
    assert ps["baseline_wins"] == 5, f"Expected 5 baseline wins on fixture suite, got {ps['baseline_wins']}"
    assert ps["ties"] == 3, f"Expected 3 ties on fixture suite, got {ps['ties']}"

    ci = ps["candidate_win_rate_tie_half"]
    assert ci["ci95_low"] <= ci["estimate"] <= ci["ci95_high"], ci
    assert 0.65 <= ci["estimate"] <= 0.85, ci
    assert report["position_bias"]["position_bias_rate"] == 0.0
    assert report["inter_judge_agreement"]["fleiss_kappa"] is not None
    assert report["inter_judge_agreement"]["fleiss_kappa"] >= 0.75

    # 2. Verify position-biased judge detection (swapped-order double judging catches first-position bias)
    biased_judges = [
        {
            "id": "biased-first-slot",
            "family": "biased-family-a",
            "provider": "fake",
            "model": "fake-biased-1",
            "profile": "position_biased_first",
        },
        {
            "id": "biased-second-slot",
            "family": "biased-family-b",
            "provider": "fake",
            "model": "fake-biased-2",
            "profile": "position_biased_second",
        },
    ]
    biased_report = run_evaluation(briefs[:6], pairs[:6], biased_judges, seed=99)
    assert biased_report["position_bias"]["position_bias_rate"] == 1.0
    assert biased_report["position_bias"]["both_picked_first_count"] == 6
    assert biased_report["position_bias"]["both_picked_second_count"] == 6
    # Order-inconsistent pairs must never be awarded as outright wins
    assert biased_report["panel_summary"]["candidate_wins"] == 0
    assert biased_report["panel_summary"]["baseline_wins"] == 0
    assert biased_report["panel_summary"]["ties"] == 6

    # 2b. Verify panel_consensus requires a strictly unique highest vote count for directional wins
    assert panel_consensus(["candidate", "tie"]) == "tie"
    assert panel_consensus(["baseline", "tie"]) == "tie"
    assert panel_consensus(["candidate", "baseline"]) == "tie"
    assert panel_consensus(["candidate", "candidate", "tie", "tie"]) == "tie"
    assert panel_consensus(["baseline", "baseline", "tie", "tie"]) == "tie"
    assert panel_consensus(["candidate", "candidate", "baseline", "baseline", "tie"]) == "tie"
    assert panel_consensus(["candidate", "candidate", "baseline", "tie"]) == "candidate"
    assert panel_consensus(["baseline", "baseline", "candidate", "tie"]) == "baseline"
    assert panel_consensus(["candidate", "candidate"]) == "candidate"
    assert panel_consensus(["baseline", "baseline"]) == "baseline"
    assert panel_consensus([]) == "tie"

    # 3. Verify generation adapter (both variant-driven and saved baseline/candidate files)
    gen_pairs = load_or_generate_pairs(
        briefs[:5],
        variant_a="compact_baseline",
        variant_b="spindle_full_context",
        generator_override={"provider": "fake", "model": "fake-drafter-test"},
    )
    assert len(gen_pairs) == 5
    gen_report = run_evaluation(briefs[:5], gen_pairs, offline_judges, seed=7)
    assert gen_report["panel_summary"]["candidate_wins"] == 5

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        base_file = root / "baseline.json"
        cand_file = root / "candidate.json"
        write_json(base_file, [p["baseline"] for p in pairs[:5]])
        write_json(cand_file, [p["candidate"] for p in pairs[:5]])
        loaded_from_split = load_or_generate_pairs(
            briefs[:5],
            baseline_path=base_file,
            candidate_path=cand_file,
        )
        assert len(loaded_from_split) == 5

        # 4. Verify 100-pair human calibration workflow reusing serial_fiction.py
        brief_map = {b["id"]: b for b in briefs}
        cal_pairs = expand_pairs_for_calibration(pairs, brief_map, target_count=100, seed=20261107)
        assert len(cal_pairs) == 100
        cal_dir = root / "blind_100"
        serial_fiction.blind_pairs(cal_pairs, cal_dir, seed=20261107)
        pkey = read_json(cal_dir / "private-key.json")
        pratings = read_json(cal_dir / "ratings.json")
        assert len(pkey["mapping"]) == 100
        # Populate simulated human labels matching reference expectations on first 40 pairs
        ref_by_case = {
            p["case_id"]: p["expected_reference_verdict"]
            for p in read_json(FIXTURE_PAIRS_PATH)["pairs"]
        }
        map_by_pid = {m["pair_id"]: m for m in pkey["mapping"]}
        for row in pratings[:40]:
            m = map_by_pid[row["pair_id"]]
            inv = {cond: lbl for lbl, cond in m["labels"].items()}
            exp = ref_by_case[m["case_id"]]
            if exp == "tie":
                row["preferred"] = ["A", "B"]
            else:
                row["preferred"] = [inv[exp]]
        write_json(cal_dir / "ratings.json", pratings)

        cal_agreement = compute_calibration_agreement(
            cal_dir / "private-key.json",
            cal_dir / "ratings.json",
            report,
        )
        assert cal_agreement["total_calibration_pairs"] == 100
        assert cal_agreement["human_pairs_rated"] == 40
        assert cal_agreement["human_pairs_unrated"] == 60
        assert cal_agreement["panel_consensus_vs_human"]["raw_agreement_rate"]["estimate"] == 1.0
        assert cal_agreement["panel_consensus_vs_human"]["cohens_kappa"] == 1.0

        # 4b. Verify reordered/custom non-sequential pair_ids and multiple distinct pairs per brief
        p1_base = dict(pairs[0])
        p2_base = dict(pairs[1])
        # Reordered: P002 (PJ02, swapped so baseline wins) first, P001 (PJ01, candidate wins) second,
        # plus a second distinct prose pair on PJ01 (CUSTOM-PJ01-SWAPPED, baseline wins)
        reordered_custom_pairs = [
            {
                **p2_base,
                "pair_id": "P002",
                "case_id": "PJ02",
                "brief_id": "PJ02",
                "baseline": {**p2_base["candidate"], "condition": "baseline"},
                "candidate": {**p2_base["baseline"], "condition": "candidate"},
            },
            {
                **p1_base,
                "pair_id": "P001",
                "case_id": "PJ01",
                "brief_id": "PJ01",
            },
            {
                **p1_base,
                "pair_id": "CUSTOM-PJ01-SWAPPED",
                "case_id": "PJ01",
                "brief_id": "PJ01",
                # Swap baseline and candidate prose so baseline is the stronger draft on this second PJ01 pair
                "baseline": {**p1_base["candidate"], "condition": "baseline"},
                "candidate": {**p1_base["baseline"], "condition": "candidate"},
            },
        ]
        custom_judge_report = run_evaluation(briefs[:2], reordered_custom_pairs, offline_judges, seed=20261107)
        assert [pr["pair_id"] for pr in custom_judge_report["pairs"]] == ["P002", "P001", "CUSTOM-PJ01-SWAPPED"]
        assert [pr["panel_consensus"] for pr in custom_judge_report["pairs"]] == ["baseline", "candidate", "baseline"]

        non_expanded_custom = expand_pairs_for_calibration(reordered_custom_pairs, brief_map, target_count=3, seed=20261107)
        assert [p["pair_id"] for p in non_expanded_custom] == ["P002", "P001", "CUSTOM-PJ01-SWAPPED"]
        expanded_custom = expand_pairs_for_calibration(reordered_custom_pairs, brief_map, target_count=6, seed=20261107)
        assert [p["pair_id"] for p in expanded_custom[:3]] == ["P002", "P001", "CUSTOM-PJ01-SWAPPED"]
        assert all(p["source_pair_id"] in {"P002", "P001", "CUSTOM-PJ01-SWAPPED"} for p in expanded_custom)

        custom_cal_dir = root / "blind_custom"
        serial_fiction.blind_pairs(expanded_custom, custom_cal_dir, seed=20261107)
        ckey = read_json(custom_cal_dir / "private-key.json")
        cratings = read_json(custom_cal_dir / "ratings.json")
        cmap = {m["pair_id"]: m for m in ckey["mapping"]}
        for row in cratings:
            m = cmap[row["pair_id"]]
            inv = {cond: lbl for lbl, cond in m["labels"].items()}
            # Human agrees with candidate ONLY on P001, and with baseline on P002 and CUSTOM-PJ01-SWAPPED
            expected_winner = "candidate" if m.get("source_pair_id") == "P001" else "baseline"
            row["preferred"] = [inv[expected_winner]]
        write_json(custom_cal_dir / "ratings.json", cratings)

        custom_cal = compute_calibration_agreement(
            custom_cal_dir / "private-key.json",
            custom_cal_dir / "ratings.json",
            custom_judge_report,
        )
        assert custom_cal["human_pairs_rated"] == 6
        assert custom_cal["panel_consensus_vs_human"]["raw_agreement_rate"]["estimate"] == 1.0
        assert custom_cal["panel_consensus_vs_human"]["cohens_kappa"] == 1.0

        # 4c. Reject mismatched prose / different drafting run or missing second pair on same brief
        corrupted_report = json.loads(json.dumps(custom_judge_report))
        corrupted_report["pairs"][0]["candidate_text_hash"] = "0" * 64
        try:
            compute_calibration_agreement(
                custom_cal_dir / "private-key.json",
                custom_cal_dir / "ratings.json",
                corrupted_report,
            )
            raise AssertionError("Expected ValueError on mismatched candidate_text_hash")
        except ValueError:
            pass

        incomplete_same_brief_report = json.loads(json.dumps(custom_judge_report))
        incomplete_same_brief_report["pairs"] = incomplete_same_brief_report["pairs"][:2]  # drop CUSTOM-PJ01-SWAPPED
        try:
            compute_calibration_agreement(
                custom_cal_dir / "private-key.json",
                custom_cal_dir / "ratings.json",
                incomplete_same_brief_report,
            )
            raise AssertionError("Expected ValueError when a distinct prose pair on the same brief is missing from judge results")
        except ValueError:
            pass

        # 4d. Verify DraftingPacketV1 conversion across all 30 briefs, multi-seed pairing,
        # cluster bootstrap CI, and Rust bridge adapter (draft-packet & draft-messages).
        for b in briefs:
            pkt = brief_to_drafting_packet_v1(b)
            assert pkt["schema_version"] == "drafting_packet_v1"
            assert pkt["project_id"].startswith("project:"), pkt["project_id"]
            assert pkt["branch_id"].startswith("bible_branch:"), pkt["branch_id"]
            assert len(pkt["scene_card"]["required_beats"]) >= 3
            assert len(pkt["hard_constraints"]) >= 2
            assert len(pkt["source_versions"]) >= 6
            assert pkt["scene_card"]["chapter"]["is_closer"] is (b["scene_type"] == "chapter-closer")

        multi_seed_pairs = load_or_generate_pairs(
            briefs,
            variant_a="compact_baseline",
            variant_b="drafting_packet_v1",
            generator_override={"provider": "fake", "model": "fake-drafter-test"},
            generation_seeds=[101, 202, 303],
        )
        assert len(multi_seed_pairs) == 90
        assert multi_seed_pairs[0]["pair_id"] == "P001-S101"
        assert multi_seed_pairs[1]["pair_id"] == "P001-S202"
        assert multi_seed_pairs[2]["pair_id"] == "P001-S303"
        assert multi_seed_pairs[-1]["pair_id"] == "P030-S303"
        assert multi_seed_pairs[0]["candidate"]["packet_hash"] is not None
        assert multi_seed_pairs[0]["candidate"]["source_packet_sha256"] is not None
        assert (
            multi_seed_pairs[0]["candidate"]["source_packet_sha256"]
            == multi_seed_pairs[1]["candidate"]["source_packet_sha256"]
        )

        # Save and reload multi-seed baseline/candidate files
        ms_base_file = root / "ms_baseline.json"
        ms_cand_file = root / "ms_candidate.json"
        write_json(ms_base_file, [p["baseline"] for p in multi_seed_pairs[:6]])
        write_json(ms_cand_file, [p["candidate"] for p in multi_seed_pairs[:6]])
        reloaded_ms = load_or_generate_pairs(
            briefs[:2],
            baseline_path=ms_base_file,
            candidate_path=ms_cand_file,
        )
        assert len(reloaded_ms) == 6
        assert [p["pair_id"] for p in reloaded_ms] == [
            "P001-S101",
            "P001-S202",
            "P001-S303",
            "P002-S101",
            "P002-S202",
            "P002-S303",
        ]

        ms_report = run_evaluation(briefs[:6], multi_seed_pairs[:18], offline_judges, seed=20261107)
        assert ms_report["n_briefs_evaluated"] == 18
        assert ms_report["n_unique_briefs"] == 6
        assert ms_report["panel_summary"]["n_pairs"] == 18
        assert ms_report["panel_summary"]["n_clusters"] == 6
        assert ms_report["panel_summary"]["candidate_win_rate_tie_half"]["clustered_by"] == "brief_id"
        assert set(ms_report["by_generation_seed"].keys()) == {"101", "202", "303"}
        assert ms_report["pairs"][0]["candidate_meta"]["generation_seed"] == 101
        assert ms_report["pairs"][0]["candidate_meta"]["source_packet_sha256"] is not None

        # Verify mock Rust bridge executable for both draft-packet and draft-messages
        mock_bin = root / "mock-spindle-tokenizer"
        mock_bin.write_text(
            "#!/usr/bin/env python3\n"
            "import hashlib, json, sys\n"
            "args = sys.argv[1:]\n"
            "cmd = args[0]\n"
            "if cmd == 'draft-packet':\n"
            "    cfg, db, pkt_path, art_path = args[1:5]\n"
            "    seed = int(args[5]) if len(args) > 5 else None\n"
            "    pkt = json.loads(open(pkt_path, encoding='utf-8').read())\n"
            "    text = f\"Packet prose for {pkt['scene_id']} seed={seed}\"\n"
            "    sha_pkt = hashlib.sha256(text.encode('utf-8')).hexdigest()\n"
            "    truncated = (seed == 999)\n"
            "    accepted_sha = None if seed in (998, 999) else sha_pkt\n"
            "    print(json.dumps({\n"
            "        'mode': 'draft_packet',\n"
            "        'artifact_id': 'draft_artifact:test',\n"
            "        'artifact_path': art_path,\n"
            "        'packet_hash': 'pkt_hash_123',\n"
            "        'source_packet_sha256': 'src_pkt_456',\n"
            "        'prompt_hash': 'pkt_hash_123',\n"
            "        'generation_seed': seed,\n"
            "        'model': 'gemma4:31b',\n"
            "        'model_call_id': 'call_pkt_1',\n"
            "        'tokenizer_id': 'hf_json',\n"
            "        'tokenizer_version': 'gemma4',\n"
            "        'input_tokens': 512,\n"
            "        'output_tokens': 410,\n"
            "        'truncated': truncated,\n"
            "        'text_sha256': sha_pkt,\n"
            "        'accepted_text_sha256': accepted_sha,\n"
            "        'text': text,\n"
            "        'settings': {'transport': 'ollama_native_chat', 'context_window': 32768, 'max_tokens': 1200, 'temperature': 0.4, 'seed': seed, 'model_revision': 'rev1'}\n"
            "    }))\n"
            "elif cmd == 'draft-messages':\n"
            "    cfg, db, route, rating, msg_path = args[1:6]\n"
            "    seed = int(args[6]) if len(args) > 6 else None\n"
            "    msg = json.loads(open(msg_path, encoding='utf-8').read())\n"
            "    text = f\"Legacy prose len={len(msg['user'])} seed={seed}\"\n"
            "    sha_msg = hashlib.sha256(text.encode('utf-8')).hexdigest()\n"
            "    truncated = (seed == 999)\n"
            "    print(json.dumps({\n"
            "        'mode': 'draft_messages',\n"
            "        'prompt_hash': 'msg_hash_789',\n"
            "        'generation_seed': seed,\n"
            "        'model': 'gemma4:31b',\n"
            "        'model_call_id': 'call_msg_1',\n"
            "        'tokenizer_id': 'hf_json',\n"
            "        'tokenizer_version': 'gemma4',\n"
            "        'input_tokens': 480,\n"
            "        'output_tokens': 395,\n"
            "        'truncated': truncated,\n"
            "        'text_sha256': sha_msg,\n"
            "        'text': text,\n"
            "        'settings': {'transport': 'ollama_native_chat', 'context_window': 32768, 'max_tokens': 1200, 'temperature': 0.4, 'seed': seed, 'model_revision': 'rev1'}\n"
            "    }))\n",
            encoding="utf-8",
        )
        mock_bin.chmod(0o755)
        variants = read_json(VARIANTS_PATH)["variants"]
        bridge_cfg = {
            "provider": "rust-bridge",
            "rust_config": str(root / "agents.toml"),
            "rust_db": str(root / "usage.db"),
            "artifact_dir": str(root / "bridge_out"),
            "tokenizer_binary": str(mock_bin),
        }
        bridge_pkt_cand = generate_candidate_for_brief(
            briefs[0],
            variants["drafting_packet_v1"],
            bridge_cfg,
            condition_label="candidate",
            generation_seed=101,
        )
        assert bridge_pkt_cand["packet_hash"] == "pkt_hash_123"
        assert bridge_pkt_cand["source_packet_sha256"] == "src_pkt_456"
        assert bridge_pkt_cand["generation_seed"] == 101
        assert bridge_pkt_cand["truncated"] is False
        assert bridge_pkt_cand["accepted_text_sha256"] == bridge_pkt_cand["text_sha256"]
        assert bridge_pkt_cand["settings"]["context_window"] == 32768
        assert "seed=101" in bridge_pkt_cand["text"]

        bridge_msg_cand = generate_candidate_for_brief(
            briefs[0],
            variants["spindle_full_context"],
            bridge_cfg,
            condition_label="baseline",
            generation_seed=101,
        )
        assert bridge_msg_cand["prompt_hash"] == "msg_hash_789"
        assert bridge_msg_cand["generation_seed"] == 101
        assert bridge_msg_cand["truncated"] is False
        assert bridge_msg_cand["accepted_text_sha256"] is None
        assert bridge_msg_cand["settings"]["context_window"] == 32768

        # Verify truncated=True and unaccepted packet outputs are rejected before checkpointing/judging
        try:
            generate_candidate_for_brief(
                briefs[0],
                variants["drafting_packet_v1"],
                bridge_cfg,
                condition_label="candidate",
                generation_seed=999,
            )
            raise AssertionError("Expected RuntimeError for truncated=True packet bridge output")
        except RuntimeError as exc:
            assert "truncated=True" in str(exc)

        try:
            generate_candidate_for_brief(
                briefs[0],
                variants["spindle_full_context"],
                bridge_cfg,
                condition_label="baseline",
                generation_seed=999,
            )
            raise AssertionError("Expected RuntimeError for truncated=True message bridge output")
        except RuntimeError as exc:
            assert "truncated=True" in str(exc)

        try:
            generate_candidate_for_brief(
                briefs[0],
                variants["drafting_packet_v1"],
                bridge_cfg,
                condition_label="candidate",
                generation_seed=998,
            )
            raise AssertionError("Expected RuntimeError for unaccepted packet bridge output (accepted_text_sha256=None)")
        except RuntimeError as exc:
            assert "accepted_text_sha256" in str(exc)

        # Verify saved candidate files with truncated=True or missing packet acceptance are rejected on load
        bad_cand_file = root / "bad_cand.json"
        write_json(
            bad_cand_file,
            [
                {
                    **bridge_pkt_cand,
                    "truncated": True,
                    "accepted_text_sha256": None,
                }
            ],
        )
        base_ok_file = root / "base_ok.json"
        write_json(base_ok_file, [bridge_msg_cand])
        try:
            load_or_generate_pairs(
                briefs[:1],
                baseline_path=base_ok_file,
                candidate_path=bad_cand_file,
            )
            raise AssertionError("Expected ValueError when loading truncated/unaccepted candidate")
        except ValueError as exc:
            assert "truncated=True" in str(exc)

        write_json(
            bad_cand_file,
            [
                {
                    **bridge_pkt_cand,
                    "truncated": False,
                    "accepted_text_sha256": None,
                }
            ],
        )
        try:
            load_or_generate_pairs(
                briefs[:1],
                baseline_path=base_ok_file,
                candidate_path=bad_cand_file,
            )
            raise AssertionError("Expected ValueError when loading packet candidate without accepted_text_sha256")
        except ValueError as exc:
            assert "accepted_text_sha256" in str(exc)

        # Verify generation checkpoint fingerprint validation (PRRT_kwDOSmzJlM61D0Xy):
        # changing variant, provider, or brief content while keeping brief_id and seed fixed must regenerate
        gen_ckpt_file = root / "gen_ckpt.json"
        fake_cfg = {"provider": "fake", "model": "spindle-fake-drafter-v1"}
        out_compact = generate_candidates_with_checkpoint(
            briefs[:1],
            variants["compact_baseline"],
            fake_cfg,
            gen_ckpt_file,
            condition_label="candidate",
            seeds=[101],
        )
        assert out_compact[0]["prompt_variant"] == "compact_baseline"
        # Poison the checkpoint text while keeping its fingerprint to prove matching inputs reuse the checkpoint
        poisoned_compact = [{**out_compact[0], "text": "CACHED_COMPACT_SENTINEL"}]
        write_json(gen_ckpt_file, poisoned_compact)
        reused_compact = generate_candidates_with_checkpoint(
            briefs[:1],
            variants["compact_baseline"],
            fake_cfg,
            gen_ckpt_file,
            condition_label="candidate",
            seeds=[101],
        )
        assert reused_compact[0]["text"] == "CACHED_COMPACT_SENTINEL"

        # Changing --variant with same --out, brief, and seed must NOT reuse the compact checkpoint
        out_full = generate_candidates_with_checkpoint(
            briefs[:1],
            variants["spindle_full_context"],
            fake_cfg,
            gen_ckpt_file,
            condition_label="candidate",
            seeds=[101],
        )
        assert out_full[0]["prompt_variant"] == "spindle_full_context"
        assert out_full[0]["text"] != "CACHED_COMPACT_SENTINEL"

        # Changing provider (fake -> rust-bridge) with same --out, variant, brief, and seed must NOT reuse fake output
        out_bridge = generate_candidates_with_checkpoint(
            briefs[:1],
            variants["spindle_full_context"],
            bridge_cfg,
            gen_ckpt_file,
            condition_label="candidate",
            seeds=[101],
        )
        assert out_bridge[0]["model_version"].startswith("rust-bridge:")
        assert "Legacy prose len=" in out_bridge[0]["text"]

        # Editing brief content with same --out, variant, provider, and seed must NOT reuse stale brief output
        edited_brief = {**briefs[0], "scene_purpose": "Edited scene purpose for fingerprint invalidation test"}
        out_edited_brief = generate_candidates_with_checkpoint(
            [edited_brief],
            variants["spindle_full_context"],
            fake_cfg,
            gen_ckpt_file,
            condition_label="candidate",
            seeds=[101],
        )
        assert "Edited scene purpose for fingerprint invalidation test" in out_edited_brief[0]["text"]

        # Verify judgment checkpoint fingerprint & judge identity matching (PRRT_kwDOSmzJlM61D0X0):
        judge_ckpt_file = root / "judge_ckpt.json"
        ckpt_pairs = multi_seed_pairs[:2]
        rep_initial = run_evaluation(
            briefs[:2],
            ckpt_pairs,
            offline_judges,
            seed=20261107,
            checkpoint_path=judge_ckpt_file,
        )
        # Stamp distinct per-judge marker notes in checkpoint while preserving valid fingerprints
        ckpt_rows = read_json(judge_ckpt_file)
        for row in ckpt_rows:
            for dj in row["judge_evaluations"]:
                dj["pass_1"]["rationale"] = f"CACHED_FOR_{dj['judge_id']}"
        write_json(judge_ckpt_file, ckpt_rows)

        # Reordering the judge panel with same configs/brief/seed must reuse cached evaluations matched by judge_id
        reordered_judges = list(reversed(offline_judges))
        rep_reordered = run_evaluation(
            briefs[:2],
            ckpt_pairs,
            reordered_judges,
            seed=20261107,
            checkpoint_path=judge_ckpt_file,
        )
        for j_cfg, dj in zip(reordered_judges, rep_reordered["pairs"][0]["judge_evaluations"]):
            assert dj["judge_id"] == j_cfg["id"]
            assert dj["pass_1"]["rationale"] == f"CACHED_FOR_{j_cfg['id']}"
            assert rep_reordered["per_judge"][j_cfg["id"]]["win_rates"]["candidate_wins"] == rep_initial["per_judge"][j_cfg["id"]]["win_rates"]["candidate_wins"]

        # Changing randomization seed must invalidate judgment checkpoint and re-evaluate
        rep_new_seed = run_evaluation(
            briefs[:2],
            ckpt_pairs,
            offline_judges,
            seed=99999999,
            checkpoint_path=judge_ckpt_file,
        )
        assert not rep_new_seed["pairs"][0]["judge_evaluations"][0]["pass_1"]["rationale"].startswith("CACHED_FOR_")

        # Re-stamp marker notes then verify changing judge model/config or brief content invalidates checkpoint
        ckpt_rows = read_json(judge_ckpt_file)
        for row in ckpt_rows:
            for dj in row["judge_evaluations"]:
                dj["pass_1"]["rationale"] = f"CACHED_SEED2_{dj['judge_id']}"
        write_json(judge_ckpt_file, ckpt_rows)

        modified_judges = [
            {**offline_judges[0], "model": "modified-judge-model-v2"},
            *offline_judges[1:],
        ]
        rep_mod_judge = run_evaluation(
            briefs[:2],
            ckpt_pairs,
            modified_judges,
            seed=99999999,
            checkpoint_path=judge_ckpt_file,
        )
        assert not rep_mod_judge["pairs"][0]["judge_evaluations"][0]["pass_1"]["rationale"].startswith("CACHED_SEED2_")

        ckpt_rows = read_json(judge_ckpt_file)
        for row in ckpt_rows:
            for dj in row["judge_evaluations"]:
                dj["pass_1"]["rationale"] = f"CACHED_BRIEF_{dj['judge_id']}"
        write_json(judge_ckpt_file, ckpt_rows)

        modified_briefs = [{**briefs[0], "contract": "Modified contract text"}, briefs[1]]
        rep_mod_brief = run_evaluation(
            modified_briefs,
            ckpt_pairs,
            modified_judges,
            seed=99999999,
            checkpoint_path=judge_ckpt_file,
        )
        assert not rep_mod_brief["pairs"][0]["judge_evaluations"][0]["pass_1"]["rationale"].startswith("CACHED_BRIEF_")

    # 5. Verify secret redaction & key check safety
    os.environ["SPINDLE_TEST_FAKE_SECRET_KEY"] = "super-secret-value-12345"
    try:
        scrubbed = redact_secrets(
            "Error calling endpoint with bearer super-secret-value-12345",
            ["SPINDLE_TEST_FAKE_SECRET_KEY"],
        )
        assert "super-secret-value-12345" not in scrubbed
        assert "[REDACTED]" in scrubbed
    finally:
        os.environ.pop("SPINDLE_TEST_FAKE_SECRET_KEY", None)

    key_report = check_keys()
    assert "OLLAMA_API_KEY" in key_report["keys"]
    serialized_keys = json.dumps(key_report)
    for k in key_report["keys"]:
        val = os.environ.get(k)
        if val:
            assert val not in serialized_keys, "check_keys must never include secret values"

    print(
        "30-brief suite, swapped-order double judging, position-bias guard, cluster bootstrap CIs, "
        "inter-judge kappa, Rust packet/message generation bridge, 100-pair calibration workflow, and key safety passed."
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("self-test", help="Run offline unit and integration self-tests")

    ck = sub.add_parser("check-keys", help="Check which provider API keys are set in the environment")
    ck.add_argument("--judges", default=str(JUDGES_PATH))

    gen = sub.add_parser("generate", help="Generate candidate outputs from saved files, a model prompt variant, or the Rust bridge")
    gen.add_argument("--briefs", default=str(BRIEFS_PATH))
    gen.add_argument("--variants-file", default=str(VARIANTS_PATH))
    gen.add_argument("--variant", required=True, help="Variant id in prompt_variants.json (e.g. compact_baseline, spindle_full_context, drafting_packet_v1)")
    gen.add_argument("--condition", default="candidate", choices=["baseline", "candidate"])
    gen.add_argument("--provider", default=None, help="Override provider (rust-bridge, ollama, anthropic, openai-compatible, fake)")
    gen.add_argument("--endpoint", default=None)
    gen.add_argument("--model", default=None)
    gen.add_argument("--api-key-env", default=None)
    gen.add_argument("--rust-config", default=None, help="Agents TOML config path for rust-bridge generation")
    gen.add_argument("--rust-db", default=None, help="SQLite DB path for rust-bridge usage/probe/project state")
    gen.add_argument("--artifact-dir", default=None, help="Directory to store DraftingPacketV1 and PlainTextDraftArtifactV1 files")
    gen.add_argument("--tokenizer-binary", default=None, help="Optional explicit path to spindle-tokenizer binary")
    gen.add_argument("--route", default="draft", help="ModelRouter route name for rust-bridge (default: draft)")
    gen.add_argument("--rating", default="general", help="Content rating for rust-bridge route clearance (default: general)")
    gen.add_argument("--generation-seed", type=int, default=None, help="Single generation seed for model sampling")
    gen.add_argument("--generation-seeds", default=None, help="Comma-separated list of generation seeds (e.g. 101,202,303)")
    gen.add_argument("--limit", type=int, default=None, help="Optional cap on number of briefs to generate")
    gen.add_argument("--out", required=True, help="Output JSON file path")

    jdg = sub.add_parser("judge", help="Run pairwise randomized + swapped-order double judging")
    jdg.add_argument("--briefs", default=str(BRIEFS_PATH))
    jdg.add_argument("--pairs", default=None, help="Combined pairs JSON file (default: evals/prose_judge/fixture_pairs.json)")
    jdg.add_argument("--baseline", default=None, help="Saved baseline candidate list JSON")
    jdg.add_argument("--candidate", default=None, help="Saved candidate list JSON")
    jdg.add_argument("--variant-a", default=None, help="Generate baseline end-to-end from variant A")
    jdg.add_argument("--variant-b", default=None, help="Generate candidate end-to-end from variant B")
    jdg.add_argument("--provider", default=None, help="Override generator provider when using --variant-a/--variant-b")
    jdg.add_argument("--rust-config", default=None, help="Agents TOML config path for rust-bridge generation")
    jdg.add_argument("--rust-db", default=None, help="SQLite DB path for rust-bridge generation")
    jdg.add_argument("--artifact-dir", default=None, help="Artifact output directory for rust-bridge generation")
    jdg.add_argument("--tokenizer-binary", default=None, help="Optional explicit path to spindle-tokenizer binary")
    jdg.add_argument("--route", default="draft", help="ModelRouter route name for rust-bridge (default: draft)")
    jdg.add_argument("--rating", default="general", help="Content rating for rust-bridge route clearance (default: general)")
    jdg.add_argument("--generation-seed", type=int, default=None, help="Single generation seed for variant generation/filtering")
    jdg.add_argument("--generation-seeds", default=None, help="Comma-separated list of generation seeds (e.g. 101,202,303)")
    jdg.add_argument("--judges", default=str(JUDGES_PATH), help="Judges config JSON path")
    jdg.add_argument("--live", action="store_true", help="Use live_judges panel instead of offline_judges")
    jdg.add_argument("--limit", type=int, default=None, help="Evaluate first N briefs (useful for smoke tests)")
    jdg.add_argument("--seed", type=int, default=20261107)
    jdg.add_argument("--calibration-key", default=None, help="Optional private-key.json from human calibration")
    jdg.add_argument("--calibration-ratings", default=None, help="Optional ratings.json from human calibration")
    jdg.add_argument("--out", default=str(RESULTS_DIR / "prose_judge_offline_baseline.json"))

    bl = sub.add_parser("blind", help="Create a blinded human-rating packet (~100 pairs) reusing serial_fiction.py")
    bl.add_argument("--pairs", default=str(FIXTURE_PAIRS_PATH))
    bl.add_argument("--briefs", default=str(BRIEFS_PATH))
    bl.add_argument("--count", type=int, default=100, help="Number of pairs to include in the blinded packet (default: 100)")
    bl.add_argument("--seed", type=int, default=20261107)
    bl.add_argument("--out", required=True, help="Empty directory for reviewer-packet.json, ratings.json, private-key.json")

    cal = sub.add_parser("calibrate", help="Compute judge-human agreement from a blinded human rating packet")
    cal.add_argument("--key", required=True, help="Path to private-key.json")
    cal.add_argument("--ratings", required=True, help="Path to human-completed ratings.json")
    cal.add_argument("--judge-results", required=True, help="Path to judge results JSON")
    cal.add_argument("--update-results", action="store_true", help="Write calibration section back into --judge-results")
    cal.add_argument("--out", default=None, help="Optional output path for standalone calibration JSON")

    args = parser.parse_args()

    if args.command == "self-test":
        self_test()
        return

    if args.command == "check-keys":
        report = check_keys(args.judges)
        print(json.dumps(report, indent=2))
        return

    if args.command == "generate":
        briefs = load_briefs(args.briefs)
        if args.limit:
            briefs = briefs[: args.limit]
        vfile = read_json(args.variants_file)
        variants = vfile.get("variants", vfile)
        if args.variant not in variants:
            raise SystemExit(f"Unknown variant {args.variant!r}; available: {sorted(variants)}")
        gen_cfg = dict(vfile.get("default_generator") or {"provider": "fake", "model": "spindle-fake-drafter-v1"})
        if args.provider:
            gen_cfg["provider"] = args.provider
        if args.endpoint:
            gen_cfg["endpoint"] = args.endpoint
        if args.model:
            gen_cfg["model"] = args.model
        if args.api_key_env is not None:
            gen_cfg["api_key_env"] = args.api_key_env
        if args.rust_config:
            gen_cfg["rust_config"] = args.rust_config
            if not args.provider:
                gen_cfg["provider"] = "rust-bridge"
        if args.rust_db:
            gen_cfg["rust_db"] = args.rust_db
        if args.artifact_dir:
            gen_cfg["artifact_dir"] = args.artifact_dir
        if args.tokenizer_binary:
            gen_cfg["tokenizer_binary"] = args.tokenizer_binary
        if args.route:
            gen_cfg["route"] = args.route
        if args.rating:
            gen_cfg["rating"] = args.rating

        seeds = parse_generation_seeds(args.generation_seed, args.generation_seeds)
        outputs = generate_candidates_with_checkpoint(
            briefs,
            variants[args.variant],
            gen_cfg,
            args.out,
            condition_label=args.condition,
            seeds=seeds,
        )
        print(f"Wrote {len(outputs)} generated candidates ({args.variant}, seeds={seeds}) to {args.out}")
        return

    if args.command == "judge":
        briefs = load_briefs(args.briefs)
        jcfg = read_json(args.judges)
        if args.live:
            key_status = check_keys(args.judges)
            if not key_status["all_primary_keys_set"]:
                print(
                    "Missing required API keys for live judging: "
                    + ", ".join(key_status["missing_primary_keys"])
                    + f"\nRun via: {key_status['recommended_invocation']}",
                    file=sys.stderr,
                )
                raise SystemExit(2)
            judges = jcfg["live_judges"]
        else:
            judges = jcfg["offline_judges"]

        gen_override: dict = {} if args.live else {"provider": "fake", "model": "spindle-fake-drafter-v1"}
        if args.provider:
            gen_override["provider"] = args.provider
        if args.rust_config:
            gen_override["rust_config"] = args.rust_config
            if not args.provider:
                gen_override["provider"] = "rust-bridge"
        if args.rust_db:
            gen_override["rust_db"] = args.rust_db
        if args.artifact_dir:
            gen_override["artifact_dir"] = args.artifact_dir
        if args.tokenizer_binary:
            gen_override["tokenizer_binary"] = args.tokenizer_binary
        if args.route:
            gen_override["route"] = args.route
        if args.rating:
            gen_override["rating"] = args.rating

        seeds = (
            parse_generation_seeds(args.generation_seed, args.generation_seeds)
            if (args.generation_seed is not None or args.generation_seeds is not None)
            else None
        )
        pairs = load_or_generate_pairs(
            briefs,
            pairs_path=args.pairs,
            baseline_path=args.baseline,
            candidate_path=args.candidate,
            variant_a=args.variant_a,
            variant_b=args.variant_b,
            generator_override=gen_override or None,
            limit=args.limit,
            generation_seeds=seeds,
        )
        result = run_evaluation(
            briefs,
            pairs,
            judges,
            seed=args.seed,
            calibration_key=args.calibration_key,
            calibration_ratings=args.calibration_ratings,
            checkpoint_path=f"{args.out}.checkpoint.json" if args.live else None,
        )
        write_json(args.out, result)
        ps = result["panel_summary"]
        print(
            f"Evaluated {result['n_briefs_evaluated']} pairs ({result.get('n_unique_briefs', ps['n_clusters'])} briefs) "
            f"across {len(judges)} judges -> "
            f"candidate_wins={ps['candidate_wins']}, baseline_wins={ps['baseline_wins']}, ties={ps['ties']}, "
            f"win_rate_tie_half={ps['candidate_win_rate_tie_half']['estimate']} "
            f"[{ps['candidate_win_rate_tie_half']['ci95_low']}, {ps['candidate_win_rate_tie_half']['ci95_high']}], "
            f"position_bias_rate={result['position_bias']['position_bias_rate']}. "
            f"Saved to {args.out}"
        )
        return

    if args.command == "blind":
        briefs = load_briefs(args.briefs)
        brief_map = {b["id"]: b for b in briefs}
        base_pairs = load_or_generate_pairs(briefs, pairs_path=args.pairs)
        cal_pairs = expand_pairs_for_calibration(base_pairs, brief_map, target_count=args.count, seed=args.seed)
        serial_fiction.blind_pairs(cal_pairs, args.out, seed=args.seed, briefs_path=args.briefs)
        print(f"Created blinded human calibration packet ({len(cal_pairs)} pairs) in {args.out}")
        return

    if args.command == "calibrate":
        judge_results = read_json(args.judge_results)
        cal = compute_calibration_agreement(args.key, args.ratings, judge_results)
        if args.update_results:
            judge_results["calibration"] = cal
            write_json(args.judge_results, judge_results)
        if args.out:
            write_json(args.out, cal)
        print(json.dumps(cal, indent=2))
        return


if __name__ == "__main__":
    main()

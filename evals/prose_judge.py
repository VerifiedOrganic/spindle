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
        s1 = entry.get("passage_1_score")
        s2 = entry.get("passage_2_score")
        if isinstance(s1, bool) or not isinstance(s1, int) or not (1 <= s1 <= 5):
            raise ValueError(f"{dim}.passage_1_score must be an integer 1..5, got {s1!r}")
        if isinstance(s2, bool) or not isinstance(s2, int) or not (1 <= s2 <= 5):
            raise ValueError(f"{dim}.passage_2_score must be an integer 1..5, got {s2!r}")
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
        "markdown commentary outside the JSON."
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
        safe_err = redact_secrets(f"HTTP {exc.code} from {url}: {err_body}", [key_env] if key_env else [])
        raise RuntimeError(safe_err) from None
    except Exception as exc:
        safe_err = redact_secrets(f"Request failed for {url}: {exc}", [key_env] if key_env else [])
        raise RuntimeError(safe_err) from None


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
            payload = {
                "model": model,
                "stream": False,
                "think": False,
                "format": "json" if agent_cfg.get("json_mode", True) else None,
                "messages": [
                    {"role": "system", "content": system_prompt},
                    {"role": "user", "content": user_prompt},
                ],
                "options": {
                    "temperature": temperature,
                    "num_predict": max(max_tokens, 4096),
                },
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
# Generation Adapter (Saved files OR model + prompt-variant)
# =============================================================================

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


def generate_candidate_for_brief(
    brief: dict,
    variant_cfg: dict,
    generator_cfg: dict,
    condition_label: str = "candidate",
) -> dict:
    """Produce a single candidate output for a brief via fake or live model adapter."""
    sys_prompt, user_prompt = render_variant_prompt(brief, variant_cfg)
    provider = str(generator_cfg.get("provider", "fake")).lower()
    variant_id = variant_cfg.get("id", "custom_variant")

    if provider == "fake":
        # Offline deterministic generator: full_drafter_context honors all brief constraints;
        # compact mode produces a shorter, slop-prone draft.
        loc_name = brief.get("drafter_context", {}).get("location", {}).get("name", "the room")
        pov_char = brief.get("pov", {}).get("character", "The protagonist")
        persp = brief.get("pov", {}).get("perspective", "third_limited")
        subj = "I" if persp == "first_person" else pov_char
        if variant_cfg.get("context_mode") == "compact":
            text = (
                f"{subj} stood in {loc_name}. It wasn't fear. It was determination. "
                f"A mix of relief and dread washed over the moment.\n\n"
                f"'We must finish this,' {subj} hissed, not knowing what tomorrow would bring."
            )
        else:
            crit_summary = " ".join(brief.get("criteria", []))
            tail = brief.get("drafter_context", {}).get("previous_scene_tail", "")
            text = (
                f"{tail}\n\n"
                f"At {loc_name}, {subj} acted on the immediate constraint: {brief.get('scene_purpose', '')}\n\n"
                f"'Hold the line steady,' {subj} said, keeping to the terms of the brief ({crit_summary}).\n\n"
                f"The turn completed cleanly as {brief.get('tension_turn', '')}"
            )
        return {
            "case_id": brief["id"],
            "brief_id": brief["id"],
            "condition": condition_label,
            "prompt_variant": variant_id,
            "model": generator_cfg.get("model", "spindle-fake-drafter-v1"),
            "model_version": "offline-v1",
            "text": text.strip(),
            "input_tokens": len(user_prompt.split()),
            "output_tokens": len(text.split()),
            "elapsed_seconds": 0.001,
        }

    gen_call_cfg = {**generator_cfg, "json_mode": False}
    resp = call_model_provider(gen_call_cfg, sys_prompt, user_prompt)
    return {
        "case_id": brief["id"],
        "brief_id": brief["id"],
        "condition": condition_label,
        "prompt_variant": variant_id,
        "model": resp.get("model", generator_cfg.get("model", "unknown")),
        "model_version": f"{provider}:{generator_cfg.get('model', 'unknown')}",
        "text": resp["text"],
        "input_tokens": resp.get("input_tokens"),
        "output_tokens": resp.get("output_tokens"),
        "elapsed_seconds": resp.get("elapsed_seconds"),
    }


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
) -> list[dict]:
    """Load candidate pairs from saved files OR generate them from two prompt variants."""
    selected_briefs = briefs[:limit] if limit else briefs
    brief_map = {b["id"]: b for b in selected_briefs}

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
            base_cand = generate_candidate_for_brief(b, variants[variant_a], gen_cfg, condition_label="baseline")
            cand_cand = generate_candidate_for_brief(b, variants[variant_b], gen_cfg, condition_label="candidate")
            pairs.append({
                "pair_id": f"P{idx:03d}",
                "case_id": b["id"],
                "brief_id": b["id"],
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
        base_by_id = {(r.get("case_id") or r.get("brief_id")): r for r in base_list}
        cand_by_id = {(r.get("case_id") or r.get("brief_id")): r for r in cand_list}
        pairs = []
        for idx, b in enumerate(selected_briefs, start=1):
            bid = b["id"]
            if bid not in base_by_id or bid not in cand_by_id:
                raise ValueError(f"Missing brief {bid} in baseline or candidate file")
            pairs.append({
                "pair_id": f"P{idx:03d}",
                "case_id": bid,
                "brief_id": bid,
                "scene_type": b["scene_type"],
                "genre": b["genre"],
                "baseline": base_by_id[bid],
                "candidate": cand_by_id[bid],
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
    if c["candidate"] > c["baseline"] and c["candidate"] >= c["tie"]:
        return "candidate"
    if c["baseline"] > c["candidate"] and c["baseline"] >= c["tie"]:
        return "baseline"
    return "tie"


def summarize_verdicts(verdicts: list[str], seed: int = 20261107) -> dict:
    """Compute win rates and 95% bootstrap confidence intervals from a list of verdicts."""
    n = len(verdicts)
    c_wins = sum(1 for v in verdicts if v == "candidate")
    b_wins = sum(1 for v in verdicts if v == "baseline")
    ties = sum(1 for v in verdicts if v == "tie")

    tie_half_scores = [1.0 if v == "candidate" else (0.5 if v == "tie" else 0.0) for v in verdicts]
    c_outright_scores = [1.0 if v == "candidate" else 0.0 for v in verdicts]
    b_outright_scores = [1.0 if v == "baseline" else 0.0 for v in verdicts]

    return {
        "n_pairs": n,
        "candidate_wins": c_wins,
        "baseline_wins": b_wins,
        "ties": ties,
        "candidate_win_rate_tie_half": bootstrap_ci(tie_half_scores, seed=seed),
        "candidate_outright_win_rate": bootstrap_ci(c_outright_scores, seed=seed + 1),
        "baseline_outright_win_rate": bootstrap_ci(b_outright_scores, seed=seed + 2),
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
    """Build a calibration set of ~100 pairs drawn across the 30 scene briefs."""
    if len(base_pairs) >= target_count:
        return [
            {**p, "pair_id": f"P{idx:03d}"}
            for idx, p in enumerate(base_pairs[:target_count], start=1)
        ]

    rng = random.Random(seed)
    expanded: list[dict] = []
    variant_Round = 0
    while len(expanded) < target_count:
        variant_Round += 1
        cycle_pairs = list(base_pairs)
        if variant_Round > 1:
            rng.shuffle(cycle_pairs)
        for p in cycle_pairs:
            if len(expanded) >= target_count:
                break
            idx = len(expanded) + 1
            cid = p["case_id"]
            brief = briefs_by_id.get(cid, {})
            base_copy = dict(p["baseline"])
            cand_copy = dict(p["candidate"])
            if variant_Round > 1:
                # Distinct pair sampling tag so each of the ~100 calibration pairs has unique provenance
                base_copy["model_version"] = f"{base_copy.get('model_version', 'v1')}-cal-r{variant_Round}"
                cand_copy["model_version"] = f"{cand_copy.get('model_version', 'v1')}-cal-r{variant_Round}"
            expanded.append({
                "pair_id": f"P{idx:03d}",
                "case_id": cid,
                "brief_id": cid,
                "task": p.get("task") or brief.get("task", ""),
                "contract": p.get("contract") or brief.get("contract", ""),
                "scene_purpose": p.get("scene_purpose") or brief.get("scene_purpose", ""),
                "continuity_checklist": p.get("continuity_checklist") or brief.get("criteria", []),
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
    human_by_pair: dict[str, str] = {}
    human_by_case: dict[str, list[str]] = {}
    for row in human_scored.get("deblinded_pairs", []):
        verdict = row.get("human_verdict")
        if verdict in VERDICT_LABELS:
            human_by_pair[row["pair_id"]] = verdict
            human_by_case.setdefault(row["case_id"], []).append(verdict)

    pair_records = judge_results.get("pairs", [])
    judge_ids = [j["id"] for j in judge_results.get("judges", [])]

    aligned_human: list[str] = []
    aligned_consensus: list[str] = []
    aligned_by_judge: dict[str, list[str]] = {jid: [] for jid in judge_ids}

    # Align either by exact pair_id or by brief_id/case_id across the calibration set
    judge_by_pair = {pr["pair_id"]: pr for pr in pair_records}
    judge_by_case = {pr["brief_id"]: pr for pr in pair_records}

    for row in human_scored.get("deblinded_pairs", []):
        h_verdict = row.get("human_verdict")
        if h_verdict not in VERDICT_LABELS:
            continue
        pr = judge_by_pair.get(row["pair_id"]) or judge_by_case.get(row["case_id"])
        if not pr:
            continue
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

    for pair in pairs:
        bid = pair.get("brief_id") or pair["case_id"]
        brief = brief_map[bid]
        evals_for_pair = []
        for judge_cfg in judges:
            dj = judge_pair_double(judge_cfg, brief, pair, seed=seed)
            evals_for_pair.append(dj)
            all_double_judgments.append(dj)
            by_judge_judgments[judge_cfg["id"]].append(dj)
            by_judge_verdicts[judge_cfg["id"]].append(dj["verdict"])

        consensus = panel_consensus([e["verdict"] for e in evals_for_pair])
        consensus_verdicts.append(consensus)
        pair_reports.append({
            "pair_id": pair["pair_id"],
            "brief_id": bid,
            "name": brief["name"],
            "project_id": brief["project_id"],
            "genre": brief["genre"],
            "scene_type": brief["scene_type"],
            "pov": brief["pov"],
            "baseline_meta": {
                "model": pair["baseline"].get("model"),
                "model_version": pair["baseline"].get("model_version"),
                "prompt_variant": pair["baseline"].get("prompt_variant"),
            },
            "candidate_meta": {
                "model": pair["candidate"].get("model"),
                "model_version": pair["candidate"].get("model_version"),
                "prompt_variant": pair["candidate"].get("prompt_variant"),
            },
            "panel_consensus": consensus,
            "judge_evaluations": evals_for_pair,
        })

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

    # Breakdown by scene_type and genre
    by_scene_type = {}
    for stype in SCENE_TYPES:
        st_verdicts = [pr["panel_consensus"] for pr in pair_reports if pr["scene_type"] == stype]
        if st_verdicts:
            by_scene_type[stype] = summarize_verdicts(st_verdicts, seed=seed)

    by_genre = {}
    for genre in sorted({pr["genre"] for pr in pair_reports}):
        g_verdicts = [pr["panel_consensus"] for pr in pair_reports if pr["genre"] == genre]
        by_genre[genre] = summarize_verdicts(g_verdicts, seed=seed)

    per_judge_summary = {}
    for j in judges:
        jid = j["id"]
        per_judge_summary[jid] = {
            "family": j.get("family", "unknown"),
            "provider": j.get("provider", "fake"),
            "model": j.get("model", "unknown"),
            "win_rates": summarize_verdicts(by_judge_verdicts[jid], seed=seed),
            "position_bias": summarize_position_bias(by_judge_judgments[jid]),
        }

    report = {
        "schema_version": 1,
        "suite_id": "spindle.prose_judge.v1",
        "seed": seed,
        "n_briefs_evaluated": len(pair_reports),
        "judges": [
            {
                "id": j["id"],
                "family": j.get("family", "unknown"),
                "provider": j.get("provider", "fake"),
                "model": j.get("model", "unknown"),
            }
            for j in judges
        ],
        "panel_summary": summarize_verdicts(consensus_verdicts, seed=seed),
        "position_bias": summarize_position_bias(all_double_judgments),
        "inter_judge_agreement": compute_inter_judge_agreement([j["id"] for j in judges], by_judge_verdicts),
        "rubric_summary": rubric_summary,
        "by_scene_type": by_scene_type,
        "by_genre": by_genre,
        "per_judge": per_judge_summary,
        "calibration": None,
        "pairs": pair_reports,
        "interpretation": (
            "Pairwise LLM-judge evaluation with randomized initial order and swapped-order double judging. "
            "Order-inconsistent pairs are recorded as position-biased ties rather than awarded as wins. "
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
        "30-brief suite, swapped-order double judging, position-bias guard, bootstrap CIs, "
        "inter-judge kappa, generation adapter, 100-pair calibration workflow, and key safety passed."
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("self-test", help="Run offline unit and integration self-tests")

    ck = sub.add_parser("check-keys", help="Check which provider API keys are set in the environment")
    ck.add_argument("--judges", default=str(JUDGES_PATH))

    gen = sub.add_parser("generate", help="Generate candidate outputs from saved files or a model prompt variant")
    gen.add_argument("--briefs", default=str(BRIEFS_PATH))
    gen.add_argument("--variants-file", default=str(VARIANTS_PATH))
    gen.add_argument("--variant", required=True, help="Variant id in prompt_variants.json (e.g. compact_baseline or spindle_full_context)")
    gen.add_argument("--condition", default="candidate", choices=["baseline", "candidate"])
    gen.add_argument("--provider", default=None, help="Override provider (ollama, anthropic, openai-compatible, fake)")
    gen.add_argument("--endpoint", default=None)
    gen.add_argument("--model", default=None)
    gen.add_argument("--api-key-env", default=None)
    gen.add_argument("--limit", type=int, default=None, help="Optional cap on number of briefs to generate")
    gen.add_argument("--out", required=True, help="Output JSON file path")

    jdg = sub.add_parser("judge", help="Run pairwise randomized + swapped-order double judging")
    jdg.add_argument("--briefs", default=str(BRIEFS_PATH))
    jdg.add_argument("--pairs", default=None, help="Combined pairs JSON file (default: evals/prose_judge/fixture_pairs.json)")
    jdg.add_argument("--baseline", default=None, help="Saved baseline candidate list JSON")
    jdg.add_argument("--candidate", default=None, help="Saved candidate list JSON")
    jdg.add_argument("--variant-a", default=None, help="Generate baseline end-to-end from variant A")
    jdg.add_argument("--variant-b", default=None, help="Generate candidate end-to-end from variant B")
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
        outputs = [
            generate_candidate_for_brief(b, variants[args.variant], gen_cfg, condition_label=args.condition)
            for b in briefs
        ]
        write_json(args.out, outputs)
        print(f"Wrote {len(outputs)} generated candidates ({args.variant}) to {args.out}")
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

        gen_override = None if args.live else {"provider": "fake", "model": "spindle-fake-drafter-v1"}
        pairs = load_or_generate_pairs(
            briefs,
            pairs_path=args.pairs,
            baseline_path=args.baseline,
            candidate_path=args.candidate,
            variant_a=args.variant_a,
            variant_b=args.variant_b,
            generator_override=gen_override,
            limit=args.limit,
        )
        result = run_evaluation(
            briefs,
            pairs,
            judges,
            seed=args.seed,
            calibration_key=args.calibration_key,
            calibration_ratings=args.calibration_ratings,
        )
        write_json(args.out, result)
        ps = result["panel_summary"]
        print(
            f"Evaluated {result['n_briefs_evaluated']} briefs across {len(judges)} judges -> "
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

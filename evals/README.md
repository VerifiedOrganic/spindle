# Eval kits

## Fiction anti-slop (scanner regressions)

Phase 4 locks the twelve fiction shelves against scanner drift and false
positives. Regression cases live in `evals/anti_slop/cases.json`, and the
precision/recall evaluation corpus lives in `evals/anti_slop/corpus.json`
(160 public-domain human prose passages from Project Gutenberg documented in
`evals/anti_slop/SOURCES.md` plus 48 synthetic LLM-style positive passages).
Optional `preference_dims` are labeled editorial observations
(`present` / `absent` / `n/a`) — not a quality superiority claim.

```sh
python3 evals/anti_slop.py self-test
python3 evals/anti_slop.py drift
python3 evals/anti_slop.py regress
python3 evals/anti_slop.py report
python3 evals/anti_slop.py score
```

`self-test` validates case and corpus schemas, twelve-shelf coverage, and
`solitary_fade` stability. `drift` fails if catalog markdown, the v0 pack,
the scanner constant, and the eval manifest disagree. `regress` runs
`cargo test -p spindle-core --test antislop_eval`. `report` prints the
per-shelf precision/recall report on the corpus and enforces that every `hard`
shelf achieves precision ≥ 0.9. `score` tallies labeled dims without ranking
writing quality.

CI runs `self-test`, `drift`, and `report` on every pull request.

---

# Serial-fiction comparison kit

Twelve original synthetic cases compare three conditions: compact context,
Spindle’s actual context packet, and the full draft/review/revision workflow.
The long case expands to 200 chapter summaries across two books. This tests
synopsis retention; it is not a substitute for evaluating a real 200-chapter
manuscript. The cases cover delayed and reopened promises, historical knowledge,
world-rule costs, voice, quiet endings, retcons, hiatus recovery and revision.

No writing model is invoked by this script. No human ratings are supplied.
`collect` creates new `EVAL …` projects and captures context through real MCP
calls. Use a disposable workspace so fixtures do not clutter your actual books.

## Run

Start a second Spindle server with an empty model configuration and a fresh data
directory. In one terminal (replace the binary path with your local build):

```sh
mkdir -p /tmp/spindle-eval-workspace
printf '' > /tmp/spindle-eval-workspace/config.toml
SPINDLE_DATA_DIR=/tmp/spindle-eval-workspace \
SPINDLE_CONFIG=/tmp/spindle-eval-workspace/config.toml \
SPINDLE_HTTP_ADDR=127.0.0.1:8940 target/debug/spindle-mcp
```

In another terminal, from this repository:

```sh
python3 evals/serial_fiction.py self-test
python3 evals/serial_fiction.py collect --server http://127.0.0.1:8940/mcp --out /tmp/serial-captures
python3 evals/serial_fiction.py prepare --captures /tmp/serial-captures/captures.json --out /tmp/serial-requests
```

`collect` uses setup and read tools only. The full-workflow condition needs your
intended draft/review model configuration when you generate its candidate.
Preserve captures: they include exact requests, responses and a case hash.
Interrupted collection preserves completed captures; use a new output directory
for a fresh collection. It does not remove existing fixture projects.

Generate all 36 requests in `requests.json` through the agent/model you are
comparing. Copy `candidate-template.json` to your candidate file and fill each
prose, model/version and usage field. Keep the same model version, output length,
temperature/seed (when supported) and creative instructions across conditions.
Record these settings with the results. The full workflow gets one revision;
record review and revision costs as well as the final generation. If any call’s
token count is unknown, mark the candidate’s total null rather than silently
summing an incomplete total. Host-agent calls outside Spindle require the host’s
own usage records; `get_model_usage` cannot see them.

The compact condition receives only the last three earlier summaries and the
contract. The context condition receives the actual captured packet. This is a
context-value comparison; input sizes intentionally differ. The full workflow
uses the fixture’s project and existing scene-writer tools. Use fresh fixture
projects for another run so previous candidates cannot contaminate it. Do not
release evaluation chapters.

```sh
python3 evals/serial_fiction.py blind --candidates /tmp/candidates.json --out /tmp/blind-review --seed 20260905
```

Give the reviewer **only** `reviewer-packet.json` and `ratings.json`. Keep
`private-key.json` private until the ratings are final. Its condition labels,
model identities and token figures are absent from the reviewer packet. A fixed
seed reproducibly balances the A/B/C positions across the 12 cases. Prose can
still reveal stylistic clues; blinding is not a guarantee of anonymity.

For each case, `preferred` is one label, multiple tied labels, or `[]` for no
judgment. Rate each passage’s voice and engagement from 1–5. Count concrete
continuity errors against the supplied checklist and record actual minutes
spent revising that passage if you perform that edit. Leave unmeasured fields
null. A zero is a measured zero. Use human readers for human-preference claims;
a model critic must be identified as a model critic in the accompanying record.

```sh
python3 evals/serial_fiction.py score --key /tmp/blind-review/private-key.json --ratings /tmp/blind-review/ratings.json
```

The report includes outright wins, tied top choices, fractional preference
credit, rated counts, separate craft/effort means and known/unknown usage. It
reports missing judgments and does not rank an unjudged experiment as a win.
Twelve cases are a screening set. Before changing your production writing
workflow, repeat on several real arcs and have the author judge the results.

---

# Continuity recall benchmark

`evals/continuity_recall.py` measures continuity detection recall and control
false-positive counts across two synthetic 20-chapter manuscripts
(`ashfall_expedition` and `veridian_station`, 40 chapters total with distinct
narrative events per chapter). Each manuscript is seeded twice against a
disposable `spindle-mcp` server and temporary `SPINDLE_DATA_DIR`:

1. **Unplanted control copy** — measures false positives emitted by
   `check_consistency` when no contradictions are planted.
2. **Planted copy** — plants 2 contradictions per known category per manuscript
   (14 per manuscript, 28 total across the 7 categories:
   `character_attribute`, `dead_character_acting`,
   `timeline_chronology_rewind`, `knowledge_leak_before_learned`,
   `world_rule_cost_violation`, `quantity_affordability`, and `secret_leak`),
   pairing a lexical/direct variant with a natural-prose/paraphrased variant in
   each category.

## Run

```sh
cargo build -p spindle-mcp
python3 evals/continuity_recall.py self-test
python3 evals/continuity_recall.py --output evals/results/continuity_recall_baseline.json
```

To include the optional model-backed deep tier (`deep_check: true`), pass
`--deep` with a configured `review` route:

```sh
python3 evals/continuity_recall.py --deep --agent-config /path/to/spindle.toml --output /tmp/continuity_recall_deep.json
```

The checked-in deterministic baseline lives at
`evals/results/continuity_recall_baseline.json`. Implicit debug/release binary
selection rejects binaries older than any crate file or root Cargo manifest/lockfile.
Rebuild before measuring; an explicit `--binary /path/to/spindle-mcp` deliberately
bypasses this timestamp guard for a chosen benchmark artifact. CI runs `self-test`
and the deterministic benchmark on every pull request.

---

# Pairwise LLM-judge prose quality kit

`evals/prose_judge.py` (+ fixtures under `evals/prose_judge/`) evaluates pairwise
prose quality across a fixed suite of 30 scene briefs (`PJ01`–`PJ30` in
`evals/prose_judge/briefs.json`). The 30 briefs span six synthetic projects and
six genres (`fantasy`, `comic fantasy`, `science fiction`, `noir mystery`,
`political intrigue`, `progression fantasy`, `gothic horror`), three POV modes
(`first_person`, `third_limited`, `close_third`), and five scene types (six
briefs each: `action`, `dialogue-heavy`, `quiet-interior`, `reveal`,
`chapter-closer`). Every brief carries the full Spindle drafter context packet
(location, in-world time anchor, hard constraints, character snapshots with
knowledge gates, prior episode summaries, previous-scene closing lines, open
promises, and rendered Markdown context).

## Offline self-test & CI-free evaluation

CI runs `python3 evals/prose_judge.py self-test` offline on every pull request:

```sh
python3 evals/prose_judge.py self-test
python3 evals/prose_judge.py check-keys
python3 evals/prose_judge.py judge --out evals/results/prose_judge_offline_baseline.json
```

`evals/prose_judge/fixture_pairs.json` supplies 30 saved `(baseline, candidate)`
pairs (22 where `candidate` is stronger, 5 where `baseline` is stronger, and 3
closely matched ties) so bidirectional discrimination, position-bias guards,
bootstrap confidence intervals, and inter-judge kappa can be verified without
network calls.

## Swapped-order double judging & multi-family judge panels

For every `(brief, judge)` pair, `prose_judge.py` runs **two independent passes**:

1. **Pass 1 (randomized order):** a seeded RNG assigns `baseline` and `candidate`
   to neutral labels `Passage 1` and `Passage 2`.
2. **Pass 2 (swapped order):** the passages are evaluated again in reversed
   order (`Passage 1` and `Passage 2` swapped).

Each pass requires a structured 7-dimension rubric (`scene_purpose_fulfilled`,
`tension_turn`, `pov_discipline`, `voice_distinctiveness`, `dialogue`,
`prose_freshness`, `continuity_with_brief` scored 1–5 with per-dimension winner
and note) plus `overall_preference` and `rationale`. If swapping presentation
order changes the judge's de-blinded winner, the pair is flagged as
order-inconsistent (`position_bias_kind`: `first_position`, `second_position`,
or `tie_flip`) and resolved to a tie rather than awarded as an outright win.

Judge panels are configured in `evals/prose_judge/judges.json` across multiple
model families and providers (`ollama`, `anthropic`, `openai-compatible`, and
`fake`). Multi-judge runs enforce at least two distinct model families and
report:

- **Win rates** (`candidate_win_rate_tie_half`, `candidate_outright_win_rate`,
  `baseline_outright_win_rate`, `tie_rate`) with **95% non-parametric bootstrap
  confidence intervals** (`n_resamples = 2000`).
- **Position-bias rate** and order-consistency rate (overall and per-judge).
- **Inter-judge agreement** (pairwise raw agreement, non-tie directional
  agreement, **Cohen's kappa**, and multi-rater **Fleiss' kappa**).

## Live evaluation & generation adapter (`ai run`)

API keys are read from environment variables only and never printed or logged.
Check key status with `ai keys ls` or `python3 evals/prose_judge.py check-keys`,
and run live calls via `ai run`:

```sh
# Generate candidate outputs for two prompt variants end-to-end:
ai run OLLAMA_API_KEY -- python3 evals/prose_judge.py generate \
  --variant compact_baseline --condition baseline --limit 5 --out /tmp/base.json
ai run OLLAMA_API_KEY -- python3 evals/prose_judge.py generate \
  --variant spindle_full_context --condition candidate --limit 5 --out /tmp/cand.json

# Run live multi-family Ollama Cloud judges (zhipu-glm, deepseek, google-gemma):
ai run OLLAMA_API_KEY -- python3 evals/prose_judge.py judge \
  --live --limit 5 --out evals/results/prose_judge_live_ollama_smoke.json
```

You can also compare saved candidate files directly via
`--baseline /path/to/base.json --candidate /path/to/cand.json` or compare two
prompt variants from `evals/prose_judge/prompt_variants.json` in one step with
`--variant-a compact_baseline --variant-b spindle_full_context`.

## Human calibration (~100 pairs)

`prose_judge.py` reuses and extends `serial_fiction.py blind` (`--pairwise`) so
an operator can blind and label ~100 pairs without seeing condition names,
prompt variants, or model identities:

```sh
python3 evals/prose_judge.py blind --count 100 --out /tmp/prose-calibration --seed 20261107
# Operator fills preferred ("A", "B", or ["A", "B"] for tie) in /tmp/prose-calibration/ratings.json
python3 evals/prose_judge.py calibrate \
  --key /tmp/prose-calibration/private-key.json \
  --ratings /tmp/prose-calibration/ratings.json \
  --judge-results evals/results/prose_judge_offline_baseline.json
```

`calibrate` reports raw agreement, non-tie directional agreement, Cohen's kappa
between each judge (and panel consensus) and the human rater, the 3×3 confusion
matrix, and candidate/tie preference drift relative to human labels.

## What this kit does and does not establish

- **What it establishes:** reproducible pairwise comparisons between two drafting
  configurations or saved candidate sets across 30 structured scene briefs,
  explicit accounting for presentation-order position bias via swapped-order
  double judging, inter-judge agreement across distinct model families, and
  quantified judge-human alignment when paired with a blinded human calibration
  set.
- **What it does NOT establish:** LLM judges are automated proxies with known
  failure modes (surface fluency bias, length sensitivity, and blind spots on
  long-horizon structural payoff). Uncalibrated LLM-judge win rates do not prove
  human reader preference or publication-grade novel quality across a full book.
  Use blinded human calibration ratings before treating proxy win-rate gains as
  validated story-quality improvements.

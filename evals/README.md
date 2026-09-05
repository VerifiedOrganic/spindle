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

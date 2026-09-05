# Serial fiction implementation

This specification covers long-running serial fiction. It preserves existing MCP contracts, local SQLite ownership, rating clearance, canon ratification, and resumable host/agent authoring.

## Scope and decisions

- One published episode initially corresponds to one existing chapter. Releases are immutable local snapshots; corrections create a new revision. No platform posting.
- Story history records actual events. Planned payoff dates are never substituted for unknown actual dates.
- Recent story context and unresolved commitments get dedicated space. Historical drafting must not receive summaries from later chapters.
- Host-assisted AI authorship is explicit; human writing remains human by default. Style learning remains opt-in and reviewable.
- Reader memory carries across runs/books only while its source manuscript and reader contract remain valid.
- Craft feedback creates reviewable editorial work. It never silently changes canon or forces a rewrite.
- The existing console and MCP service remain the product surface. Use native HTML/JavaScript and existing service operations.
- Evaluation measures continuity, preferences, effort, and model usage separately. Human preferences are not fabricated.

## Work ledger

| Slice | Assessment | Acceptance | Status |
|---|---|---|---|
| Story history | E-024 | Actual payoff and reopening history drives cursor-bound recaps; legacy unknown dates remain unknown | implemented; focused tests pass |
| Context | E-015 | Recent books and open commitments survive bounded context; future summaries excluded | implemented; focused tests pass |
| Author feedback | E-020 | Explicit host-AI drafts produce opt-in edit examples; human-over-human edits excluded | implemented; focused tests pass |
| Agent entry | E-017 | Small complete authoring profile with tested workflow coverage | implemented; focused tests pass |
| Usage and coverage | E-016 | Nullable actual usage, elapsed time, and explicit audit coverage exposed | implemented; focused tests pass |
| Reader memory | E-021 | Validated branch/source-scoped memory survives new runs and book boundaries | implemented; focused tests pass |
| Editorial loop | E-018/E-022 | Evidence-backed concerns and canon/plan decisions actionable in the existing console | implemented; regression tests and live editorial acceptance pass |
| Releases | E-019 | Immutable chapter releases, correction revisions, published cursor and draft backlog | implemented; focused tests pass |
| Serial planning | E-023 | Series/arc/episode recipe and agent guidance use existing planning models | implemented |
| Evaluation | E-014 | Runnable fixed cases, blinded comparisons, continuity checks, and honest score aggregation | implemented; live fixture capture and self-check pass |
| Integration | all | Workspace tests, format, Clippy, and browser checks for console changes | implemented; automated and browser checks pass; both visual-review fixes resolved |

## Validation

Use focused regressions for changed behavior, then repository gates: `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`. Keep tests about historical state, isolation, idempotency, invalid inputs, and complete workflows. Record results in this ledger as slices finish.

## Verification notes

- Baseline: 603/606 adapter unit tests passed. Three failures involved scene delimiters. Offset repair now honors the dedicated marker; two old fixtures use that current marker.
- New focused checks pass for promise history, 240-chapter context, host-AI edit learning, provider usage, audit paging, and immutable releases.
- First broad run: all integration suites, 183 core unit tests, 52 harness tests, and 149 MCP tests passed. Two adapter expectations still assumed old context behavior; those fixtures are corrected.

- Reader persistence, source-validated editorial decisions and releases pass their focused tests. A real console session accepted an editorial request in a disposable project.
- The comparison kit captured all 12 cases through MCP, produced 36 condition requests, excluded a future confession from historical context, and retained both recent events and the oldest open promise in a 200-chapter synopsis fixture.
- That fixture exposed implicit webserial pacing/hook enforcement. Style intent now requires creative pacing/ending signals; publication format alone cannot force them.
- A parallel CLI fixture exposed dispatch through a shared environment override. Configured CLI endpoints now stay bound to the cleared agent; legacy unconfigured routes keep the environment fallback. Fixture environment mutations were removed, and a two-agent routing check passes.
- Final workspace suite: **1,069 passed, 0 failed, 2 ignored**. `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check` pass.
- Live browser checks used an isolated temporary database and deterministic local responses: MCP session startup, accepting an editorial request, previewing the exact chapter, recording a release, and reopening the immutable released prose all worked. No production manuscript was modified or released.
- The evaluation self-check passes balanced blinding, ties, missing ratings, zero/unknown usage, and invalid metrics. Final context captures produce 36 requests across 12 synthetic cases. This verifies the kit and context behavior; it does **not** establish better stories, author preference, or production model quality. Those require generating the candidates with recorded model versions and completing the blinded human ratings described in `evals/README.md`.
- `PRODUCT.md` records the product scope and principles. One shared request helper replaces loading text on failures without letting old requests overwrite newer views. `node crates/spindle-mcp/tests/console_episode_requests.cjs` passes all three failure paths, retry guidance, stale-response suppression, and project switching.
- After the console correction, all 150 MCP tests pass and MCP Clippy remains clean. A live failure check confirmed the inline error and retry guidance using only a disposable local server. Both requested review fixes were verified as resolved.
- `DESIGN.md` and `.impeccable/design.json` record the finished console's existing styles and components. The extracted heading weights, prose headings, line heights, and control states were reconciled with the actual CSS; no new visual identity was introduced.

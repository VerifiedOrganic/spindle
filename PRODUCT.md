# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Authors creating long-running webseries and other long-form fiction with AI writing agents.

## Product Purpose

Spindle preserves the story's state across chapters, books, revisions, and agent sessions. Authors can plan, draft, check continuity, review reader concerns, and preserve released episodes without rebuilding context for every writing session.

## Operating Context

Spindle is a local Rust/SQLite service used through MCP clients and an embedded browser console. Configured model providers receive the context for requested model calls. The console supports series status, episode previews and local releases, editorial decisions, narrative threads, run monitoring, manuscript reading, and canon/plan review.

## Capabilities and Constraints

- A released episode is an immutable snapshot of one chapter. Corrections append revisions. Recording a release does not post to an external platform.
- Reader memory is scoped to the manuscript branch, source history, reader contract, and configured model route. Retcons invalidate derived readings; missing readings remain explicit gaps.
- Reader concerns become author-reviewable editorial work. Accepted author intent informs later drafting; canon and plan changes require explicit decisions.
- Model usage and audit coverage distinguish observed results from unknown or skipped work.
- Series/arc/episode planning uses existing story models. Serial publication alone does not prescribe fast pacing or cliffhanger endings.

## Evidence on Hand

`docs/serial-fiction-implementation.md` records implementation and verification. `evals/` contains twelve synthetic comparison cases and a blinded rating workflow. These fixtures establish behavior, not author preference or superior story quality; no such quality result has been measured.

## Product Principles

- Preserve author control over story intent, prose, canon, and release decisions.
- Keep actual story history distinct from future plans.
- Make stale context, incomplete review, and unknown usage visible.
- Improve story quality through measured comparison and human judgment.

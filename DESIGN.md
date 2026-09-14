---
name: Spindle Operator Console
description: Incumbent dark Operate console for long-running webseries authoring.
colors:
  neutral-bg: "#14161a"
  surface-panel: "#1c1f26"
  surface-raised: "#23272f"
  border-line: "#313742"
  text-primary: "#d7dbe2"
  text-muted: "#8b93a1"
  action-blue: "#6ea8fe"
  status-success: "#5cd18b"
  status-warning: "#e0b64a"
  status-error: "#e56b6b"
  action-ink: "#0c0f14"
  warning-surface: "#201d14"
typography:
  body:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  title:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "22px"
    fontWeight: 700
    lineHeight: 1.25
  label:
    fontFamily: "system-ui, -apple-system, Segoe UI, Roboto, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.5
  mono:
    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: "normal"
  manuscript:
    fontFamily: "Georgia, serif"
    fontSize: "17px"
    fontWeight: 400
    lineHeight: 1.7
rounded:
  badge: "4px"
  control: "6px"
  panel: "8px"
  pill: "10px"
spacing:
  control: "6px"
  tight: "8px"
  row: "12px"
  card: "14px"
  main: "16px"
  section: "20px"
  heading: "22px"
components:
  button-default:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  button-primary:
    backgroundColor: "{colors.action-blue}"
    textColor: "{colors.action-ink}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "8px 12px"
  select:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text-primary}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
  card:
    backgroundColor: "{colors.surface-panel}"
    textColor: "{colors.text-primary}"
    rounded: "{rounded.panel}"
    padding: "14px"
  status-pill:
    textColor: "{colors.text-muted}"
    typography: "{typography.mono}"
    rounded: "{rounded.pill}"
    padding: "1px 7px"
  manuscript:
    textColor: "{colors.text-primary}"
    typography: "{typography.manuscript}"
---

# Design System: Spindle Operator Console

## Overview

**Creative North Star: "Incumbent Operate console"**

This is a compact, dark operator surface for authors managing long-running webseries. It keeps the existing charcoal layers, blue action language, native controls, and serif manuscript reading surface intact. The visual system is functional and restrained: state, source, decision, and release language carry the interface.

The console uses system sans for workspace chrome and Georgia for prose. It favors borders, tonal layering, and whitespace over decoration. Errors, loading, empty, stale, disabled, and immutable states remain visible in place so author control is clear.

**Key Characteristics:**

- Charcoal background with two raised surface levels.
- One blue accent for selection, action, focus, and caret states.
- Semantic green, gold, and red status colors.
- Compact controls with readable manuscript prose.
- No external assets, dependencies, shadows, or gradients.

## Colors

The palette is a restrained dark neutral field with a single cool blue action accent and explicit semantic status colors.

### Primary

- **Action Blue** (`#6ea8fe`): Selected navigation, primary actions, hover borders, focus outlines, text caret, and selection background.

### Neutral

- **Console Charcoal** (`#14161a`): Page background, active button surface, and payload background.
- **Panel Charcoal** (`#1c1f26`): Header, cards, feedback, and primary containers.
- **Raised Charcoal** (`#23272f`): Controls, nav buttons, scene cells, and legacy preformatted reading surfaces. Episode previews inherit the page background.
- **Divider Slate** (`#313742`): 1px borders, table rules, evidence rule, and separators.
- **Primary Text** (`#d7dbe2`): Main interface copy and control text.
- **Muted Slate** (`#8b93a1`): Labels, metadata, empty/loading copy, secondary prose, and quiet status text.

### Semantic

- **Success Green** (`#5cd18b`): Completed, clean, staged, and committed states.
- **Warning Gold** (`#e0b64a`): Medium priority, notes, and review cautions.
- **Error Red** (`#e56b6b`): Failed, blocked, skipped, and error states.
- **Action Ink** (`#0c0f14`): Text on the blue primary action and selected navigation.
- **Warning Surface** (`#201d14`): Tonal background for review notes.

**The One Accent Rule.** Keep blue for selection, action, focus, and direct interaction. Use green, gold, and red for state meaning rather than decoration.

## Typography

**Display Font:** None; headings use `system-ui, -apple-system, Segoe UI, Roboto, sans-serif`.

**Body Font:** `system-ui, -apple-system, Segoe UI, Roboto, sans-serif`.

**Label/Mono Font:** `ui-monospace, SFMono-Regular, Menlo, monospace` for badges, status pills, event timelines, identifiers, and compact labels.

**Reading Font:** `Georgia, serif` for manuscript and episode prose.

**Character:** UI type is quiet, compact, and familiar. Serif prose creates a clear reading mode without changing the surrounding operating surface.

### Hierarchy

- **Title** (browser-default bold, `22px`, `1.25`): Pane headings and episode/release detail headings.
- **Section title** (browser-default bold, `16px`, inherited `1.5`): Card and work-item headings.
- **Body** (400, `14px`, `1.5`): Controls, descriptions, tables, status summaries, and actions.
- **Label** (400, `12px`, `1.5`): Field labels, table headings, notes, and supporting metadata.
- **Mono status** (400, `11px`, normal line height): Badges and status pills; event timelines use `12px`, while payloads use `11px`.
- **Manuscript** (400, `17px`, `1.7`): Compiled books, episode previews, and released prose, constrained to `72ch`.

**The Reading Voice Rule.** Use Georgia only where the user reads story prose; keep navigation, controls, decisions, and metadata in the system sans voice.

## Layout

The header is a flex row with `10px 16px` padding, a `16px` gap, and the workspace navigation pushed to the right. The main content is centered at a maximum width of `1100px` with `16px` padding. Rows wrap with a `12px` gap and `14px` bottom margin; cards and work items create separation with `14px` to `20px` vertical rhythm.

Scene status uses an auto-fill grid with `minmax(140px, 1fr)` columns and an `8px` gap. Episode lists use a full-width table inside a horizontal scroll container on wide screens. At `max-width: 760px`, navigation wraps to a full-width row, buttons reach `42px` minimum height, project controls flex, event rows wrap, and episode table rows become stacked blocks without requiring horizontal scrolling. Reading panels scroll vertically with a `60vh` maximum height; the manuscript measure remains `72ch`.

## Elevation & Depth

The console has no box shadows. Depth comes from the background (`#14161a`), panel (`#1c1f26`), and raised (`#23272f`) tonal steps, reinforced by single-pixel divider borders. Notes use a warm warning surface with a warning border; state pills use borders and semantic text rather than glow.

**The Flat Surface Rule.** Use tonal layers and 1px borders to separate work areas. Do not introduce shadows or decorative effects into this incumbent surface.

## Shapes

The form language is gently squared and consistent: `4px` for the local-workspace badge, `6px` for controls and small cells, `8px` for cards and reading panels, and `10px` for status pills. Borders are `1px` and use the divider slate. Controls use compact padding; reading panels and cards get more interior room. There are no decorative masks, gradients, or image treatments.

## Components

### Buttons

Buttons are compact native controls with clear state changes.

- **Shape:** `6px` radius, `8px 12px` padding, `36px` minimum height; mobile buttons use `42px` minimum height.
- **Default:** Raised charcoal background, primary text, 1px divider border.
- **Primary:** Action blue background and border with action ink text.
- **Hover / Focus / Active:** Hover changes the border to action blue; active changes the background to page charcoal; focus-visible uses a 2px action-blue outline with `3px` offset.
- **Disabled:** Reduced to `55%` opacity with the default cursor.

### Status Pills

Pills are compact, bordered state labels rather than decorative tags.

- **Style:** Transparent tonal surface, 1px divider border, `10px` radius, `1px 7px` padding, and `11px` mono type.
- **State:** Muted for pending/low; blue for in-progress/draft; green for completed/clean/staged; gold for medium; red for blocked/findings/error/skipped.

### Cards / Containers

- **Corner Style:** `8px` radius for console cards, with `14px` padding and `14px` bottom margin.
- **Background:** Panel charcoal; inner scene cells use raised charcoal. Payloads and episode reading surfaces use the page background.
- **Shadow Strategy:** None; depth follows the tonal layers and divider borders in Elevation & Depth.
- **Border:** 1px divider slate.
- **Internal Padding:** `14px` for cards, `8px` for scene cells, `12px` for feedback, and `16px` for reading surfaces.

### Inputs / Fields

- **Style:** Native `select` and number inputs use raised charcoal, primary text, a 1px divider border, `6px` radius, and `6px 10px` padding. Textareas use page charcoal, `10px` padding, `80px` minimum height, and vertical resize.
- **Focus:** A 2px action-blue `:focus-visible` outline with `3px` offset; textareas use the action blue caret and muted placeholder text.
- **Error / Disabled:** Inline role-alert text and error-colored state copy; disabled buttons retain layout and reduce opacity.

### Navigation

The workspace navigation is a wrapping row of native buttons in the header. Resting buttons use raised charcoal with divider borders; `aria-pressed="true"` uses action blue with action ink text and border. At mobile widths navigation becomes a full-width wrapped row with `6px` gaps and `42px` minimum button height.

### Manuscript Reader

Compiled and released prose uses a `72ch` Georgia column at `17px/1.7`. Episode previews and released versions sit in bordered, vertically scrolling prose containers; the manuscript pane uses the same readable measure while preserving source and release metadata in system sans.

Prose headings retain Georgia with browser-default bold weight; the second-level prose heading uses `17px/1.25` and `16px 0 6px` margins. Episode preview containers have square corners; rounded legacy preformatted readers are a separate style.

### Review Work Item

Editorial, canon, and plan queues use border-separated work items with `20px` vertical padding. Evidence is italic muted text with a 1px divider rule; proposed payloads stay in expandable native `details` blocks and compact mono preformatted surfaces. Actions remain grouped in a wrapping row.

## Do's and Don'ts

### Do:

- **Do** keep the charcoal background/panel/raised hierarchy and divider slate as the base surface language.
- **Do** reserve action blue for selected navigation, primary actions, hover borders, focus, caret, and selection.
- **Do** use system sans for operating UI and Georgia for manuscript prose.
- **Do** preserve native `select`, number input, textarea, `details`, and `summary` behavior, then style them with the existing tokens.
- **Do** keep errors, loading, empty, stale, blocked, disabled, and immutable-release states inline and readable.
- **Do** keep mobile episode rows stacked at `760px` and preserve readable prose at `72ch` on wider screens.

### Don't:

- **Don't** introduce a new visual identity, display face, illustration, external asset, dependency, request, gradient, or shadow into this console surface.
- **Don't** use blue as a general decoration or semantic substitute for green, gold, or red state colors.
- **Don't** turn status pills, bordered cards, or native controls into soft-shadowed or oversized marketing surfaces.
- **Don't** hide source changes, stale review context, missing prose, failed requests, or release immutability behind a blank pane.

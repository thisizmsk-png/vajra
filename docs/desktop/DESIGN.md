---
version: alpha
name: Vajra Desktop
description: Dense, keyboard-first macOS tool UI over the Vajra harness. Dark theme from the brand art, derived light theme, one brand color for the primary action and live highlights.
colors:
  background: "#0d1117"
  surface: "#161b22"
  surface-elevated: "#1c2128"
  accent: "#21262d"
  border: "#30363d"
  border-strong: "#768390"
  text: "#e6edf3"
  text-secondary: "#c9d1d9"
  text-muted: "#9da7b3"
  primary: "#d97757"
  text-on-primary: "#0d1117"
  success: "#7ee787"
  warning: "#e3b341"
  danger: "#ff7b72"
  info: "#79c0ff"
  focus-ring: "#79c0ff"
  light-background: "#f6f8fa"
  light-surface: "#fdfdfe"
  light-surface-elevated: "#ffffff"
  light-accent: "#eaeef2"
  light-border: "#d8dee4"
  light-border-strong: "#6e7781"
  light-text: "#1f2328"
  light-text-secondary: "#424a53"
  light-text-muted: "#59636e"
  light-primary: "#a84a29"
  light-text-on-primary: "#ffffff"
  light-success: "#116329"
  light-warning: "#7d4e00"
  light-danger: "#b42318"
  light-info: "#0550ae"
  light-focus-ring: "#0550ae"
typography:
  headline-lg: {fontFamily: -apple-system, fontSize: 20px, fontWeight: 600, lineHeight: 1.3}
  headline-md: {fontFamily: -apple-system, fontSize: 15px, fontWeight: 600, lineHeight: 1.35}
  body-md: {fontFamily: -apple-system, fontSize: 13px, fontWeight: 400, lineHeight: 1.45}
  body-sm: {fontFamily: -apple-system, fontSize: 12px, fontWeight: 400, lineHeight: 1.4}
  label-md: {fontFamily: -apple-system, fontSize: 13px, fontWeight: 500, lineHeight: 1.2}
  label-sm: {fontFamily: -apple-system, fontSize: 11px, fontWeight: 500, lineHeight: 1.3}
  mono-md: {fontFamily: ui-monospace, fontSize: 12px, fontWeight: 400, lineHeight: 1.5}
  mono-number: {fontFamily: -apple-system, fontSize: 12px, fontWeight: 400, lineHeight: 1.4, fontFeature: '"tnum" 1'}
rounded: {none: 0px, sm: 4px, md: 6px, full: 9999px}
spacing: {xs: 4px, sm: 8px, md: 12px, lg: 16px, xl: 24px, xxl: 32px, row: 28px, target-min: 24px, sidebar-collapsed: 56px, sidebar: 220px, dock: 320px, statusbar: 24px}
components:
  button-primary: {backgroundColor: "{colors.primary}", textColor: "{colors.text-on-primary}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  button-secondary: {backgroundColor: "{colors.surface-elevated}", textColor: "{colors.text}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  button-danger: {backgroundColor: "{colors.surface-elevated}", textColor: "{colors.danger}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  list-row: {backgroundColor: "{colors.background}", textColor: "{colors.text}", typography: "{typography.body-md}", height: 28px, padding: 8px}
  list-row-selected: {backgroundColor: "{colors.accent}", textColor: "{colors.text}"}
  input: {backgroundColor: "{colors.surface}", textColor: "{colors.text}", typography: "{typography.body-md}", rounded: "{rounded.md}", height: 28px, padding: 8px}
  input-outline: {backgroundColor: "{colors.border-strong}", width: 1px}
  separator: {backgroundColor: "{colors.border}", width: 1px}
  focus-ring: {backgroundColor: "{colors.focus-ring}", width: 2px}
  card: {backgroundColor: "{colors.surface}", textColor: "{colors.text}", rounded: "{rounded.md}", padding: 12px}
  section-label: {textColor: "{colors.text-secondary}", typography: "{typography.label-md}"}
  status-success: {textColor: "{colors.success}", typography: "{typography.label-sm}"}
  status-warning: {textColor: "{colors.warning}", typography: "{typography.label-sm}"}
  status-danger: {textColor: "{colors.danger}", typography: "{typography.label-sm}"}
  status-info: {textColor: "{colors.info}", typography: "{typography.label-sm}"}
  status-dot: {rounded: "{rounded.full}", size: 8px}
  badge: {textColor: "{colors.text-secondary}", typography: "{typography.label-sm}", rounded: "{rounded.sm}", padding: 4px}
  terminal-pane: {backgroundColor: "{colors.background}", textColor: "{colors.text}", typography: "{typography.mono-md}", padding: 8px}
  status-bar: {backgroundColor: "{colors.surface}", textColor: "{colors.text-muted}", typography: "{typography.mono-number}", height: 24px}
  window-light: {backgroundColor: "{colors.light-background}", textColor: "{colors.light-text}"}
  surface-light: {backgroundColor: "{colors.light-surface}", textColor: "{colors.light-text-secondary}"}
  popover-light: {backgroundColor: "{colors.light-surface-elevated}", textColor: "{colors.light-text-muted}"}
  list-row-selected-light: {backgroundColor: "{colors.light-accent}", textColor: "{colors.light-text}"}
  button-primary-light: {backgroundColor: "{colors.light-primary}", textColor: "{colors.light-text-on-primary}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  separator-light: {backgroundColor: "{colors.light-border}", width: 1px}
  input-outline-light: {backgroundColor: "{colors.light-border-strong}", width: 1px}
  focus-ring-light: {backgroundColor: "{colors.light-focus-ring}", width: 2px}
  status-success-light: {textColor: "{colors.light-success}"}
  status-warning-light: {textColor: "{colors.light-warning}"}
  status-danger-light: {textColor: "{colors.light-danger}"}
  status-info-light: {textColor: "{colors.light-info}"}
---

# Vajra Desktop design system

Companion to [UX.md](UX.md). Front matter tokens are normative (dark is the default set; `light-*` is
the light theme). This file follows the UI design SOP; its anti-slop rules override any other skill.

## Overview

A working tool, not a showcase: dense rows, quiet chrome, text that reads fast, and one warm brand
color (#d97757, "Vajra clay") that marks the primary action and what is live right now. Direction
borrows Linear's density and Raycast's palette, on a GitHub-dark base that the brand art already uses.

**Stack note.** Stack-agnostic until the spike decides; Tauri v2 with a Rust core is the option to
beat. Component pattern is shadcn-style: the project owns the source in one kit folder
(`components/ui`), built on Radix-like accessible primitives, variants with `cva`, class merging
with `cn()` (clsx + tailwind-merge). Tokens are CSS variables consumed as
`rgb(var(--c-primary) / <alpha-value>)`, so a theme swaps variables and nothing else. If a native
toolkit wins the spike, the same token roles map to native theme objects.

**Role names.** The UI shows generic names. This is the only place engine ids appear in design docs:

| Engine id | UI name | Engine id | UI name |
|---|---|---|---|
| krishna | Lead | draupadi | Product |
| arjuna | Architect | dhrishtadyumna | Release planner |
| bhima | Backend | yudhishthira | Arbiter |
| nakula | Frontend | abhimanyu | Prototyper |
| vidura | Verifier | karna | Numerics |
| bhishma | Security | sahadeva | Analyst |
| shakuni | Red team | hanuman | Platform |
| duryodhana | Critic (pre-mortem) | ashwatthama | Incident lead |
| drona | Acceptance | sepoy-backend-1 etc. | Backend worker 1 etc. |

Unknown ids render title-cased. The worker Task tab has an "Engine id" field; nothing else shows ids.

## Colors

**Dark theme sources** (exact pixels sampled from `docs/assets/vajra-social-preview.png`, 2560x1280):
background #0d1117 (canvas), surface #161b22 (chip fill), border #30363d (chip outline), text #e6edf3
(wordmark), text-secondary #c9d1d9 (chip text), text-muted #9da7b3 (body line), primary #d97757
(tagline), success #7ee787 ("verify" chip). The URL gray #6e7681 is 4.12:1 on the canvas, under 4.5,
so it is not used for text. Derived (not in the art): surface-elevated, accent, border-strong,
warning, danger, info, focus-ring, ANSI. **The light theme is derived; no light brand art exists.**
Its primary is a darker clay #a84a29 because #d97757 on #f6f8fa is 2.93:1.

| Role | Use |
|---|---|
| background | Window canvas, terminal, stream |
| surface | Sidebar, status bar, cards, inputs |
| surface-elevated | Popovers, dialogs, palette, secondary buttons |
| accent | Selected row, hover row, active tab (a neutral, not a hue) |
| border | Decorative separators and pane edges (never the only cue) |
| border-strong | Input and control outlines (3:1 rule) |
| text / text-secondary / text-muted | Primary content / labels / metadata and timestamps |
| primary | One primary button per view, live node indicator, progress of the current phase |
| text-on-primary | Label on primary fill |
| success / warning / danger / info | Status only, always with a shape and a label |
| focus-ring | 2 px ring, 2 px offset on background |

**Contrast** (WCAG 2.x ratio, computed with a throwaway script in /tmp). Text pairs need 4.5:1;
border-strong and focus-ring need 3:1. Status colors are used as text and icon color on these four
backgrounds; there are no tinted status fills, so no tinted pairs exist.

| Dark fg on: | background | surface | surface-elevated | accent |
|---|---|---|---|---|
| text #e6edf3 | 16.02 | 14.64 | 13.70 | 12.88 |
| text-secondary #c9d1d9 | 12.26 | 11.21 | 10.49 | 9.86 |
| text-muted #9da7b3 | 7.76 | 7.09 | 6.64 | 6.24 |
| primary #d97757 | 6.06 | 5.54 | 5.18 | 4.88 |
| success #7ee787 | 12.32 | 11.26 | 10.53 | 9.91 |
| warning #e3b341 | 9.72 | 8.89 | 8.32 | 7.82 |
| danger #ff7b72 | 7.51 | 6.86 | 6.42 | 6.04 |
| info, focus-ring #79c0ff | 9.73 | 8.89 | 8.32 | 7.82 |
| border-strong #768390 | 4.88 | 4.46 | 4.18 | 3.93 |

| Light fg on: | background #f6f8fa | surface #fdfdfe | surface-elevated #ffffff | accent #eaeef2 |
|---|---|---|---|---|
| text #1f2328 | 14.84 | 15.54 | 15.80 | 13.55 |
| text-secondary #424a53 | 8.44 | 8.84 | 8.99 | 7.71 |
| text-muted #59636e | 5.74 | 6.01 | 6.11 | 5.24 |
| primary #a84a29 | 5.37 | 5.62 | 5.71 | 4.90 |
| success #116329 | 6.94 | 7.27 | 7.39 | 6.34 |
| warning #7d4e00 | 6.66 | 6.97 | 7.09 | 6.08 |
| danger #b42318 | 6.18 | 6.47 | 6.57 | 5.64 |
| info, focus-ring #0550ae | 7.13 | 7.47 | 7.59 | 6.51 |
| border-strong #6e7781 | 4.27 | 4.47 | 4.55 | 3.90 |

**On primary.** #d97757 on #0d1117 is 6.06:1 (passes). White on #d97757 is 3.12:1 and **fails** for
13 px labels, so dark text-on-primary is #0d1117 (6.06:1). Light: white on #a84a29 is 5.71:1.
Fixes made while computing: light primary moved from #b4532f (4.27 on accent) to #a84a29; light
success, warning, info darkened off GitHub defaults (4.17 to 4.45 on accent); border-strong replaced
the source gray #6e7681 (3.31 on accent); ANSI entries below adjusted where they fell under 4.5.

**Terminal ANSI 16.** Every entry is at least 4.5:1 on its theme's background (min 5.07 dark, 4.63
light), because agents print in all 16 colors.

| Slot | Dark | Light | Slot | Dark | Light |
|---|---|---|---|---|---|
| black | #7d8590 | #24292f | bright black | #8b949e | #57606a |
| red | #ff7b72 | #cf222e | bright red | #ffa198 | #a40e26 |
| green | #3fb950 | #116329 | bright green | #56d364 | #1a7f37 |
| yellow | #d29922 | #4d2d00 | bright yellow | #e3b341 | #633c01 |
| blue | #58a6ff | #0969da | bright blue | #79c0ff | #0550ae |
| magenta | #bc8cff | #8250df | bright magenta | #d2a8ff | #6639ba |
| cyan | #39c5cf | #1b7c83 | bright cyan | #56d4dd | #0f6270 |
| white | #b1bac4 | #636c76 | bright white | #f0f6fc | #59636e |

Dark black was #484f58 (2.28) and light bright white #8c959f (2.85) in the GitHub sets; both changed.

## Typography

System fonts only, no webfonts: UI `-apple-system, BlinkMacSystemFont` (SF Pro), mono
`ui-monospace, "SF Mono", Menlo`. Three weights at most: 400, 500, 600. No letter-spacing anywhere
(SF Pro tracks itself by size). Numbers in tables, counts, timestamps, memory and exit codes use
tabular numerals (`font-variant-numeric: tabular-nums`, token `mono-number`).

| Token | Size / line | Weight | Use |
|---|---|---|---|
| headline-lg | 20 / 26 | 600 | Screen title (one per screen) |
| headline-md | 15 / 20 | 600 | Section heading, dialog title |
| body-md | 13 / 19 | 400 | Default text, rows, chat |
| label-md | 13 / 16 | 500 | Buttons, tabs, selected labels |
| body-sm | 12 / 17 | 400 | Metadata, status bar, table secondary columns |
| label-sm | 11 / 14 | 500 | Keyboard hints, badge text (sentence case, never all caps) |
| mono-md | 12 / 18 | 400 | Terminal, stream, diff, commands, paths |

Hierarchy is size and weight first, then text-secondary/muted, never the brand color.

## Layout

4 px base: 4, 8, 12, 16, 24, 32. Rows are 28 px (compact desktop); hit targets never under 24x24 px.
Sidebar 220 px, collapsed 56 px under 1200 px window width. Status bar 24 px. Chat dock 320 px
(min 280). Panes are resizable with a 4 px drag handle (8 px hit area, keyboard resizable with
arrow keys when focused) and min widths: crew tree 200, worker pane 480, inbox list 320, detail 400.
Minimum window 960x600; at that size the dock becomes an overlay sheet (see UX.md, section 4).
Content max line length 80ch for reports and chat.

## Elevation & Depth

Flat. Depth is three tonal layers (background, surface, surface-elevated) plus 1 px borders. Only
floating layers (palette, dialog, popover, toast) get one shadow, `0 8px 24px rgb(0 0 0 / 0.4)` dark
and `0 8px 24px rgb(31 35 40 / 0.12)` light, and a scrim of background at 60% for modal dialogs.

## Shapes

Radius 4 px for badges, inputs in tables and tooltips; 6 px for buttons, inputs, cards, dialogs;
full only for StatusDot. Nothing larger. Same radius family per view.

## Components

Kit folder owns the source. Screens compose these and never restyle them inline.

| Component | Variants | Purpose and states | A11y |
|---|---|---|---|
| Text | headline-lg, headline-md, body, body-sm, label, mono | All type goes through it | Semantic element prop |
| Button | primary, secondary, ghost, danger; sm 24, md 28 | Actions; hover, pressed, focus, disabled, busy (label changes, e.g. "Stopping") | `aria-disabled`, label on icon-only |
| Card | default, interactive | Groups a detail block; no stacked identical cards | Interactive is a button or link |
| Badge | neutral, status | Small label next to text, outline not fill | Text label always |
| Input | text, textarea, select, number | Forms, filters; error text below, never color alone | `<label>`, `aria-invalid`, `aria-describedby` |
| ListRow | default, selected, with meta | 28 px rows in every list | `option` or `row` role per parent |
| Section | with heading, with action | Groups content under a headline-md | `<section aria-labelledby>` |
| SegmentedControl | 2 to 4 options | Max rounds, theme | `radiogroup` + `radio` |
| Switch | default | Notification toggles | `role="switch"`, announced once |
| Sheet | right, center | Message, redirect, reject reason | Focus trap, Esc closes, returns focus |
| AlertDialog | destructive, confirm | Stop worker, approve plan, take over | Safe button default focus for destructive |
| Toast | info, success, danger | Result of an action, 5 s, pausable on hover | `role="status"`, never the only record |
| Skeleton | row, block | Shown only after 300 ms | `aria-busy` on container |
| Separator | horizontal, vertical | Divides panes and groups | `role="separator"` |
| Tabs | default | Worker pane tabs | `tablist`, arrow keys |
| Table | default, expandable rows | Criteria, task list fallback | Header cells, sortable `aria-sort` |
| Tooltip | default | Collapsed sidebar labels, truncated names, shortcut hints | Not the only source of info |
| Command palette | default | Cmd-K fuzzy actions and navigation | `combobox` + `listbox` |
| Resizable panes | horizontal, vertical | Tree/pane, list/detail | Handle `role="separator"` with `aria-valuenow` |
| Terminal pane | read-only, taken over | Run session PTY; ANSI 16 palette, virtualized scrollback | Label "Run terminal, read only"; Cmd chords only |
| Crew tree | default | Lead, run, agents, sub-workers | `tree`/`treeitem`, `aria-expanded`, roving tabindex |
| StatusDot | per status below | Shape + color + text label, never dot alone | Label in accessible name |
| EvidenceRow | pass, fail, not run | Criterion, command, exit code, evidence file, commit | Row header is criterion id |
| ActionCard | message, redirect, stop request; queued, delivered, acknowledged, failed | Orchestrator or human control action, links to worker and audit line | `article` with heading; status in live region |
| Diff view | unified, file list | Worktree vs base, virtualized | Added/removed marked with +/- glyphs, not color only |

### Status vocabulary

| Status | Role color | Shape | Label |
|---|---|---|---|
| pending | text-muted | ○ empty circle | Pending |
| ready | info | ◌ dashed circle | Ready |
| claimed | info | ◐ half circle | Claimed |
| in_progress | primary (live highlight) | ● filled circle | In progress |
| done | success | ✓ check | Done |
| blocked | warning | ■ square | Blocked |
| question | warning | ◆ diamond | Question |
| failed | danger | ✕ cross | Failed |
| criterion pass | success | ✓ check | Pass |
| criterion fail | danger | ✕ cross | Fail |
| criterion unknown | text-muted | ○ empty circle | Not run |
| needs approval | warning | ▲ triangle | Needs approval |

Icons are drawn as SVG shapes from one icon set; the glyphs above describe the shape, not a font.

### Motion

Plain CSS transitions, 120 ms for hover and press, 180 ms for panels and sheets, `ease-out`. State
changes (status, card status) cross-fade color only. No motion on streams, terminals or lists:
new lines appear, nothing slides. No spinner without text; any wait over 1 s shows what it waits for.
`prefers-reduced-motion: reduce` sets every duration to 0.

## Do's and Don'ts

**Anti-slop (blocking):**
- Don't stack identical cards or put icons in colored circles.
- Don't use checkmark bullet lists, testimonials, invented stats or fake urgency.
- Don't use decorative or animated gradients, glows, glass, radial orbs or dot grids.
- Don't use ALL-CAPS tracked labels, letter-spacing, or meta strings joined with middle dots.
- Do use the brand color only for the primary action and live highlights; status colors only for status.
- Do build hierarchy from size, weight and spacing first, then color.
- Do write honest empty states; never fake data, sample charts or placeholder rows that look real.
- Don't show fake activity: no pulsing "thinking" dots without a real event, no invented progress bars.
- Don't use emoji for status, "AI sparkle" icons, or mahabharat names anywhere in the UI.
- Don't use Inter, Geist or Space Grotesk; system fonts only.
- Don't put hover animation on everything or drop shadows on flat content.

**Copy:** sentence case; plain verbs ("Start run", "Approve plan", "Stop worker"); say what happened
with numbers ("Stopped Backend at 14:22", "3 of 7 criteria pass"); no em dashes. Strings live in
one file; log visible wording in `docs/design/changed-strings.md`. Run copy through `humanizer` or
`clarify` before review.

## Verification

**Design-guard test** (runs in CI once the renderer exists). Fails on: raw hex, `rgb(`/`hsl(` values,
any `letter-spacing` or `tracking-*` class, inline `style` color properties, and named colors in
screens and kit components. Token files (`tokens.css`, theme config) are exempt. Baseline: 0 screens,
0 violations, because no UI exists yet; the target stays 0.

**Screenshot plan** (once screens exist). Playwright drives the renderer (web build of the UI with
a fixture engine state) at 1280x800 and 960x600, dark and light: every MVP screen plus palette,
dialogs and each empty, loading, error and engine-missing state. A click crawl presses every visible
control and records crashes, page errors, console errors, outbound requests and dead buttons; target
0 of each. Shots go to `docs/design/after/`. Then `vidura` checks accessibility independently.

design-md-lint: ran `npx @google/design.md lint` (CLI 0.4.0 from npm, in /tmp; the catalog pins git commit 6e5da53, which was not cross-checked against this npm build). Result: 0 errors, 0 warnings, 1 info (token summary) after fixing 23 orphaned-token warnings.

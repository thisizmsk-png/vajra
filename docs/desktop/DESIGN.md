---
version: alpha
name: Vajra Desktop
description: Graphite. A dense, keyboard-first macOS tool. Graphite neutrals in dark and light, an ink primary action, and one azure for focus, the selection's live state and the live line.
colors:
  background: "#101113"
  surface: "#16171a"
  surface-elevated: "#1d1e22"
  accent: "#24262b"
  border: "#2a2c31"
  border-strong: "#74787f"
  text: "#ededef"
  text-secondary: "#b9bbc2"
  text-muted: "#8f929b"
  primary: "#7aa7ff"
  text-on-primary: "#0b1020"
  action: "#ededef"
  text-on-action: "#101113"
  panel: "#141518"
  success: "#5fd38d"
  warning: "#f0b44c"
  danger: "#ff7070"
  info: "#6cc6d9"
  focus-ring: "#7aa7ff"
  light-background: "#fbfbfc"
  light-surface: "#f4f4f6"
  light-surface-elevated: "#ffffff"
  light-accent: "#e9e9ee"
  light-border: "#e1e1e6"
  light-border-strong: "#7c7f88"
  light-text: "#18191c"
  light-text-secondary: "#44464d"
  light-text-muted: "#5f626b"
  light-primary: "#2556c8"
  light-text-on-primary: "#ffffff"
  light-action: "#18191c"
  light-text-on-action: "#ffffff"
  light-panel: "#ffffff"
  light-success: "#1a7341"
  light-warning: "#855600"
  light-danger: "#c0282b"
  light-info: "#0b6b80"
  light-focus-ring: "#2556c8"
typography:
  headline-lg: {fontFamily: -apple-system, fontSize: 22px, fontWeight: 600, lineHeight: 1.27}
  headline-md: {fontFamily: -apple-system, fontSize: 15px, fontWeight: 600, lineHeight: 1.33}
  body-md: {fontFamily: -apple-system, fontSize: 13px, fontWeight: 400, lineHeight: 1.46}
  body-sm: {fontFamily: -apple-system, fontSize: 12px, fontWeight: 400, lineHeight: 1.42}
  label-md: {fontFamily: -apple-system, fontSize: 13px, fontWeight: 500, lineHeight: 1.23}
  label-sm: {fontFamily: -apple-system, fontSize: 11px, fontWeight: 500, lineHeight: 1.27}
  mono-md: {fontFamily: SF Mono, fontSize: 12px, fontWeight: 400, lineHeight: 1.5}
  mono-number: {fontFamily: -apple-system, fontSize: 12px, fontWeight: 400, lineHeight: 1.42, fontFeature: '"tnum" 1'}
rounded: {none: 0px, sm: 4px, md: 6px, lg: 10px, full: 9999px}
spacing: {xs: 4px, sm: 8px, md: 12px, lg: 16px, xl: 24px, xxl: 32px, control-sm: 24px, row: 28px, control-lg: 32px, target-min: 24px, sidebar: 240px, action-bar: 40px, dock: 320px}
components:
  button-primary: {backgroundColor: "{colors.action}", textColor: "{colors.text-on-action}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  button-secondary: {backgroundColor: "{colors.surface-elevated}", textColor: "{colors.text}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  button-danger: {backgroundColor: "{colors.surface-elevated}", textColor: "{colors.danger}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  list-row: {backgroundColor: "{colors.background}", textColor: "{colors.text}", typography: "{typography.body-md}", rounded: "{rounded.md}", height: 28px, padding: 8px}
  list-row-selected: {backgroundColor: "{colors.accent}", textColor: "{colors.text}"}
  input: {backgroundColor: "{colors.background}", textColor: "{colors.text}", typography: "{typography.body-md}", rounded: "{rounded.md}", height: 28px, padding: 8px}
  input-outline: {backgroundColor: "{colors.border-strong}", width: 1px}
  separator: {backgroundColor: "{colors.border}", width: 1px}
  focus-ring: {backgroundColor: "{colors.focus-ring}", width: 2px}
  card: {backgroundColor: "{colors.surface}", textColor: "{colors.text}", rounded: "{rounded.md}", padding: 12px}
  dialog: {backgroundColor: "{colors.surface-elevated}", textColor: "{colors.text}", rounded: "{rounded.lg}", padding: 16px}
  section-label: {textColor: "{colors.text-secondary}", typography: "{typography.label-md}"}
  status-working: {textColor: "{colors.text-secondary}", typography: "{typography.label-sm}"}
  status-success: {textColor: "{colors.success}", typography: "{typography.label-sm}"}
  status-warning: {textColor: "{colors.warning}", typography: "{typography.label-sm}"}
  status-danger: {textColor: "{colors.danger}", typography: "{typography.label-sm}"}
  status-info: {textColor: "{colors.info}", typography: "{typography.label-sm}"}
  status-muted: {textColor: "{colors.text-muted}", typography: "{typography.label-sm}"}
  badge: {backgroundColor: "{colors.accent}", textColor: "{colors.text-secondary}", typography: "{typography.label-sm}", rounded: "{rounded.sm}", height: 20px, padding: 6px}
  terminal-pane: {backgroundColor: "{colors.background}", textColor: "{colors.text}", typography: "{typography.mono-md}", padding: 8px}
  action-bar: {backgroundColor: "{colors.surface}", textColor: "{colors.text-secondary}", typography: "{typography.body-sm}", height: 40px}
  timestamp: {textColor: "{colors.text-muted}", typography: "{typography.mono-number}"}
  window-light: {backgroundColor: "{colors.light-background}", textColor: "{colors.light-text}"}
  surface-light: {backgroundColor: "{colors.light-surface}", textColor: "{colors.light-text-secondary}"}
  popover-light: {backgroundColor: "{colors.light-surface-elevated}", textColor: "{colors.light-text-muted}"}
  list-row-selected-light: {backgroundColor: "{colors.light-accent}", textColor: "{colors.light-text}"}
  button-primary-light: {backgroundColor: "{colors.light-action}", textColor: "{colors.light-text-on-action}", typography: "{typography.label-md}", rounded: "{rounded.md}", height: 28px, padding: 12px}
  separator-light: {backgroundColor: "{colors.light-border}", width: 1px}
  input-outline-light: {backgroundColor: "{colors.light-border-strong}", width: 1px}
  focus-ring-light: {backgroundColor: "{colors.light-focus-ring}", width: 2px}
  status-success-light: {textColor: "{colors.light-success}"}
  status-warning-light: {textColor: "{colors.light-warning}"}
  status-danger-light: {textColor: "{colors.light-danger}"}
  status-info-light: {textColor: "{colors.light-info}"}
---

# Vajra Desktop design system: Graphite

Companion to [UX.md](UX.md). Front matter tokens are normative (dark is the default set; `light-*` is
the light theme). This file follows the UI design SOP; its anti-slop rules override any other skill.

## Overview

Graphite is the owner's pick (6 November 2026) from three directions mocked up in
`app/ui/mockups/` (PR #42; the refined version is `mockups/a2/`). It is a working tool in the family of
fast keyboard-first trackers and launchers: graphite neutrals, dense rows, quiet chrome, and one azure that means "act here" or
"live now". The signature is the **action bar**: a footer on every screen that names what is selected
and lists its actions with their keys.

**Stack note (what is built today).** Tauri v2 with a Rust core, React UI in `app/ui`. The kit in
`components/ui` is hand-rolled: plain CSS classes over the CSS variables in `theme/tokens.css`,
native `<dialog>` for Sheet, AlertDialog and the command palette, native radios for SegmentedControl,
`role="switch"` for Switch, and `cn()` is a string join (not clsx, not tailwind-merge). There is no
Tailwind, Radix, cva or tailwind-merge. The decision (accepted by the orchestrator): keep the
hand-rolled kit and its tokens; add a Radix primitive per component only where an accessibility gap
needs it (none does today), and add `cva` only if variants outgrow the props. Colours are hex CSS
variables consumed as `var(--c-primary)`, so a theme swaps variables and nothing else. The other
tokens that exist are `--font-ui`, `--font-mono`, `--space-xs` to `--space-xl`, `--radius-sm`,
`--radius-md`, `--radius-lg`, `--ctl-sm`, `--row-h`, `--ctl-lg`, `--target-min`, `--sidebar-w`,
`--bar-h`, `--t-fast`, `--t-panel` and `--ease-out`. Type sizes are not tokens; they live in the
`Text` variants and the kit CSS, from the scale below.

## Art direction (app screens)

Written under the UI SOP with `interface-design` and an `impeccable` critique of the first mockups.

**Intent.** Who: a developer at a Mac who has started a crew and checks on it for hours, between
other work. They glance at what needs them, decide (approve a plan, answer a question, redirect, stop
a worker) and go back to their editor. Feel: a precise instrument. Quiet and dense, legible at a
glance, with keys shown next to every action so the mouse becomes optional.

**Domain (territory, not features).** Run, crew, worker, round, plan, approval, question, handoff,
stream, diff, worktree, terminal, evidence, criterion, campaign, verifier, guard.

**Colour world.** Graphite canvas (#101113), slate panels (#16171a), a raised fill (#1d1e22), a
pressed fill (#24262b), and one cold azure (#7aa7ff) like a pilot light. Status uses an instrument's
lights (green, amber, red, teal). Everything else is structure.

**Signature.** The **action bar**, 40 px, at the foot of every screen. Left: the selection, in words
("Backend, t2 Token bucket middleware"). Right: its actions, each with its keys, the primary first
("Approve plan ⌘↵"). People learn the shortcuts by reading the bar. It replaces the 24 px status bar
in UX.md section 2; the run health that sat there (guard, engine, memory) moves to the sidebar foot
(not built yet). UX.md sections 2 and 3 record the change. The bar lists only actions the screen can
run today: Check again on Home, none yet on Crew until Message, Redirect and Stop are wired.

**The primary action is ink.** The one primary button on a view is filled with the text colour
(`action`: near-white on dark, near-black on light), the way the native toolbars beside it are
(polish pass, owner decision 2026-11-07). It is the loudest thing on the view without being a hue.

**What azure means.** Only three things: focus, the live line (the run
that is running, the stream line being written), and a working status *inside the selected row*. A
busy crew must not turn blue, so working agents elsewhere are ink (`text-secondary`). Text never
uses azure except a link or a match highlight in the palette.

**Panels.** A list, table or form sits in a panel: a 1 px border, the large radius, a 44 px bar with
its title (and quiet meta or an action at the end), then the rows edge to edge. Table headers are a
quiet 34 px band in `surface`; rows are 52 px with two lines where there is a name and an id (goal
over slug). Times in lists read relative ("2 days ago") with the exact time as the tooltip. Status
badges are pills: a 10% tint and a 35% edge of the status colour behind status-coloured text.

**Window.** On macOS the title bar is an overlay: the traffic lights sit in the sidebar's top row,
beside the brand, and that row and every screen head drag the window. Settings sits at the foot of
the sidebar.

**Defaults rejected.**
- Identical stacked cards for workers → one row per worker, grouped by what it needs from you
  (Needs you, Working, Waiting, Done).
- A coloured rail on the selected row → the accent fill. No `border-left` wider than 1 px anywhere.
- Unicode glyphs as icons (✓, ▲) → one SVG set on a 16 px grid, 1.5 px stroke, `currentColor`.
- A hero metric block for "X of Y" → the run's own sentence ("3 of 7 criteria pass").
- A form with a submit button for the ask → a launcher: one field, settings as inline tokens
  (Base, Rounds, Workers, Guard), Start run at the right; its ⌘↵ hint shows while focus is in the form, where the key works. The sidebar's ⌘1 to ⌘7 hints are out of layout until hover or focus. Neither is drawn in the idle window, and Home sets the check-fix hint in the UI face, so idle loads one typeface (idle memory gate).
- Separate search and command fields → one ⌘K field for actions, runs and files.
- An empty state with sample data → the real Vajra crews and the one command that starts each.

**Hierarchy per screen (the focal element wins through size, weight, contrast or space).**
- **Home:** the launcher, under a headline-lg "Start a run". Below it, "Needs you" leads, then the
  running run, then recent runs. Lists, never cards.
- **Crew:** the selected worker's stream. The crew list on the left is grouped by attention; the
  pending approval is a band under the header; the review pane on the right lists files changed,
  commands run with exit codes, and the criteria Verify will check.
- **Inbox:** the open request. The list on the left is triage (J and K move, ⌘↵ approves); the
  detail shows the plan diff, what happens on approve and on reject, and the cost.
- **History:** the table. Sort headers are the only controls in the header row. Numbers are tabular.
- **Settings:** the check list. Each check is a row with its shape, label, detail and fix.

**Measured, not guessed.** Sizes were checked against the shipped CSS of two agent orchestration
apps on the owner's Mac (Competitor A and Competitor B, read-only): both use a 240 px sidebar and a 24/28/32 control
scale, and Competitor A's palette is 720 px wide at 14% from the top. Graphite matches those. It rejects their
negative heading letter-spacing, half-pixel type sizes, bundled webfont and 20 px composer radius.

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

Graphite replaces the earlier GitHub-dark palette and its clay primary. Both themes are designed
together; neither is derived from brand art. Neutrals are a near-neutral graphite with a faint cool
cast, so azure reads as the only hue in the chrome.

| Role | Use |
|---|---|
| background | Window canvas, terminal, stream, inputs (inset) |
| surface | Sidebar, action bar, cards, the review pane |
| surface-elevated | Popovers, dialogs, palette, secondary buttons |
| accent | Selected row, hover row, badge fill, segmented track (a neutral, not a hue) |
| border | Separators and pane edges (never the only cue) |
| border-strong | Input and control outlines, floating layer edges (3:1 rule) |
| text / text-secondary / text-muted | Content / labels and working status / metadata and timestamps |
| primary | Focus, the on switch, the live line, working status in the selected row |
| text-on-primary | Label on an azure fill (the skip link, the on switch) |
| action / text-on-action | The primary button's ink fill and its label |
| panel | The raised sheet that holds a list, table or form (`.v-panel`): white on light, one step above the background on dark |
| success / warning / danger / info | Status only, always with a shape and a label |
| focus-ring | 2 px ring, 2 px offset |

Non-hex tokens in `tokens.css`, per theme: `--shadow-floating`, `--c-live-tint` (azure at 10% / 7%,
behind the live stream line), `--c-warn-tint` (amber at 8% / 7%, behind the approval band),
`--c-kbd-on-primary` (key outline on a primary button), `--c-selection` (text selection) and
`--c-scrim` (behind modal layers, background at 60%). Text on the two tints, composited over the
background: the lowest pair is light danger on the live tint at 5.10:1, and dark text-muted on the
live tint is 5.24:1. Tints cost up to 1.4 points of contrast, so nothing near 4.5 goes on them.

**Contrast** (WCAG 2.x ratio; `tests/contrast.test.ts` recomputes every cell from `tokens.css`).
Text pairs need 4.5:1; border-strong and focus-ring need 3:1.

| Dark fg on: | background | surface | surface-elevated | accent |
|---|---|---|---|---|
| text #ededef | 16.16 | 15.33 | 14.24 | 12.95 |
| text-secondary #b9bbc2 | 9.85 | 9.35 | 8.68 | 7.89 |
| text-muted #8f929b | 6.07 | 5.76 | 5.35 | 4.87 |
| primary, focus-ring #7aa7ff | 7.92 | 7.51 | 6.98 | 6.34 |
| success #5fd38d | 10.07 | 9.55 | 8.87 | 8.07 |
| warning #f0b44c | 10.20 | 9.68 | 8.99 | 8.17 |
| danger #ff7070 | 7.02 | 6.66 | 6.19 | 5.63 |
| info #6cc6d9 | 9.64 | 9.15 | 8.50 | 7.73 |
| border-strong #74787f | 4.26 | 4.04 | 3.76 | 3.41 |

| Light fg on: | background #fbfbfc | surface #f4f4f6 | surface-elevated #ffffff | accent #e9e9ee |
|---|---|---|---|---|
| text #18191c | 17.00 | 16.00 | 17.58 | 14.53 |
| text-secondary #44464d | 9.11 | 8.58 | 9.42 | 7.79 |
| text-muted #5f626b | 5.89 | 5.55 | 6.09 | 5.04 |
| primary, focus-ring #2556c8 | 6.26 | 5.89 | 6.47 | 5.35 |
| success #1a7341 | 5.68 | 5.35 | 5.88 | 4.86 |
| warning #855600 | 6.10 | 5.74 | 6.31 | 5.21 |
| danger #c0282b | 5.67 | 5.34 | 5.87 | 4.85 |
| info #0b6b80 | 5.92 | 5.58 | 6.12 | 5.06 |
| border-strong #7c7f88 | 3.87 | 3.64 | 4.00 | 3.31 |

The lowest text pair is light danger on accent, 4.85:1. **On primary:** #0b1020 on #7aa7ff is 7.93:1;
white on #2556c8 is 6.47:1. **On action:** #101113 on #ededef is 16.16:1; white on #18191c is
17.58:1. On a panel every text and status colour stays at 4.5:1 or more (lowest dark text-muted, 5.87:1). Info is teal, not a second blue, so "ready" never reads as "live".

**Terminal ANSI 16.** Every entry is at least 4.5:1 on its theme's background (lowest 5.06 dark,
ansi black; 4.77 light, ansi cyan), because agents print in all 16 colours.

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

### Theme selection

The app follows the OS appearance: `prefers-color-scheme: light` selects the light set, anything else
(including no media query support) selects dark. `src/theme.ts` sets `data-theme` on the root before
the first render and follows OS changes; the values stay in `tokens.css`. There is no Settings toggle
yet. The kit Gallery has its own toggle (and `?theme=light` to open on light) and restores the OS
theme when it unmounts. Text selection, the caret and scrollbars use the theme.

## Typography

System fonts only, no webfonts: UI `-apple-system, BlinkMacSystemFont` (SF Pro), mono `"SF Mono",
ui-monospace, Menlo`. Three weights: 400, 500, 600. Whole pixels on a ratio near 1.2. No
letter-spacing anywhere (SF Pro tracks itself by size). Tabular numerals on counts, timestamps,
memory, diff counts and exit codes.

| Token | Size / line | Weight | Use |
|---|---|---|---|
| headline-lg | 22 / 28 | 600 | Screen title (one per screen), the open Inbox request |
| headline-md | 15 / 20 | 600 | Section heading, dialog title, worker name |
| body-md | 13 / 19 | 400 | Default text, rows, stream cards |
| label-md | 13 / 16 | 500 | Buttons, tabs, selected rows |
| body-sm | 12 / 17 | 400 | Metadata, group headings, the action bar |
| label-sm | 11 / 14 | 500 | Keys, badge text, status labels (sentence case, never all caps) |
| mono-md | 12 / 18 | 400 | Terminal, stream, diff, commands, paths |

The launcher ask is the one exception: 16 / 25 at 400, because it is the field people type a
paragraph into. Hierarchy is size and weight first, then text-secondary and text-muted, never azure.

## Layout

4 px base: 4, 8, 12, 16, 24, 32. Controls are 24 (small), 28 (rows, buttons, inputs) and 32 px
(search, palette field, toolbar tabs); hit targets never under 24x24 px. Sidebar 240 px; it holds
navigation, the runs in the current repository (live run first, with a spinner and a count of what
needs you) and the health line at its foot. Action bar 40 px. Chat dock 320 px (min 280). Panes are
resizable with a 4 px drag handle (8 px hit area, keyboard resizable with arrow keys when focused);
min widths: crew list 240, worker pane 480, review pane 280, inbox list 320, detail 400. Minimum
window 960x600; at that size the dock becomes an overlay sheet (UX.md section 4). The launcher and
reading columns stop at 800 px and 68ch; tables, the stream and the crew list use the full width.

## Elevation & Depth

Flat. Depth is three tonal layers (background, surface, surface-elevated) plus 1 px borders. Inputs
are inset: they sit on `background`, darker than the surface around them. Only floating layers
(palette, dialog, sheet, toast, tooltip) take a `border-strong` edge and the one shadow,
`--shadow-floating`, over the `--c-scrim` for modal layers. No blur, glass, gradients or glows; the
palette's backdrop is a flat scrim (vibrancy would cost memory against the 200 MiB budget).

## Shapes

Radius 4 px for badges, keys and tooltips; 6 px for buttons, inputs, rows and cards; 10 px for
floating layers (dialog, palette, toast) and the launcher; full only for StatusDot, the switch and
count pills. Same radius family per view.

## Components

The kit folder owns the source. Screens compose these and never restyle them inline.

| Component | Variants | Purpose and states | A11y |
|---|---|---|---|
| Text | headline-lg, headline-md, body, body-sm, label, mono | All type goes through it | Semantic element prop |
| Button | primary, secondary, ghost, danger; sm 24, md 28 | Actions; hover, pressed, focus, disabled, busy (label changes, e.g. "Stopping") | `aria-disabled`, label on icon-only |
| Card | default, interactive | Groups a detail block; no stacked identical cards | Interactive is a button or link |
| Badge | neutral, status | Small label next to text; accent fill, status-coloured text | Text label always |
| Input | text, textarea, select | Forms, filters; inset; error text below, never colour alone | `<label>`, `aria-invalid`, `aria-describedby` |
| ListRow | default, selected, with meta | 28 px rows in every list; selection is the accent fill | `option` or `row` role per parent |
| Section | with heading, with action | Groups content under a heading | `<section aria-labelledby>` |
| SegmentedControl | 2 to 4 options | View toggles, max rounds, theme; recessed track, raised choice | `radiogroup` + `radio` |
| Switch | default | Follow the live line, notification toggles | `role="switch"`, announced once |
| Sheet | right, center | Message, redirect, reject reason | Focus trap, Esc closes, returns focus |
| AlertDialog | destructive, confirm | Stop worker, take over | Safe button default focus for destructive |
| Toast | info, success, danger | Result of an action, 5 s, pausable on hover | `role="status"`, never the only record |
| Skeleton | row, block | Shown only after 300 ms | `aria-busy` on container |
| Separator | horizontal, vertical | Divides panes and groups | `role="separator"` |
| Tabs | default | Worker pane tabs; ink underline on the selected tab | `tablist`, arrow keys |
| Table | default, expandable rows | Criteria, task list fallback | Header cells, sortable `aria-sort` |
| Tooltip | default | Truncated names, shortcut hints | Not the only source of info |
| Command palette | default | ⌘K: actions, runs and files in one field; 720 px, 34 px rows | `combobox` + `listbox` |
| Resizable panes | horizontal, vertical | List/pane/review, list/detail | Handle `role="separator"` with `aria-valuenow` |
| Terminal pane | read-only, taken over | Run session PTY; ANSI 16 palette, virtualized scrollback | Label "Run terminal, read only"; Cmd chords only |
| Crew tree | default | Lead, run, agents, sub-workers; a drawn chevron (never a Unicode arrow) opens a group; long names truncate with a tooltip | `tree`/`treeitem`, `aria-expanded`, roving tabindex |
| StatusDot | per status below | Shape + colour + text label, never a dot alone | Label in accessible name |
| EvidenceRow | pass, fail, not run | Criterion, command, exit code, evidence file, commit | Row header is criterion id |
| ActionCard | message, redirect, stop request; queued, delivered, acknowledged, failed | Lead or human control action, links to worker and audit line | `article` with heading; status in live region |
| Diff view | unified, file list | Worktree vs base, virtualized; + and − glyphs in success and danger | Glyphs, not colour only |

**Built with the shell** (PR #39): `Kbd` (a key, 18 px, radius 4; `decorative` inside a control,
which then carries `aria-keyshortcuts`) and `ActionBar` (the signature: the selection in words, then
actions with label and keys, primary first; chords bind on the window while it is mounted, and a
chord without a modifier never fires while typing in a field). The ⌘K command palette is wired into
the shell: it lists the other screens with their keys, and each screen adds what it can really run
(`useCommands` in `src/palette.tsx`): Crew adds "Go to worker" for each worker with its status, Home
adds Check again. Runs, tasks, criteria and files join as their screens land. The sidebar's
"Search or run ⌘K" button opens the same palette for the mouse.

`Icon` (`components/ui/base/Icon.tsx`) is the authored set: 16 px grid, 1.5 px stroke, round caps and
joins, `currentColor`, always `aria-hidden` (the text beside it or the control's label names the
thing). A 12 px icon draws a 2 px line on the 16 grid, so it still renders at 1.5 px. It holds only
icons in use: the seven screens, search, chevron (the tree), check, redirect, stop, terminal, file and
branch. Icons sit in muted text and take the text colour on hover and on the current item. The Gallery
shows the set at both sizes.

**Planned, not built yet** (the mockups in `mockups/a2/` are their reference): `Launcher` (the ask
field with inline tokens and @-mentions) and `Banner` (the approval band on `--c-warn-tint`).

### Status vocabulary

| Status | Role colour | Shape | Label |
|---|---|---|---|
| pending | text-muted | ○ empty circle | Pending |
| ready | info | ◌ dashed circle | Ready |
| claimed | info | ◐ half circle | Claimed |
| in_progress | text-secondary; primary in the selected row or the live run | ● filled circle | In progress |
| done | success | ✓ check | Done |
| blocked | warning | ■ square | Blocked |
| question | warning | ◆ diamond | Question |
| failed | danger | ✕ cross | Failed |
| criterion pass | success | ✓ check | Pass |
| criterion fail | danger | ✕ cross | Fail |
| criterion unknown | text-muted | ○ empty circle | Not run |
| needs approval | warning | ▲ triangle | Needs approval |

Status shapes are SVG from one set; the glyphs above describe the shape, not a font. A row that
shows its status as an icon keeps the label for screen readers (`.v-sr`).

### Motion

Plain CSS transitions on `--ease-out` (cubic-bezier(0.23, 1, 0.32, 1)): 120 ms (`--t-fast`) for
hover, press and toggles, 180 ms (`--t-panel`) for panels and sheets. State changes cross-fade
colour only. No motion on streams, terminals or lists: new lines appear, nothing slides. The only
looping motion is the live run's spinner and the caret on the live stream line; both stop under
reduced motion. Any wait over 1 s says what it waits for. `prefers-reduced-motion: reduce` sets
every duration to 0.

## Do's and Don'ts

**Anti-slop (blocking):**
- Don't stack identical cards or put icons in coloured circles.
- Don't use checkmark bullet lists, testimonials, invented stats or fake urgency.
- Don't use decorative or animated gradients, glows, glass, blur, radial orbs or dot grids.
- Don't use ALL-CAPS tracked labels, letter-spacing, or meta strings joined with middle dots.
- Don't mark selection with a coloured side rail; use the accent fill.
- Don't use Unicode symbols or emoji as icons; use the SVG set and the status shapes.
- Do keep azure to focus, the live line and the selected row's live state; status
  colours only for status.
- Do build hierarchy from size, weight and spacing first, then colour.
- Do show keys next to actions (the action bar, the palette, buttons with a shortcut).
- Do write honest empty states; never fake data, sample charts or placeholder rows that look real.
- Don't show fake activity: no pulsing "thinking" dots without a real event, no invented progress bars.
- Don't use "AI sparkle" icons or mahabharat names anywhere in the UI.
- Don't use Inter, Geist or Space Grotesk; system fonts only. No half-pixel type sizes.
- Don't put hover animation on everything or drop shadows on flat content.

**Copy:** sentence case; plain verbs ("Start run", "Approve plan", "Stop worker"); say what happened
with numbers ("Stopped Backend at 14:22", "3 of 7 criteria pass"); no em dashes. Strings live in
one file; log visible wording in `docs/design/changed-strings.md`. Run copy through `humanizer` or
`clarify` before review.

## Verification

**Design-guard test** (`app/ui/tests/design-guard.test.ts`, in CI). Fails on raw hex, `rgb(`/`hsl(`
values, any `letter-spacing` or `tracking-*` class, inline `style` colour properties and named
colours anywhere in `app/ui` outside token files (`theme/` directories). Baseline 0; target 0.

**Contrast test** (`app/ui/tests/contrast.test.ts`). Recomputes every role pair from `tokens.css`
and checks the tables above to within 0.01.

**Screenshots.** The kit Gallery (`#kit`, `?theme=light`) is shot before and after each kit change
at 1440 px, dark and light. Once screens migrate: every MVP screen plus palette, dialogs and each
empty, loading, error and engine-missing state at 1280x800 and 960x600, and a click crawl that
presses every visible control and records crashes, page errors, console errors, outbound requests
and dead buttons; target 0 of each. Shots go to `docs/design/after/`. Then `vidura` checks
accessibility independently.

**Rendered audit (PR #39, dev fixture, 1440x900).** An in-page check measured every visible text
against its real composited background on Home, Crew, History (with the report open), Settings, the
palette, and the empty and core-down states, in both themes: 0 below AA, lowest 4.85:1. It found 0
interactive elements without an accessible name. Tabbing with real key presses: the skip link
("Skip to the screen") is the first stop and lands in the screen's first control; then the sidebar,
the screen in reading order, and the action bar. Every stop shows a ring: the ask and the palette
field, which have no box of their own, ring their frame (2 px). ⌘K moves focus into the palette, and
Esc gives it back to where it was. A run starts from the keyboard alone (skip link, ask, Tab,
repository, ⌘↵). With reduced motion, every transition is 0 s. The window title names the screen.
Not done yet: 960x600, the click crawl and a real screen reader pass.

design-md-lint: last run on the previous palette (`npx @google/design.md lint`, CLI 0.4.0: 0 errors,
0 warnings). Not re-run on this version; the front matter keeps the same shape, and every colour
token is still referenced by a component.

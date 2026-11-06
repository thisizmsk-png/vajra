# M3 kit: K1 and K2 verification notes

- Stack: plain CSS variables (`app/ui/theme/tokens.css`), no Tailwind and no Radix. Modal layers use the native `<dialog>`
  (focus trap, Esc, focus return from the browser). This differs from DESIGN.md's "shadcn-style on Radix" wording on purpose
  (fewer dependencies, smaller supply-chain surface); DESIGN.md should say so.
- Design guard baseline: 0 violations over the kit and screens; its own rules are proven on fixtures (`tests/design-guard.test.ts`).
- Contrast: every pair in the DESIGN.md tables is recomputed from `tokens.css` and compared to 0.01; all 16 ANSI colors per theme >= 4.5.
- Screenshots: `docs/design/after/kit-dark-1280.png`, `kit-light-1280.png` (1280x720 CSS px, the AO browser panel size).
  Not shot: 960x600 (the panel has no viewport control), Playwright (not allowed until the benchmark is done).
- Click crawl of the gallery (`#kit`): 8 controls pressed, 0 console errors, 0 page errors, 0 unmatched controls.

## K3 (desktop kit)
- Tabs, Table, Tooltip, CommandPalette, ResizablePanes, TerminalPane, VirtualList, CrewTree, EvidenceRow, ActionCard, DiffView in `app/ui/components/ui/desktop/*`.
- `ansi.ts` is the NFR-SEC-3 sanitizer: only SGR (16 foreground colors, bold, dim, italic, underline) survives; OSC (incl. OSC 8 links and OSC 52 clipboard), DCS/APC/PM/SOS, all other CSI, two-character escapes, C0/C1 controls and carriage returns are dropped; bidi and zero-width characters are shown as visible escapes; lines are capped at 8,192 characters and escape sequences at 4,096. Tested with PoCs and a 3,000-case fuzz.
- Terminal and diff are virtualized (fixed row height, only visible rows rendered): 100,000 lines render under 60 rows.
- Screenshots `kit-{dark,light}-desktop*-1280.png` (1280x720 CSS px). Not shot: 960x600, Playwright. Click crawl of the gallery: tabs, palette, action buttons, table expand, all matched, no page errors.

# M3 kit: K1 and K2 verification notes

- Stack: plain CSS variables (`app/ui/theme/tokens.css`), no Tailwind and no Radix. Modal layers use the native `<dialog>`
  (focus trap, Esc, focus return from the browser). This differs from DESIGN.md's "shadcn-style on Radix" wording on purpose
  (fewer dependencies, smaller supply-chain surface); DESIGN.md should say so.
- Design guard baseline: 0 violations over the kit and screens; its own rules are proven on fixtures (`tests/design-guard.test.ts`).
- Contrast: every pair in the DESIGN.md tables is recomputed from `tokens.css` and compared to 0.01; all 16 ANSI colors per theme >= 4.5.
- Screenshots: `docs/design/after/kit-dark-1280.png`, `kit-light-1280.png` (1280x720 CSS px, the AO browser panel size).
  Not shot: 960x600 (the panel has no viewport control), Playwright (not allowed until the benchmark is done).
- Click crawl of the gallery (`#kit`): 8 controls pressed, 0 console errors, 0 page errors, 0 unmatched controls.

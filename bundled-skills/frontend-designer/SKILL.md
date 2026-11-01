---
name: frontend-designer
description: Thin frontend design workflow. Use when building or reviewing a web page or UI - pick a design direction first, review it against a UX checklist, then run the gate-web floor checks. Routes to impeccable and hallmark for deeper design work.
---

# Frontend designer

A short routine: decide the look, build it, ask the review questions, run the gate.
It is deliberately thin. For deeper design work use the third-party skills named at
the end; this skill does not bundle or copy them.

## 1. Design direction (before building)

- Pick a point of view for this page: who it is for and what it should feel like.
- Commit to type, color and layout in one or two sentences before writing markup.
  Do not drift to defaults mid-build.
- Defer to `steering/rules/anti-slop.md` for what to avoid (generic fonts,
  default white backgrounds, stock gradients, filler copy).
- If the repo already has a design system or DESIGN.md, follow it instead.

## 2. UX checklist (ask these in review)

- Hierarchy: is the one most important thing on screen obvious in two seconds?
- Action: is there a single clear primary action per view, labelled with a verb?
- Type: at most two typefaces, a readable body size and line length?
- Color and contrast: is text readable on its background, and is color never the only signal?
- Spacing: does spacing follow a consistent scale, with related items grouped?
- States: do empty, loading, error and disabled states exist and read clearly?
- Responsive: does it hold at phone, tablet and desktop widths without sideways scroll?
- Keyboard and focus: can every control be reached, seen focused and used by keyboard?
- Motion: does motion have a purpose, and does it stop under reduced-motion?
- Copy: is the text specific to this product, free of filler and stray em dashes?

## 3. Running the gate

The gate is `scripts/gate-web.mjs` in the Vajra repo. It needs Playwright and axe-core in
the target repo only.

The gate executes the page, scripts included, with network access. Run it only on pages
you trust, never on third-party URLs or unreviewed HTML.

1. In the target repo: `npm i -D -E playwright@1.64.1 axe-core@4.14.0`, then
   `npx playwright install chromium`
2. Copy `scripts/gate-web.mjs` into it (it is a single file).
3. Run: `node gate-web.mjs <url-or-file>`
4. Exit 0 is clean, 1 means a check failed (one line per failed id), 2 means the gate
   could not run.

Check ids: `axe`, `overflow-375`, `overflow-768`, `overflow-1440`, `layout-shift`,
`reduced-motion`, `white-bg`, `banned-font`, `em-dash`.

Proof that a browser launches (exits 0 when Playwright can use its cached Chromium):

```
node -e "const{chromium}=require('playwright');(async()=>{const b=await chromium.launch();const p=await b.newPage();await p.setContent('<title>ok</title>');if(await p.title()!=='ok')process.exit(1);await b.close()})().catch(()=>process.exit(1))"
```

Passing means not broken, not good. The gate cannot judge hierarchy or taste; the
checklist above and a human eye still have to.

## Deeper help (third-party)

Use these only if already installed. Do not search for or install them yourself.

- `impeccable` for design, critique, polish and design-system capture.
- `hallmark` for building new pages that look designed rather than generated.

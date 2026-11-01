#!/usr/bin/env node
// gate-web: floor checks for a rendered web page.
//
// These are FLOOR checks. They prove a page is not broken, not that it is good.
// A pass says nothing about hierarchy, taste or whether the design works; a human
// or a design critique still has to judge that.
//
// Usage:    node gate-web.mjs <url-or-file-path>
// Requires: npm i -D -E playwright@1.64.1 axe-core@4.14.0   (in the target repo; run from its root)
//           Vajra itself does not depend on either. Copy this one file anywhere.
// Trust:    this executes the page with network access and read access to the scripts and
//           stylesheets under the repo. Run it from the repo root, only on pages you trust.
// Exit:     0 clean, 1 at least one check failed, 2 the gate could not run.
//           One line per failed check, naming its id.
//
// Check ids: axe, overflow-375, overflow-768, overflow-1440, layout-shift,
//            reduced-motion, white-bg, banned-font, em-dash
//
// Browser proof, reusing the Chromium cached in ~/Library/Caches/ms-playwright with
// no browser download. Needs a Playwright release that matches a cached revision
// (1.64.1 matches chromium-1248). Exits 0:
//
//   node -e "const{chromium}=require('playwright');(async()=>{const b=await chromium.launch();const p=await b.newPage();await p.setContent('<title>ok</title>');if(await p.title()!=='ok')process.exit(1);await b.close()})().catch(()=>process.exit(1))"

import { existsSync, realpathSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const WIDTHS = [375, 768, 1440];
const SETTLE_MS = 1000;
const TIMEOUT_MS = 120000;
const CLS_LIMIT = 0.1;

function bail(message) {
  console.error(`gate-web: ${message}`);
  process.exit(2);
}

// Resolve from the target repo first, then from wherever this script was copied.
function load(name) {
  for (const base of [process.cwd(), dirname(fileURLToPath(import.meta.url))]) {
    try {
      return createRequire(join(base, 'gate-web.cjs'))(name);
    } catch (err) {
      if (err.code !== 'MODULE_NOT_FOUND') throw err;
    }
  }
  bail(`cannot find ${name}. Run: npm i -D -E playwright@1.64.1 axe-core@4.14.0`);
}

function toUrl(input) {
  if (/^[a-z][a-z0-9+.-]+:/i.test(input)) {
    // Normalized, because the file: containment below keys on the exact scheme.
    const parsed = URL.canParse(input) ? new URL(input) : null;
    if (!parsed || !['http:', 'https:', 'file:'].includes(parsed.protocol)) bail(`unsupported target: ${input}`);
    return parsed.href;
  }
  const path = resolve(input);
  if (!existsSync(path)) bail(`no such file: ${input}`);
  return pathToFileURL(path).href;
}

// A file: page can load any local file as a subresource and read its effects, so
// everything local stays inside the repo or the page's folder, dot paths excluded.
// Returns the real path of an allowed file: URL, otherwise null.
function localAccess(url) {
  const roots = [process.cwd(), dirname(fileURLToPath(url))].map((p) => realpathSync(p));
  return (href) => {
    try {
      const path = realpathSync(fileURLToPath(href));
      const allowed = roots.some(
        (root) =>
          path.startsWith(root + sep) &&
          !path.slice(root.length).split(sep).some((part) => part.startsWith('.')),
      );
      return allowed ? path : null;
    } catch {
      return null;
    }
  };
}

async function checkAxe(page, axeSource) {
  await page.evaluate(axeSource);
  const { violations } = await page.evaluate(() =>
    window.axe.run(document, {
      runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
    }),
  );
  // A page can stub window.axe, so only rule-shaped ids reach the output.
  const name = (v) => (/^[a-z0-9-]{1,40}$/.test(v.id) ? v.id : 'invalid-id');
  if (violations.length) return violations.map((v) => `${name(v)} (${Number(v.nodes.length)})`).join(', ');
}

async function checkReducedMotion(page, context, access) {
  const found = await page.evaluate(() => {
    const moves = (s) =>
      (s.animationName && s.animationName !== 'none') ||
      (s.transitionDuration || '').split(',').some((d) => parseFloat(d) > 0);
    let motion = document.getAnimations().length > 0;
    let rule = false;
    const unreadable = [];
    const walk = (rules) => {
      for (const r of rules) {
        if (r instanceof CSSKeyframesRule || (r.style && moves(r.style))) motion = true;
        if (r instanceof CSSMediaRule && /prefers-reduced-motion/i.test(r.conditionText)) rule = true;
        if (r.styleSheet) readSheet(r.styleSheet);
        if (r.cssRules) walk(r.cssRules);
      }
    };
    const readSheet = (sheet) => {
      try {
        walk(sheet.cssRules);
      } catch {
        if (sheet.href) unreadable.push(sheet.href);
      }
    };
    [...document.styleSheets, ...document.adoptedStyleSheets].forEach(readSheet);
    motion ||= [...document.querySelectorAll('*')].some((el) => moves(getComputedStyle(el)));
    return { motion, rule, unreadable };
  });
  if (!found.motion || found.rule) return;
  // Cross-origin sheets hide their rules from the page, so read their text instead.
  // Every file: page is its own origin, so local sheets land here too. href is
  // page-controlled, so local reads go through the same access check as the page.
  for (const href of found.unreadable) {
    if (href.startsWith('file:')) {
      const path = access && access(href);
      if (!path) continue;
      const text = await readFile(path, 'utf8').catch(() => '');
      if (/prefers-reduced-motion/i.test(text)) return;
      continue;
    }
    const response = await context.request.get(href).catch(() => null);
    if (response && /prefers-reduced-motion/i.test(await response.text())) return;
  }
  return 'page animates but has no prefers-reduced-motion rule';
}

async function checkWhiteBg(page) {
  const white = await page.evaluate(() => {
    const clear = (c) => c === 'transparent' || /^rgba\(.*,\s*0\)$/.test(c);
    for (const el of [document.body, document.documentElement]) {
      const s = getComputedStyle(el);
      if (s.backgroundImage !== 'none') return false;
      if (!clear(s.backgroundColor)) return s.backgroundColor === 'rgb(255, 255, 255)';
    }
    return true; // Nothing painted, so the browser's white canvas shows through.
  });
  if (white) return 'page background is pure white';
}

async function checkBannedFont(page) {
  const fonts = await page.evaluate(() => {
    // Word edges rather than exact names, so "__Inter_a1b2c3" and "Geist Mono" count.
    const banned = /(^|[\s_-])(inter|geist|space[\s_-]?grotesk)([\s_-]|$)/i;
    const seen = new Set();
    for (const el of document.querySelectorAll('body, body *')) {
      const hasText = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
      if (!hasText || !el.getClientRects().length) continue;
      for (const family of getComputedStyle(el).fontFamily.split(',')) {
        const match = family.trim().replace(/^["']|["']$/g, '').match(banned);
        if (match) seen.add(match[2].toLowerCase()); // Never the page's own string.
      }
    }
    return [...seen];
  });
  const known = fonts.filter((font) => /^(inter|geist|space[\s_-]?grotesk)$/.test(font));
  if (known.length) return known.join(', ');
}

async function checkEmDash(page) {
  const count = await page.evaluate(() => document.body.innerText.split('—').length - 1);
  if (count) return `${count} in visible text`;
}

async function main() {
  const input = process.argv[2];
  if (!input) bail('usage: node gate-web.mjs <url-or-file-path>');
  const url = toUrl(input);
  const { chromium } = load('playwright');
  const axeSource = load('axe-core').source;
  setTimeout(() => bail(`timed out after ${TIMEOUT_MS / 1000}s`), TIMEOUT_MS).unref();

  const browser = await chromium
    .launch({ chromiumSandbox: true })
    .catch((err) => bail(`cannot launch Chromium: ${err.message.split('\n')[0]}`));
  const failures = [];
  const run = async (id, check) => {
    try {
      const detail = await check();
      if (detail) failures.push(`${id}: ${detail}`);
    } catch {
      failures.push(`${id}: check crashed`); // The message can hold page text.
    }
  };

  try {
    const context = await browser.newContext();
    const access = url.startsWith('file:') ? localAccess(url) : null;
    if (access) {
      await context.route(
        (u) => u.protocol === 'file:',
        (route) => (access(route.request().url()) ? route.continue() : route.abort()),
      );
    }
    const page = await context.newPage();
    await page.addInitScript(() => {
      window.__gateCls = 0;
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) if (!e.hadRecentInput) window.__gateCls += e.value;
      }).observe({ type: 'layout-shift', buffered: true });
    });

    let worstCls = 0;
    for (const width of WIDTHS) {
      await page.setViewportSize({ width, height: 800 });
      const response = await page
        .goto(url, { waitUntil: 'load' })
        .catch((err) => bail(`cannot load ${url}: ${err.message.split('\n')[0]}`));
      if (response && response.status() >= 400) bail(`${url} returned ${response.status()}`);
      await page.waitForTimeout(SETTLE_MS);
      await run(`overflow-${width}`, async () => {
        const { scroll, client } = await page.evaluate(() => ({
          scroll: Number(document.documentElement.scrollWidth),
          client: Number(document.documentElement.clientWidth),
        }));
        if (scroll > client) return `content is ${scroll}px wide in a ${client}px viewport`;
      });
      await run('layout-shift', async () => {
        worstCls = Math.max(worstCls, await page.evaluate(() => window.__gateCls));
      });
    }
    if (worstCls > CLS_LIMIT) failures.push(`layout-shift: CLS ${worstCls.toFixed(3)} above ${CLS_LIMIT}`);

    await run('axe', () => checkAxe(page, axeSource));
    await run('reduced-motion', () => checkReducedMotion(page, context, access));
    await run('white-bg', () => checkWhiteBg(page));
    await run('banned-font', () => checkBannedFont(page));
    await run('em-dash', () => checkEmDash(page));
  } finally {
    await browser.close();
  }

  // Page text can reach these lines and an agent reads them, so keep them short and plain.
  for (const line of failures) console.log(`FAIL ${line.replace(/[^\x20-\x7e]/g, '?').slice(0, 200)}`);
  if (failures.length) process.exit(1);
  console.log('gate-web: floor checks passed (not broken, which is not the same as good)');
}

await main();

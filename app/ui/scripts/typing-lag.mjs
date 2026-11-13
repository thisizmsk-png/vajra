// Typing lag in a real browser: keystroke to paint, with 5 agents streaming.
//
//   node app/ui/scripts/typing-lag.mjs [--gate] [--keys 120] [--out lag.json]
//
// Serves the built typing-lag page (app/ui/dist-lag, from `vite build -c vite.lag.config.ts`), opens it in the installed
// Chrome, and types into the Director-General composer one key at a time. Per key it records the time from the key's own
// timestamp to the first frame after the update (the keydown handler, React's commit, style, layout and a paint). A second
// pass loads the same page with nothing streaming, as the control. p95 over all keys is compared with the budget.
//
// Chrome's Event Timing "duration" is reported alongside as a cross-check; it is rounded to 8 ms and hides anything under
// 16 ms, so it is not what the gate uses.
import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import { extname, join, normalize, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { pct } from "./pct.mjs";
import { BUDGET_DELTA_P95_MS, BUDGET_P95_MS, verdict } from "./verdict.mjs";

const here = fileURLToPath(new URL(".", import.meta.url));
const DIST = resolve(here, "../dist-lag");
const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2" };


function serve() {
  const server = createServer(async (req, res) => {
    const path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname));
    const file = resolve(join(DIST, path === "/" ? "lag.html" : path));
    if (!file.startsWith(DIST + "/") && file !== DIST) { res.writeHead(403).end(); return; }
    try {
      const body = await readFile(file);
      res.writeHead(200, { "content-type": TYPES[extname(file)] ?? "application/octet-stream" }).end(body);
    } catch { res.writeHead(404).end(); }
  });
  return new Promise((ok) => server.listen(0, "127.0.0.1", () => ok(server)));
}

async function pass(browser, base, query, keys, opts = {}) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  // --throttle N: slow the main thread N times (Chrome's own CPU throttle), to stand in for a slow runner.
  const cdp = opts.throttle > 1 || opts.profile ? await page.context().newCDPSession(page) : null;
  if (opts.throttle > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: opts.throttle });
  if (opts.profile) { await cdp.send("Profiler.enable"); await cdp.send("Profiler.setSamplingInterval", { interval: 500 }); }
  await page.addInitScript(() => {
    window.__keys = [];
    window.__events = [];
    window.__loaf = [];
    // Frames the main thread held for over 50 ms: if the page itself were slow, they would show here.
    try { new PerformanceObserver((l) => l.getEntries().forEach((e) => window.__loaf.push(e.duration))).observe({ type: "long-animation-frame", buffered: true }); } catch {}
    addEventListener("keydown", (e) => {
      const t0 = e.timeStamp;
      // After this key's handlers and React's commit: the next frame, then a task after that frame's paint.
      requestAnimationFrame(() => {
        const ch = new MessageChannel();
        ch.port1.onmessage = () => window.__keys.push(performance.now() - t0);
        ch.port2.postMessage(0);
      });
    }, true);
    new PerformanceObserver((l) => l.getEntries().forEach((e) => { if (e.name === "keydown") window.__events.push(e.duration); }))
      .observe({ type: "event", durationThreshold: 16, buffered: true });
  });
  await page.goto(`${base}/${query}`);
  const box = page.getByRole("textbox").first();
  await box.waitFor();
  await page.waitForTimeout(1500); // let the first reads and the stream settle
  await box.focus();
  if (opts.profile) await cdp.send("Profiler.start");
  for (let i = 0; i < keys; i++) {
    await page.keyboard.press(String.fromCharCode(97 + (i % 26)));
    await page.waitForTimeout(25);
  }
  await page.waitForTimeout(300);
  let profile = null;
  if (opts.profile) profile = (await cdp.send("Profiler.stop")).profile;
  const out = await page.evaluate(() => ({ keys: window.__keys, events: window.__events, loaf: window.__loaf, lines: window.__lag.lines(), value: document.querySelector("textarea")?.value.length ?? 0, log: window.__lag.runLog ? window.__lag.log() : null }));
  await page.close();
  return { ...out, profile };
}

/** Self time per function from a CDP CPU profile, as {name, file:line, ms}, biggest first. */
export function topSelfTime(profile, n = 12) {
  if (!profile) return [];
  const byId = new Map(profile.nodes.map((x) => [x.id, x]));
  const self = new Map();
  profile.samples.forEach((id, i) => self.set(id, (self.get(id) ?? 0) + (profile.timeDeltas[i] ?? 0)));
  const agg = new Map();
  for (const [id, us] of self) {
    const cf = byId.get(id).callFrame;
    const key = `${cf.functionName || "(anonymous)"} ${cf.url.split("/").pop()}:${cf.lineNumber + 1}`;
    agg.set(key, (agg.get(key) ?? 0) + us);
  }
  return [...agg].map(([name, us]) => ({ name, ms: +(us / 1000).toFixed(1) })).sort((a, b) => b.ms - a.ms).slice(0, n);
}

const summary = (r, keys) => ({
  keystrokes: keys, painted: r.keys.length, typed: r.value, lines_at_end: r.lines,
  p50_ms: +pct(r.keys, 50).toFixed(1), p95_ms: +pct(r.keys, 95).toFixed(1), max_ms: +Math.max(...r.keys).toFixed(1),
  long_frames_over_50ms: r.loaf.length,
  event_timing_over_16ms: r.events.length, event_timing_p95_ms: r.events.length ? pct(r.events, 95) : "all under 16",
});

async function main() {
  const args = process.argv.slice(2);
  const gate = args.includes("--gate");
  const val = (k, d) => (args.includes(k) ? args[args.indexOf(k) + 1] : d);
  const keys = Number(val("--keys", 120));
  const server = await serve();
  const base = `http://127.0.0.1:${server.address().port}`;
  // --software: no GPU, like a CI VM. --throttle N: main thread N times slower. --profile: top self-time functions, streaming pass.
  const software = args.includes("--software");
  const opts = { throttle: Number(val("--throttle", 1)), profile: args.includes("--profile") };
  const browser = await chromium.launch({ channel: "chrome", args: ["--disable-extensions", ...(software ? ["--disable-gpu", "--use-gl=disabled", "--disable-gpu-compositing"] : [])] });
  try {
    const version = browser.version();
    await pass(browser, base, "", 20, opts); // one warm-up pass, thrown away
    const streaming = await pass(browser, base, "", keys, opts);
    const idle = await pass(browser, base, "?idle=1", keys, opts);
    // The same streaming page with a core run's live log beside the composer (W2): reported, not gated.
    const withLog = await pass(browser, base, "?runlog=1", keys, opts);
    const result = { throttle: opts.throttle, software, top_self_time_streaming: opts.profile ? topSelfTime(streaming.profile) : undefined,
      top_self_time_idle: opts.profile ? topSelfTime(idle.profile) : undefined, schema: "vajra.typing_lag/1", browser: version, os: process.platform, budget_p95_ms: BUDGET_P95_MS, agents: 5,
      streaming: summary(streaming, keys), control_idle: summary(idle, keys),
      streaming_with_run_log: { ...summary(withLog, keys), run_log: withLog.log }, keys_ms: streaming.keys.map((x) => +x.toFixed(1)) };
    const out = val("--out", null);
    if (out) await writeFile(out, JSON.stringify(result, null, 1));
    console.log(JSON.stringify({ ...result, keys_ms: undefined }, null, 1));
    const v = verdict({ keystrokes: keys, painted: streaming.keys.length, typed: streaming.value, p95_ms: result.streaming.p95_ms }, result.control_idle, { absolute: args.includes("--absolute") });
    console.log(`typing lag: p95 ${result.streaming.p95_ms} ms streaming, ${result.control_idle.p95_ms} ms idle control, streaming adds ${v.delta_p95_ms} ms (budget ${BUDGET_DELTA_P95_MS} ms)`);
    const rl = result.streaming_with_run_log;
    console.log(`with the live run log: p95 ${rl.p95_ms} ms (${(rl.p95_ms - result.streaming.p95_ms).toFixed(1)} ms over streaming alone); ${rl.run_log?.entries ?? 0} entries in ${rl.run_log?.notices ?? 0} notices, ${rl.run_log?.calls ?? 0} run_log_since call(s) (report only)`);
    if (!v.ok) console.log(`typing lag ${gate ? "GATE FAILED" : "over budget (report only)"}: ${v.reasons.join("; ")}`);
    if (gate && !v.ok) process.exitCode = 1;
  } finally {
    await browser.close();
    server.close();
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();

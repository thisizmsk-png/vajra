"""A local rig that measures Vajra Desktop the way the public ADE benchmark does, on this machine.

  ade_bench.py window  APP_BINARY OUT_JSON   cold start and relaunch (launch to window ready)
  ade_bench.py weight  APP_BINARY OUT_JSON   at rest (footprint beside RSS), idle CPU, and weight with the load fixture
  ade_bench.py tokens  DATA_DIR   OUT_JSON   token receipts of the finished turns in a control ledger ("hello", real prompt)
  ade_bench.py report  IN_JSON... OUT_MD     one table from the JSON files above

Method (the owner's memory-gate method): 1 warm-up launch that is thrown away, then the median of 5. The runner and its
flavour (macOS build, WebKit build, helper types) are recorded with every result, so numbers are only compared on the same one.

What is and is not measured here:
- "Window ready" is the moment the app's WebContent helper process exists and the app is still alive. It is a stand-in for
  "typing accepted", which needs UI automation; the report says so.
- Weight is `footprint` over the app's process group (the app, its WebKit helpers and its children), reported beside summed RSS.
- New thread, first word, long thread and per-thread weight need a driven UI or a live agent. The rig lists them as `not_measured`
  with the reason instead of printing a number it did not take.
- Typing lag with 5 agents streaming is gated in the UI test job (app/ui/tests/typingLag.test.tsx); this rig does not repeat it.
"""
import json, os, re, statistics, subprocess, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "ci"))
import measure  # noqa: E402  (the memory gate's launch, sample and runner identity, reused unchanged)

WARMUPS = 1
RUNS = 5
NOT_MEASURED = {
    "new_thread_ms": "needs a driven UI",
    "first_word_ms": "needs a live agent turn",
    "long_thread_ms": "needs a driven UI with a 2,000-entry run",
    "per_thread_mib": "needs 5 live threads",
}


def median_of_runs(values, warmups=WARMUPS):
    """Drop the warm-up launches, then take the median. Returns (median, kept values)."""
    kept = [v for v in values[warmups:] if v is not None]
    if not kept:
        return None, []
    return statistics.median(kept), kept


def group_pids(proc, before):
    rows = measure.ps()
    return sorted(measure.descendants(rows, proc.pid) | (measure.webkit_pids(rows) - before)), rows


def cpu_percent(pids, seconds):
    """Mean CPU % of `pids` over `seconds`, from ps's cputime deltas (not ps's decaying %cpu)."""
    def cpu():
        out = subprocess.run(["ps", "-o", "time=", "-p", ",".join(str(p) for p in pids)], capture_output=True, text=True).stdout
        total = 0.0
        for line in out.split():
            parts = [float(x) for x in re.split(r"[:]", line)]
            total += sum(v * 60 ** i for i, v in enumerate(reversed(parts)))
        return total
    a = cpu()
    time.sleep(seconds)
    return round((cpu() - a) / seconds * 100, 2)


def window_ready(proc, before, deadline_s=30):
    """Seconds from launch until the app has a WebContent helper and is still alive, or None past the deadline."""
    t0 = time.time()
    while time.time() - t0 < deadline_s:
        rows = measure.ps()
        if proc.pid not in rows:
            return None
        if any(measure.helper_type(rows[p][2]) == "WebContent" for p in measure.webkit_pids(rows) - before):
            return round(time.time() - t0, 3)
        time.sleep(0.02)
    return None


def window_scenario(binary, out):
    cold = []
    for n in range(WARMUPS + RUNS):
        proc, before = measure.launch(binary)  # a fresh home each time, so every launch is cold
        cold.append(window_ready(proc, before))
        measure.stop(proc)
        time.sleep(3)
    ident = measure.runner_identity()
    cm, ck = median_of_runs(cold)
    json.dump({"scenario": "window", "runner": ident, "method": f"{WARMUPS} warm-up, median of {RUNS}", "proxy": "window ready, not typing accepted",
               "cold_start_s": cm, "cold_start_runs_s": ck, "relaunch_s": None,
               "not_measured": {"relaunch_s": "the launcher gives each launch a fresh home; relaunch on a kept home is not wired yet"},
               "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, open(out, "w"), indent=1)


def weight_scenario(binary, out):
    fp, rss, cpu = [], [], []
    for n in range(WARMUPS + RUNS):
        proc, before = measure.launch(binary)
        time.sleep(25)  # settle, as the memory gate does
        pids, _ = group_pids(proc, before)
        s = measure.sample(proc, before)
        rss.append(s["total_mib"])
        fp.append(measure.footprint_mib(proc, before))
        cpu.append(cpu_percent(pids, 10))
        measure.stop(proc)
        time.sleep(3)
    ident = measure.runner_identity()
    f, fk = median_of_runs(fp)
    r, rk = median_of_runs(rss)
    c, ck = median_of_runs(cpu)
    json.dump({"scenario": "weight", "runner": ident, "method": f"{WARMUPS} warm-up, median of {RUNS}",
               "at_rest_footprint_mib": f, "at_rest_footprint_runs_mib": fk, "at_rest_rss_mib": r, "at_rest_rss_runs_mib": rk,
               "idle_cpu_pct": c, "idle_cpu_runs_pct": ck, "not_measured": NOT_MEASURED,
               "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, open(out, "w"), indent=1)


def read_tokens(data_dir):
    """Token receipts of finished turns in `<data_dir>/control/ledger.jsonl`, oldest first. Lines without one are skipped."""
    path = os.path.join(data_dir, "control", "ledger.jsonl")
    turns = []
    if not os.path.exists(path):
        return turns
    for line in open(path, encoding="utf-8", errors="replace"):
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        t = rec.get("tokens") if isinstance(rec, dict) else None
        if isinstance(t, dict) and t.get("schema") == "vajra.token_receipt/1":
            turns.append({"ask_id": rec.get("action_id") or rec.get("id"), **{k: t.get(k, 0) for k in ("fresh", "cache_write", "cache_read", "output", "side", "total")}})
    return turns


def tokens_scenario(data_dir, out):
    turns = read_tokens(data_dir)
    json.dump({"scenario": "tokens", "turns": turns, "hello": turns[0] if turns else None, "real_prompt": turns[1] if len(turns) > 1 else None,
               "note": "the first finished turn is the hello, the second the real prompt; run them in that order on a fresh data dir",
               "created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}, open(out, "w"), indent=1)


def render_report(results):
    """One markdown table from the scenario JSON files; an unmeasured column says why."""
    lines = ["| metric | value | note |", "|---|---|---|"]
    for r in results:
        if r.get("scenario") == "window":
            lines.append(f"| cold start (s) | {r['cold_start_s']} | {r['proxy']}; {r['method']} |")
        elif r.get("scenario") == "weight":
            lines.append(f"| at rest, footprint (MiB) | {r['at_rest_footprint_mib']} | beside RSS {r['at_rest_rss_mib']}; {r['method']} |")
            lines.append(f"| idle CPU (%) | {r['idle_cpu_pct']} | {r['method']} |")
        elif r.get("scenario") == "tokens":
            for name in ("hello", "real_prompt"):
                t = r.get(name)
                lines.append(f"| tokens, {name.replace('_', ' ')} | {t['total'] if t else 'none'} | fresh/cache write/cache read/output: "
                             + (f"{t['fresh']}/{t['cache_write']}/{t['cache_read']}/{t['output']}" if t else "no finished turn") + " |")
        for k, why in (r.get("not_measured") or {}).items():
            lines.append(f"| {k} | not measured | {why} |")
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    mode, *rest = sys.argv[1:]
    if mode == "window":
        window_scenario(*rest)
    elif mode == "weight":
        weight_scenario(*rest)
    elif mode == "tokens":
        tokens_scenario(*rest)
    elif mode == "report":
        *ins, out = rest
        open(out, "w").write(render_report([json.load(open(p)) for p in ins]))
    else:
        raise SystemExit(__doc__)

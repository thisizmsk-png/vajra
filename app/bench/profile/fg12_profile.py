#!/usr/bin/env python3
"""FG-12 S12.1: resource profile of Vajra Desktop by component, as the reference for every later change.

usage:
  fg12_profile.py bin  APP_BINARY            OUT_JSON SECONDS   # the shipping binary and its WebKit helpers (idle, no run)
  fg12_profile.py core HARNESS_BIN SCENARIO  OUT_JSON SECONDS   # the real core without a window, one scenario
                                                           # (idle run1 run4 term_flood term_flood_unacked terms8)
  core mode needs FG12_FIXTURE (a transcripts `subagents` dir, e.g. app/core/tests/fixtures/transcripts/session/subagents).

Components, each as RSS (MiB) and CPU (% of one core): the app process; each WebKit helper by type (bin mode); each run's
agent process tree (its roots come from the harness, never guessed by name); each terminal's process tree (the app's other
direct children). The method is `ci/measure.py`'s (ps RSS; helpers found by diffing com.apple.WebKit.* pids before and
after launch, and kept only if macOS names this app as the process responsible for them) plus CPU and a visibility guard (load average, frontmost app, helper count per sample).

Focus: the shipping app is started with `open -g -j` (not activated, window hidden) and quit by pid when the run ends, so
it never covers the human's screen; the frontmost app is recorded with every sample.

Bounded and cancellable: SECONDS is clamped to MAX_SECONDS and samples to MAX_SAMPLES; SIGINT/SIGTERM stop the loop,
kill only the launch's own process group and still write what was sampled (`cancelled: true`). Nothing stays running
afterwards, and every launch uses a temp HOME and data dir (never the real ones).
"""
import json, os, re, shutil, signal, statistics, subprocess, sys, tempfile, time

KB = 1024.0
LOAD_MAX = 6.0
SETTLE_S = 25
STEP_S = 2
MAX_SECONDS = 600
MAX_SAMPLES = 300
CANCEL = {"flag": False}


def ps():
    out = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,time=,comm="], capture_output=True, text=True).stdout
    rows = {}
    for line in out.splitlines():
        p = line.split(None, 4)
        if len(p) == 5 and p[0].isdigit():
            rows[int(p[0])] = (int(p[1]), int(p[2]), cpu_s(p[3]), p[4])
    return rows


def cpu_s(t):
    return sum(float(x) * m for x, m in zip(reversed(t.replace("-", ":").split(":")), (1, 60, 3600, 86400)))


def tree(rows, root):
    seen, stack = {root}, [root]
    while stack:
        cur = stack.pop()
        for pid, r in rows.items():
            if r[0] == cur and pid not in seen:
                seen.add(pid)
                stack.append(pid)
    return seen


def webkit(rows):
    return {pid for pid, r in rows.items() if "com.apple.WebKit." in r[3]}


def helper_type(comm):
    m = re.search(r"com\.apple\.WebKit\.(\w+)", comm)
    return m.group(1) if m else "Unknown"


def responsible(pid):
    """The pid macOS holds responsible for `pid` (an XPC helper is charged to the app it serves), or None if unknown."""
    try:
        import ctypes
        f = ctypes.CDLL(None).responsibility_get_pid_responsible_for_pid
        f.argtypes, f.restype = [ctypes.c_int], ctypes.c_int
        r = f(pid)
        return r if r > 0 else None
    except (OSError, AttributeError):
        return None


def classify(rows, app_pid, run_roots, before=frozenset(), owner=responsible):
    """Pure: split the process table into the components. Returns rss MiB and cpu seconds per component.

    A WebKit helper is counted only if it is new since before launch AND `owner(pid)` says it belongs to this app, so a
    helper another app starts during sampling is never added to Vajra. Helpers that are new but not provably ours are
    reported as `comp["unattributed"]` (a count), never summed."""
    part = lambda pids: (sum(rows[p][1] for p in pids) / KB, sum(rows[p][2] for p in pids))
    runs = [tree(rows, r) for r in run_roots if r in rows]
    in_runs = set().union(*runs) if runs else set()
    kids = sorted(p for p, r in rows.items() if r[0] == app_pid and p not in in_runs)
    terminals = [tree(rows, k) for k in kids]
    helpers = {}
    other = 0
    for pid in webkit(rows) - set(before):
        if owner(pid) == app_pid:
            helpers.setdefault(helper_type(rows[pid][3]), set()).add(pid)
        else:
            other += 1
    comp = {"app": part({app_pid}), "runs": [part(t) for t in runs], "terminals": [part(t) for t in terminals],
            "webkit": {k: part(v) for k, v in sorted(helpers.items())}, "unattributed": other}
    total = comp["app"][0] + sum(r for r, _ in comp["runs"]) + sum(r for r, _ in comp["terminals"]) + sum(r for r, _ in comp["webkit"].values())
    return comp, round(total, 1), sum(len(v) for v in helpers.values())


def frontmost():
    try:
        asn = subprocess.run(["lsappinfo", "front"], capture_output=True, text=True, timeout=3).stdout.strip()
        name = subprocess.run(["lsappinfo", "info", "-only", "name", asn], capture_output=True, text=True, timeout=3).stdout
        return re.sub(r'.*"LSDisplayName"="?([^"\n]*)"?.*', r"\1", name.strip().replace("\n", " "))
    except Exception:
        return "?"


def summarize(samples):
    """Pure: medians over the clean samples (fewest helpers seen, load at most LOAD_MAX); all samples if none are clean."""
    if not samples:
        return {"samples": 0}
    lo = min(s["helpers"] for s in samples)
    clean = [s for s in samples if s["helpers"] == lo and s["load_1m"] <= LOAD_MAX]
    use = clean or samples
    med = lambda xs: round(statistics.median(xs), 1)
    first, last = use[0], use[-1]
    wall = (last["t"] - first["t"]) or 1.0
    pct = lambda a, b: round(100.0 * (b - a) / wall, 2)

    def series(get):
        return med([get(s) for s in use])

    out = {"samples": len(samples), "clean_samples": len(clean), "helpers_min": lo,
           "rss_mib": {"app": series(lambda s: s["comp"]["app"][0]),
                       "webkit": {k: series(lambda s, k=k: s["comp"]["webkit"].get(k, [0])[0]) for k in sorted({k for s in use for k in s["comp"]["webkit"]})},
                       "runs": [series(lambda s, i=i: s["comp"]["runs"][i][0] if i < len(s["comp"]["runs"]) else 0) for i in range(max(len(s["comp"]["runs"]) for s in use))],
                       "terminals": [series(lambda s, i=i: s["comp"]["terminals"][i][0] if i < len(s["comp"]["terminals"]) else 0) for i in range(max(len(s["comp"]["terminals"]) for s in use))],
                       "total": series(lambda s: s["total_mib"])},
           "total_min_max": [min(s["total_mib"] for s in use), max(s["total_mib"] for s in use)],
           "cpu_percent_of_one_core": {"app": pct(first["comp"]["app"][1], last["comp"]["app"][1]),
                                       "all": pct(first["cpu_all"], last["cpu_all"])},
           "load_1m_max": max(s["load_1m"] for s in samples), "front_app_seen": sorted({s["front"] for s in samples})}
    return out


def sample(app_pid, run_roots, before):
    rows = ps()
    if app_pid not in rows:
        return None
    comp, total, helpers = classify(rows, app_pid, run_roots, before)
    cpu_all = comp["app"][1] + sum(c for _, c in comp["runs"]) + sum(c for _, c in comp["terminals"]) + sum(c for _, c in comp["webkit"].values())
    return {"t": time.time(), "comp": comp, "total_mib": total, "helpers": helpers, "cpu_all": cpu_all,
            "load_1m": round(os.getloadavg()[0], 1), "front": frontmost()}


def start_micro(pid):
    """When the process started, in microseconds (the same `proc_pidinfo` call the core's `procid` uses), or None if it is
    gone or cannot be read. `ps lstart` has a one-second resolution, which a reused pid can fall inside."""
    import ctypes

    class BsdInfo(ctypes.Structure):  # struct proc_bsdinfo, PROC_PIDTBSDINFO
        _fields_ = [("flags", ctypes.c_uint32), ("status", ctypes.c_uint32), ("xstatus", ctypes.c_uint32), ("pid", ctypes.c_uint32),
                    ("ppid", ctypes.c_uint32), ("uid", ctypes.c_uint32), ("gid", ctypes.c_uint32), ("ruid", ctypes.c_uint32),
                    ("rgid", ctypes.c_uint32), ("svuid", ctypes.c_uint32), ("svgid", ctypes.c_uint32), ("rfu_1", ctypes.c_uint32),
                    ("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32), ("nfiles", ctypes.c_uint32), ("pgid", ctypes.c_uint32),
                    ("pjobc", ctypes.c_uint32), ("e_tdev", ctypes.c_uint32), ("e_tpgid", ctypes.c_uint32), ("nice", ctypes.c_int32),
                    ("start_tvsec", ctypes.c_uint64), ("start_tvusec", ctypes.c_uint64)]

    if sys.platform != "darwin" or pid <= 1:
        return None
    try:
        lib = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        info = BsdInfo()
        got = lib.proc_pidinfo(int(pid), 3, 0, ctypes.byref(info), ctypes.sizeof(info))
        return info.start_tvsec * 1_000_000 + info.start_tvusec if got == ctypes.sizeof(info) else None
    except Exception:
        return None


def identity(pid):
    """(start time in microseconds, command) of the process, or None. Both must still match before a signal."""
    micro = start_micro(pid)
    if micro is None:
        return None
    comm = subprocess.run(["ps", "-o", "comm=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return (micro, comm) if comm else None


def safe_identity(pid):
    """identity(), or None if it could not be read (an app whose identity is unknown is never signalled)."""
    try:
        return identity(pid)
    except Exception:
        return None


def snapshot_tree(pid):
    """Every descendant of `pid` with its identity, taken while the tree still hangs together (once the parent dies they
    are reparented to launchd and can no longer be told from anyone else's)."""
    rows = ps()
    return {p: identity(p) for p in tree(rows, pid) - {pid}} if pid in rows else {}


def stop(p):
    """End the launch: SIGTERM to the harness itself so it runs its own cleanup (shuts its terminals down), SIGKILL of its
    group if it does not exit, then SIGKILL of any snapshotted descendant that is still the same process. Terminal shells
    are their own sessions, which a group kill never reaches. Only processes this launch started are touched."""
    snap = snapshot_tree(p.pid)
    for sig, wait in ((signal.SIGTERM, 12), (signal.SIGKILL, 8)):
        try:
            os.kill(p.pid, sig) if sig == signal.SIGTERM else os.killpg(p.pid, sig)
            p.wait(timeout=wait)
            break
        except ProcessLookupError:
            break
        except Exception:
            pass
    for pid, ident in snap.items():
        # The same pid, the same start time to the microsecond and the same command: ours. Checked again right before
        # the signal; a process with no identity, or a different one, is skipped.
        if ident is not None and identity(pid) == ident:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass


def started_by_us(pid, env_extra):
    """True when this process was started with our isolated data dir in its environment (`ps eww` shows a process's own
    environment), so a same-named app the human started meanwhile is never ended."""
    mark = env_extra.get("VAJRA_DESKTOP_DATA")
    if not mark:
        return False
    out = subprocess.run(["ps", "eww", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
    return f"VAJRA_DESKTOP_DATA={mark}" in out


OWN = {}  # pid -> identity() taken when launch_hidden found the app; stop_pid signals only while it still matches


def launch_hidden(binary, env_extra, wait=20):
    """Start the .app without activating it and with its window hidden (`open -g -j`), so a bench run never takes focus or
    puts a window in front of the human. Returns the app's pid; it is not our child, so `stop_pid` ends exactly that pid. If the
    pid cannot be found in `wait` seconds, an instance this launch started is ended before the error goes on."""
    app = os.path.abspath(binary)
    while app and not app.endswith(".app"):
        parent = os.path.dirname(app)
        if parent == app:
            raise SystemExit("not inside an .app bundle: " + binary)
        app = parent
    exe = os.path.abspath(binary)
    have = {pid for pid, r in ps().items() if r[3] == exe}
    args = ["open", "-g", "-j", "-n"] + [x for k, v in env_extra.items() for x in ("--env", f"{k}={v}")] + [app]
    subprocess.run(args, check=True, timeout=20)
    end = time.time() + wait
    # A new process of the same binary is not necessarily ours (the human may start the app while we wait): only one whose
    # environment holds the isolated data dir given to this `open` counts, both to return it and to end it.
    def ours():
        return sorted(pid for pid in {pid for pid, r in ps().items() if r[3] == exe} - have if started_by_us(pid, env_extra))

    while True:
        mine = ours()
        if mine:
            OWN[mine[0]] = safe_identity(mine[0])
            return mine[0]
        if time.time() >= end:
            break
        time.sleep(0.25)
    # Never found in time: one more look; anything of ours that is running now must not be left behind.
    for pid in ours():
        OWN[pid] = safe_identity(pid)
        stop_pid(pid)
    raise SystemExit("the app did not start")


def signal_if_same(pid, ident, sig):
    """Send `sig` to `pid` only if it is still the process identified by `ident` (start time to the microsecond, and command).
    A pid that was reused, or whose identity was never taken, is not signalled. Returns True if the signal was sent."""
    if ident is None or identity(pid) != ident:
        print(f"warning: pid {pid} is not the app this run started (gone, reused or unverified): not signalling it", file=sys.stderr)
        return False
    try:
        os.kill(pid, sig)
    except ProcessLookupError:
        return False
    return True


def stop_pid(pid, ident=None):
    """Quit exactly this app pid (SIGTERM, then SIGKILL); never a match by name. The pid's identity is re-checked right
    before EACH signal, so an app that exited (its pid possibly reused) is never signalled again."""
    ident = ident if ident is not None else OWN.get(pid)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        if not signal_if_same(pid, ident, sig):
            return
        end = time.time() + 8
        while time.time() < end:
            if pid not in ps() or identity(pid) != ident:
                return
            time.sleep(0.2)


def on_cancel(*_):
    CANCEL["flag"] = True


def nap(seconds):
    end = time.time() + seconds
    while time.time() < end and not CANCEL["flag"]:
        time.sleep(min(0.25, max(0.0, end - time.time())))


def main():
    signal.signal(signal.SIGINT, on_cancel)
    signal.signal(signal.SIGTERM, on_cancel)
    mode = sys.argv[1]
    home = tempfile.mkdtemp(prefix="vajra-fg12-home-")
    env = dict(os.environ, HOME=home, VAJRA_DESKTOP_DATA=os.path.join(home, "data"))
    before = webkit(ps())
    p = None
    if mode == "bin":
        binary, out, secs = sys.argv[2], sys.argv[3], int(sys.argv[4])
        scenario, cmd = "idle", None
    else:
        binary, scenario, out, secs = sys.argv[2], sys.argv[3], sys.argv[4], int(sys.argv[5])
        fx = os.environ["FG12_FIXTURE"]
        agent = os.environ.get("FG12_AGENT", os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake_agent.py"))
        env["FG12_FIXTURE"] = fx
        cmd = [binary, home, os.path.join(home, "data"), scenario, agent, fx, str(min(secs, MAX_SECONDS) + SETTLE_S + 10)]
    secs = max(1, min(secs, MAX_SECONDS))
    runs, samples = [], []
    pid = None
    try:
        # Launching is inside the guard too: a failure here still removes the temp HOME and ends what was started.
        if mode == "bin":
            pid = launch_hidden(binary, {"HOME": home, "VAJRA_DESKTOP_DATA": os.path.join(home, "data")})
        else:
            p = subprocess.Popen(cmd, env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, start_new_session=True, text=True)
            pid = p.pid
            pid = int(p.stdout.readline().split()[1])
            runs = json.loads(p.stdout.readline().split(" ", 1)[1])
        nap(SETTLE_S)
        end = time.time() + secs
        while time.time() < end and len(samples) < MAX_SAMPLES and not CANCEL["flag"]:
            s = sample(pid, runs, before)
            if s is None:
                break
            samples.append(s)
            nap(STEP_S)
    finally:
        if p is not None:
            stop(p)
        elif pid is not None:
            stop_pid(pid)
        shutil.rmtree(home, ignore_errors=True)
    res = summarize(samples) | {"mode": mode, "scenario": scenario, "seconds": secs, "settle_s": SETTLE_S, "cancelled": CANCEL["flag"]}
    json.dump({"summary": res, "samples": samples}, open(out, "w"), indent=1)
    print(json.dumps(res, indent=1))


if __name__ == "__main__":
    main()

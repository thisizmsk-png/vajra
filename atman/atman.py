#!/usr/bin/env python3
"""Atman v2: Vajra's learning loop.

One file, standard library only, Python 3.8+. Two entry points use it:

  hooks/post-tool-use.sh  ->  atman.py hook       (PostToolUse / PostToolUseFailure)
  scripts/atman.sh        ->  atman.py <command>  (status|review|approve|reject|log|
                                                   pause|resume|propose|observe|dream|verify)

What it does, and nothing more:
  * The hook appends one HMAC-chained line per tool call to
    ~/.claude/vajra/practice-log.jsonl (contract in docs/roadmap/v0.5-work-split.md).
    It stores names, outcomes and a hash of a normalized error. No raw text.
  * `propose` reads the verified practice log and the router's routing-log.jsonl
    and writes proposals to ~/.claude/vajra/proposals/. It never edits config/,
    the skill core, skills or agents.
  * `approve` (human only) writes accepted routes to
    ~/.claude/vajra/learned-routes.json and writes skill/agent changes as patch
    files under ~/.claude/vajra/patches/ for the human to apply with git.
"""

import datetime as _dt
import hashlib
import hmac as _hmac
import json
import os
import re
import subprocess
import sys
import time

# --------------------------------------------------------------------------
# Paths and tunables
# --------------------------------------------------------------------------


def _home():
    return os.environ.get("HOME") or os.path.expanduser("~")


VAJRA_DIR = os.path.join(_home(), ".claude", "vajra")
PRACTICE_LOG = os.path.join(VAJRA_DIR, "practice-log.jsonl")
ROUTING_LOG = os.path.join(VAJRA_DIR, "routing-log.jsonl")
KEY_FILE = os.path.join(VAJRA_DIR, ".hmac-key")
EVENTS_DIR = os.path.join(VAJRA_DIR, "events")
PROPOSALS_DIR = os.path.join(VAJRA_DIR, "proposals")
PATCHES_DIR = os.path.join(VAJRA_DIR, "patches")
LEARNED_ROUTES = os.path.join(VAJRA_DIR, "learned-routes.json")
HISTORY_LOG = os.path.join(VAJRA_DIR, "atman-history.jsonl")
PAUSE_FILE = os.path.join(VAJRA_DIR, "atman.paused")
ANCHOR_FILE = os.path.join(VAJRA_DIR, "practice-log.anchor.json")
ARCHIVE_DIR = os.path.join(VAJRA_DIR, "practice-archive")
LOCK_DIR = os.path.join(VAJRA_DIR, "practice-log.lock")
SKILLS_DIR = os.path.join(_home(), ".claude", "skills")
KB_SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "scripts", "vajra_kb.py")
KB_ID_RE = re.compile(r"^kb-[0-9a-f]{6}$")
AGENTS_DIR = os.path.join(_home(), ".claude", "agents")


def _env_int(name, default):
    try:
        return int(os.environ.get(name, default))
    except ValueError:
        return default


MAX_ENTRIES = _env_int("VAJRA_ATMAN_MAX_ENTRIES", 10000)  # prune threshold
ROUTE_MIN_SUPPORT = 3        # router misses needed before a route is proposed
ROUTE_MIN_PRECISION = 0.8    # share of labeled prompts with the word that used the skill
ROUTE_WINDOW_SECONDS = 30 * 60
# Under "/vajra <ask>" the main session only launches the deliver workflow and
# the skills are called by its agents, often well after the prompt. Agent-made
# Skill calls label the session's routed prompt within this longer window
# (still cut off at the next routed prompt).
AGENT_ROUTE_WINDOW_SECONDS = 2 * 60 * 60
DELTA_MIN_FAILURES = 3       # failures in one cluster before a delta is proposed
DELTA_MIN_SESSIONS = 2       # ...spread over at least this many sessions
DELTA_MIN_AFTER = 20         # attributed calls after approval before judging a bullet
WINDOW_DAYS = 30
MAX_NEW_PROPOSALS = 5        # per propose run
MAX_ROUTING_LINES = 50000

BUILTIN_TOOLS = {
    "Read", "Write", "Edit", "MultiEdit", "Glob", "Grep", "Bash", "Agent", "Task",
    "NotebookEdit", "WebFetch", "WebSearch", "TodoWrite", "Skill", "BashOutput",
    "KillShell", "ExitPlanMode", "AskUserQuestion",
}
STOPWORDS = set("""
a an the and or but if then else for to of in on at by with from into onto over under
is are was were be been being am do does did done have has had having can could will would
shall should may might must this that these those it its i me my we our you your they them
their he she his her not no yes please thanks thank just also very really so too than as
what which who whom whose when where why how all any some each every more most less few
make made let get got use using used want need like help new now here there out up down
vajra run show give tell about after before again same other only own such via per
""".split())

NAME_RE = re.compile(r"[^A-Za-z0-9._:-]")
WORD_RE = re.compile(r"^[a-z0-9][a-z0-9-]{1,31}$")
HEX_RE = re.compile(r"^[0-9a-fA-F]{32,}$")
ID_RE = re.compile(r"^(route|skill|agent)-[0-9a-f]{12}$")


# --------------------------------------------------------------------------
# Small helpers
# --------------------------------------------------------------------------


def now_iso():
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_ts(s):
    try:
        return _dt.datetime.strptime(str(s)[:20], "%Y-%m-%dT%H:%M:%SZ").replace(
            tzinfo=_dt.timezone.utc).timestamp()
    except (ValueError, TypeError):
        return None


def sha256_hex(text):
    if isinstance(text, str):
        text = text.encode("utf-8", "replace")
    return hashlib.sha256(text).hexdigest()


def canon(obj):
    """Compact JSON, identical to `jq -c` for the ASCII-only values we write."""
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False)


def safe_name(value, maxlen=64):
    """Sanitize a skill/agent/tool/session name. Returns None if nothing is left."""
    if value is None:
        return None
    s = NAME_RE.sub("", str(value))
    s = s.lstrip(".-:")
    s = s[:maxlen]
    return s or None


def path_segment(name):
    """A name that is safe as one path segment (no separators, no leading dot)."""
    s = safe_name(name) or "unknown"
    return s.replace(":", "__")


def ensure_dir(path, mode=0o700):
    try:
        os.makedirs(path, mode=mode, exist_ok=True)
    except OSError:
        pass


def atomic_write(path, text, mode=0o600):
    ensure_dir(os.path.dirname(path))
    tmp = "%s.tmp.%d" % (path, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.chmod(tmp, mode)
    os.replace(tmp, path)


def read_json(path, default=None):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def load_key():
    try:
        with open(KEY_FILE, "r", encoding="utf-8") as f:
            k = f.read().strip()
    except OSError:
        return None
    if not HEX_RE.match(k) or len(k) % 2:
        return None
    return bytes.fromhex(k)


def sign(key, entry):
    return _hmac.new(key, canon(entry).encode("utf-8"), hashlib.sha256).hexdigest()


def last_line(path):
    """Return the last non-empty line of a file without reading all of it."""
    try:
        with open(path, "rb") as f:
            f.seek(0, os.SEEK_END)
            size = f.tell()
            if size == 0:
                return None
            block = min(size, 65536)
            f.seek(size - block)
            lines = f.read(block).splitlines()
    except OSError:
        return None
    for raw in reversed(lines):
        if raw.strip():
            return raw.decode("utf-8", "replace")
    return None


class Lock(object):
    """mkdir-based lock shared by the hook, prune and propose. Bounded wait."""

    def __init__(self, path=LOCK_DIR, tries=200, delay=0.01, stale=10.0):
        self.path, self.tries, self.delay, self.stale = path, tries, delay, stale
        self.held = False

    def __enter__(self):
        ensure_dir(os.path.dirname(self.path))
        for _ in range(self.tries):
            try:
                os.mkdir(self.path)
                self.held = True
                return self
            except FileExistsError:
                try:
                    if time.time() - os.stat(self.path).st_mtime > self.stale:
                        os.rmdir(self.path)
                        continue
                except OSError:
                    pass
                time.sleep(self.delay)
            except OSError:
                break
        return self

    def __exit__(self, *exc):
        if self.held:
            try:
                os.rmdir(self.path)
            except OSError:
                pass
        return False


# --------------------------------------------------------------------------
# Hook: PostToolUse / PostToolUseFailure -> practice log + event log
# --------------------------------------------------------------------------

_NORM_RULES = [
    (re.compile(r"(?:[A-Za-z]:)?[/\\][^\s'\"`:,;)]+"), "<path>"),
    (re.compile(r"\b[0-9a-f]{7,}\b", re.I), "<hex>"),
    (re.compile(r"\d+"), "0"),
    (re.compile(r"\s+"), " "),
]


def normalize_error(text):
    """Collapse paths, hashes and numbers so the same failure hashes the same."""
    t = str(text).lower()[:4000]
    for rx, rep in _NORM_RULES:
        t = rx.sub(rep, t)
    return t.strip()[:500]


def _text_of(v):
    if v is None:
        return ""
    if isinstance(v, str):
        return v
    try:
        return canon(v)
    except (TypeError, ValueError):
        return str(v)


def classify(payload):
    """Return (outcome, error_text) from real hook fields (and v0.4 legacy ones)."""
    event = payload.get("hook_event_name") or ""
    resp = payload.get("tool_response")
    err = payload.get("error")
    failed = event == "PostToolUseFailure" or bool(err)
    if isinstance(resp, dict):
        if resp.get("is_error") is True or resp.get("success") is False:
            failed = True
        for k in ("exit_code", "exitCode", "returnCode", "code"):
            v = resp.get(k)
            if isinstance(v, int) and not isinstance(v, bool) and v != 0:
                failed = True
        if failed and not err:
            err = resp.get("error") or resp.get("stderr") or resp.get("message")
    if not failed:
        return "success", None
    return "failure", _text_of(err) or event or "failure"


def _git_sha(cwd):
    if not cwd or not os.path.isdir(cwd):
        cwd = None
    try:
        out = subprocess.run(["git", "rev-parse", "--short", "HEAD"], cwd=cwd,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                             timeout=1.5)
        return out.stdout.decode("ascii", "ignore").strip()[:40]
    except (OSError, subprocess.SubprocessError):
        return ""


def _append_chained(path, build, key):
    """Append build(seq, prev) under the lock, chaining to the last line."""
    with Lock(path + ".lock") as lk:
        if not lk.held:
            return False
        prev, seq = "genesis", 0
        tail = last_line(path)
        if tail:
            try:
                last = json.loads(tail)
                prev = last.get("hmac") or "unsigned"
                seq = int(last.get("seq", -1)) + 1
            except (ValueError, TypeError, AttributeError):
                prev, seq = "corrupt", 0
        entry = build(seq, prev)
        entry["hmac"] = sign(key, entry) if key else None
        with open(path, "a", encoding="utf-8") as f:
            f.write(canon(entry) + "\n")
        try:
            os.chmod(path, 0o600)
        except OSError:
            pass
        return True


def cmd_hook(_args):
    raw = sys.stdin.buffer.read(16 * 1024 * 1024)
    try:
        payload = json.loads(raw.decode("utf-8", "replace"))
    except ValueError:
        return 0
    if not isinstance(payload, dict):
        return 0
    tool = safe_name(payload.get("tool_name") or payload.get("tool"))
    if not tool:
        return 0
    ensure_dir(VAJRA_DIR)
    key = load_key()
    ts = now_iso()
    session = safe_name(payload.get("session_id") or payload.get("sessionId")) or "session"
    event = safe_name(payload.get("hook_event_name")) or "PostToolUse"
    tinput = payload.get("tool_input")
    if tinput is None:
        tinput = payload.get("input")
    if not isinstance(tinput, dict):
        tinput = {}
    outcome, err_text = classify(payload)
    error_sha = sha256_hex(normalize_error(err_text)) if outcome == "failure" else None
    skill = safe_name(tinput.get("skill")) if tool == "Skill" else None
    agent = safe_name(payload.get("agent_type"))
    signal = None
    if tool == "Bash" and re.search(r"vajra-verify-work\.sh", str(tinput.get("command", ""))):
        signal = "verify-work"

    # 1. Event log: every call, per session, for scripts/vajra-replay.sh.
    ensure_dir(EVENTS_DIR)
    ev_path = os.path.join(EVENTS_DIR, path_segment(session) + ".jsonl")
    args_hash = sha256_hex(canon(tinput))
    git_sha = _git_sha(payload.get("cwd"))
    _append_chained(ev_path, lambda seq, prev: {
        "seq": seq, "ts": ts, "tool": tool, "outcome": outcome,
        "argsHash": args_hash, "gitSha": git_sha, "prevHmac": prev}, key)

    # 2. Practice log: the learning signal. (Pause stops proposals, not this.)
    def build(seq, prev):
        e = {"ts": ts, "session_id": session, "event": event, "tool": tool,
             "skill": skill, "agent": agent, "outcome": outcome,
             "error_sha256": error_sha}
        if signal:
            e["signal"] = signal
        e["seq"] = seq
        e["prevHmac"] = prev
        return e

    _append_chained(PRACTICE_LOG, build, key)
    return 0


# --------------------------------------------------------------------------
# Verification: only HMAC-valid, chain-linked entries are learned from
# --------------------------------------------------------------------------


def read_anchor(key):
    a = read_json(ANCHOR_FILE)
    if not isinstance(a, dict) or not key:
        return "genesis", False
    body = {k: a.get(k) for k in ("first_prev", "pruned", "archive", "ts")}
    if a.get("hmac") and _hmac.compare_digest(str(a["hmac"]), sign(key, body)):
        return str(a.get("first_prev") or "genesis"), True
    return "genesis", False


def verified_entries(path=PRACTICE_LOG):
    """Walk the log and keep entries whose HMAC verifies AND that link to the
    last accepted entry. Forged, edited, replayed and reordered lines are
    skipped (and counted); genuine lines after an injected one still link."""
    key = load_key()
    stats = {"lines": 0, "accepted": 0, "rejected": 0, "key": bool(key),
             "anchored": False}
    out = []
    if not key:
        return out, stats
    expect, stats["anchored"] = read_anchor(key)
    try:
        f = open(path, "r", encoding="utf-8", errors="replace")
    except OSError:
        return out, stats
    with f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            stats["lines"] += 1
            try:
                e = json.loads(line)
                h = e.pop("hmac")
                ok = (isinstance(h, str) and e.get("prevHmac") == expect
                      and _hmac.compare_digest(h, sign(key, e)))
            except (ValueError, KeyError, TypeError, AttributeError):
                ok = False
            if not ok:
                stats["rejected"] += 1
                continue
            e["hmac"] = h
            expect = h
            out.append(e)
    stats["accepted"] = len(out)
    return out, stats


def cmd_verify(_args):
    entries, st = verified_entries()
    if not st["key"]:
        print("verify: no valid HMAC key at %s (run scripts/install.sh)" % KEY_FILE)
        return 1
    print("verify: %d lines, %d accepted, %d rejected%s" % (
        st["lines"], st["accepted"], st["rejected"],
        " (anchored after prune)" if st["anchored"] else ""))
    return 0 if st["rejected"] == 0 else 1


# --------------------------------------------------------------------------
# Signals
# --------------------------------------------------------------------------


def recent(entries, days=WINDOW_DAYS):
    cutoff = time.time() - days * 86400
    return [e for e in entries if (parse_ts(e.get("ts")) or 0) >= cutoff]


def load_routing_log():
    rows = []
    try:
        f = open(ROUTING_LOG, "r", encoding="utf-8", errors="replace")
    except OSError:
        return rows
    with f:
        lines = f.readlines()[-MAX_ROUTING_LINES:]
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if not isinstance(r, dict):
            continue
        t = parse_ts(r.get("ts"))
        sess = safe_name(r.get("session_id"))
        words = r.get("words") if isinstance(r.get("words"), list) else []
        words = [w for w in (str(x).lower() for x in words[:20])
                 if WORD_RE.match(w) and w not in STOPWORDS]
        route = r.get("route") if isinstance(r.get("route"), dict) else {}
        if t is None or not sess:
            continue
        router_skill = safe_name(route.get("skill"))
        # Every skill the router suggested (its 2nd and 3rd hints too). Lines
        # written before `suggested` existed carry only the top skill.
        sugg = r.get("suggested") if isinstance(r.get("suggested"), list) else []
        suggested = [x for x in (safe_name(v) for v in sugg[:10]) if x]
        if router_skill and router_skill not in suggested:
            suggested.insert(0, router_skill)
        rows.append({"t": t, "session": sess, "words": sorted(set(words)),
                     "router_skill": router_skill, "suggested": suggested,
                     "prompt_sha256": str(r.get("prompt_sha256", ""))[:64]})
    rows.sort(key=lambda r: r["t"])
    return rows


def skill_calls_by_session(entries):
    """{session: [(t, skill, by_agent)]}: successful Skill calls, main session
    (by_agent False) and agent-scoped ones (by_agent True), sorted by time."""
    calls = {}
    for e in entries:
        if e.get("tool") == "Skill" and e.get("skill") and e.get("outcome") == "success" \
                and e.get("skill") != "vajra":
            t = parse_ts(e.get("ts"))
            if t is not None:
                calls.setdefault(e.get("session_id"), []).append(
                    (t, e["skill"], bool(e.get("agent"))))
    for v in calls.values():
        v.sort()
    return calls


def route_label(r, calls, next_t):
    """The skill that labels routed prompt `r`: the first main-session Skill
    call within ROUTE_WINDOW_SECONDS, else the first agent-made Skill call
    within AGENT_ROUTE_WINDOW_SECONDS. Both stop at the next routed prompt.
    An agent call of any skill the router suggested (its top skill or a 2nd
    or 3rd hint) is not a label: the agent may simply have followed the hint,
    and counting it would let the router reinforce itself."""
    main_end = r["t"] + ROUTE_WINDOW_SECONDS
    agent_end = r["t"] + AGENT_ROUTE_WINDOW_SECONDS
    if next_t is not None:
        main_end = min(main_end, next_t)
        agent_end = min(agent_end, next_t)
    label = next((s for (t, s, by_agent) in calls
                  if not by_agent and r["t"] <= t < main_end), None)
    if label:
        return label
    return next((s for (t, s, by_agent) in calls
                 if by_agent and r["t"] <= t < agent_end and s not in suggested_of(r)), None)


def suggested_of(r):
    """The skills the router suggested for routed prompt `r` (top skill first)."""
    s = list(r.get("suggested") or [])
    if r.get("router_skill") and r["router_skill"] not in s:
        s.insert(0, r["router_skill"])
    return s


def route_stats(entries, rows):
    """For each (word, skill): how often a prompt with the word was followed by
    a successful call of that skill, and how often the router missed it."""
    calls = skill_calls_by_session(entries)
    by_session = {}
    for r in rows:
        by_session.setdefault(r["session"], []).append(r)
    total, hit, miss, samples = {}, {}, {}, {}
    labeled = 0
    for sess, srows in by_session.items():
        sc = calls.get(sess, [])
        for i, r in enumerate(srows):
            next_t = srows[i + 1]["t"] if i + 1 < len(srows) else None
            label = route_label(r, sc, next_t)
            if not label:
                continue
            labeled += 1
            for w in r["words"]:
                total[w] = total.get(w, 0) + 1
                k = (w, label)
                hit[k] = hit.get(k, 0) + 1
                # A skill the router already suggested (at any rank) is not a
                # miss, so it is never learned as a new route.
                if label not in suggested_of(r):
                    miss[k] = miss.get(k, 0) + 1
                    samples.setdefault(k, []).append(r["prompt_sha256"])
    return {"total": total, "hit": hit, "miss": miss, "samples": samples,
            "labeled": labeled}


def attribute(entries):
    """Attribute each call to the skill active in its thread and to its agent.
    Returns {target_key: {"calls": n, "clusters": {ckey: {...}}}}."""
    active = {}
    targets = {}

    def bump(tkey, e):
        t = targets.setdefault(tkey, {"calls": 0, "clusters": {}})
        t["calls"] += 1
        if e.get("outcome") != "failure":
            return
        ckey = "%s|%s|%s" % (e.get("tool"), e.get("signal") or "-",
                             (e.get("error_sha256") or "none")[:16])
        c = t["clusters"].setdefault(ckey, {"tool": e.get("tool"),
                                            "signal": e.get("signal"),
                                            "error_sha256": e.get("error_sha256"),
                                            "failures": 0, "sessions": set(),
                                            "seqs": []})
        c["failures"] += 1
        c["sessions"].add(e.get("session_id"))
        c["seqs"].append(e.get("seq"))

    for e in entries:
        thread = (e.get("session_id"), e.get("agent"))
        if e.get("tool") == "Skill" and e.get("skill"):
            if e.get("outcome") == "success":
                active[thread] = e["skill"]
            bump(("skill", e["skill"]), e)
            continue
        sk = active.get(thread) or active.get((e.get("session_id"), None))
        if sk and sk != "vajra":
            bump(("skill", sk), e)
        if e.get("agent"):
            bump(("agent", e["agent"]), e)
    return targets


# --------------------------------------------------------------------------
# Proposals
# --------------------------------------------------------------------------


def pid(kind, *parts):
    return "%s-%s" % (kind, sha256_hex("|".join([kind] + [str(p) for p in parts]))[:12])


def load_proposals():
    out = {}
    try:
        names = sorted(os.listdir(PROPOSALS_DIR))
    except OSError:
        return out
    for n in names:
        if not n.endswith(".json"):
            continue
        p = read_json(os.path.join(PROPOSALS_DIR, n))
        if isinstance(p, dict) and ID_RE.match(str(p.get("id", ""))) and \
                n == p["id"] + ".json":
            out[p["id"]] = p
    return out


def save_proposal(p):
    atomic_write(os.path.join(PROPOSALS_DIR, p["id"] + ".json"),
                 json.dumps(p, indent=2, sort_keys=True) + "\n")


def load_learned():
    d = read_json(LEARNED_ROUTES, {})
    if not isinstance(d, dict) or not isinstance(d.get("routes"), list):
        d = {"version": 1, "routes": []}
    return d


def bullet_text(kind, target, c, sessions):
    who = "skill %s" % target if kind == "skill" else "agent %s" % target
    if c.get("signal") == "verify-work":
        return ("Before reporting the task done, run scripts/vajra-verify-work.sh and fix "
                "what it reports first. (Atman: verify-work failed %d times in %d sessions "
                "while %s was active.)" % (c["failures"], sessions, who))
    sig = (c.get("error_sha256") or "none")[:8]
    return ("Before calling %s, check its preconditions (paths exist, inputs valid, "
            "command available). (Atman: %d %s calls failed with error signature %s in %d "
            "sessions while %s was active.)" % (c["tool"], c["failures"], c["tool"], sig,
                                                 sessions, who))


def build_proposals(entries, stats):
    existing = load_proposals()
    learned = {(r.get("word"), r.get("skill")) for r in load_learned()["routes"]}
    learned_words = {r.get("word") for r in load_learned()["routes"]}
    integrity = {"accepted_entries": stats["accepted"],
                 "rejected_entries": stats["rejected"]}
    new = []
    win = recent(entries)

    # Routing: words the router missed that consistently precede one skill.
    rs = route_stats(win, load_routing_log())
    for (w, s), m in sorted(rs["miss"].items(), key=lambda kv: -kv[1]):
        h, tot = rs["hit"][(w, s)], rs["total"][w]
        prec = float(h) / tot if tot else 0.0
        if m < ROUTE_MIN_SUPPORT or prec < ROUTE_MIN_PRECISION:
            continue
        if (w, s) in learned or w in learned_words:
            continue
        i = pid("route", w, s)
        if i in existing:
            continue
        new.append({"id": i, "kind": "route", "status": "pending", "created": now_iso(),
                    "word": w, "skill": s, "agent": None,
                    "evidence": {"followed_by_skill": h, "prompts_with_word": tot,
                                 "precision": round(prec, 3), "router_misses": m,
                                 "labeled_prompts": rs["labeled"],
                                 "prompt_sha256": rs["samples"][(w, s)][:5]},
                    "integrity": integrity})

    targets = attribute(win)

    # Delta "add": a recurring failure cluster under one skill or agent.
    for (kind, target), t in sorted(targets.items()):
        for ckey, c in sorted(t["clusters"].items(), key=lambda kv: -kv[1]["failures"]):
            ns = len(c["sessions"])
            if c["failures"] < DELTA_MIN_FAILURES or ns < DELTA_MIN_SESSIONS:
                continue
            i = pid(kind, "add", target, ckey)
            if i in existing:
                continue
            new.append({"id": i, "kind": kind, "op": "add", "status": "pending",
                        "created": now_iso(), "target": target, "cluster": ckey,
                        "bullet": bullet_text(kind, target, c, ns),
                        "evidence": {"failures": c["failures"], "sessions": ns,
                                     "calls": t["calls"],
                                     "rate": round(float(c["failures"]) / t["calls"], 4),
                                     "tool": c["tool"], "signal": c["signal"],
                                     "error_sha256": c["error_sha256"],
                                     "seqs": c["seqs"][-10:]},
                        "integrity": integrity})

    # Delta "retire"/"modify": judge approved bullets on what happened after approval.
    for p in existing.values():
        if p.get("status") != "approved" or p.get("op") != "add":
            continue
        since = parse_ts(p.get("approved_at")) or 0
        after = [e for e in entries if (parse_ts(e.get("ts")) or 0) > since]
        t = attribute(after).get((p["kind"], p["target"]))
        if not t or t["calls"] < DELTA_MIN_AFTER:
            continue
        c = t["clusters"].get(p["cluster"])
        fails = c["failures"] if c else 0
        before = float(p["evidence"].get("rate") or 0)
        rate = float(fails) / t["calls"]
        if rate >= before and before > 0:
            op = "retire"
        elif fails >= DELTA_MIN_FAILURES and rate > before / 2:
            op = "modify"
        else:
            continue
        i = pid(p["kind"], op, p["id"])
        if i in existing:
            continue
        q = {"id": i, "kind": p["kind"], "op": op, "status": "pending",
             "created": now_iso(), "target": p["target"], "cluster": p["cluster"],
             "ref": p["id"],
             "evidence": {"calls_after": t["calls"], "failures_after": fails,
                          "rate_after": round(rate, 4), "rate_before": before},
             "integrity": integrity}
        if op == "modify":
            q["bullet"] = p["bullet"].replace("check its preconditions",
                                              "stop and confirm its preconditions with "
                                              "a read-only check first", 1)
            if q["bullet"] == p["bullet"]:
                q["bullet"] = "Treat this as a hard stop: " + p["bullet"]
        new.append(q)

    return new[:MAX_NEW_PROPOSALS]


def cmd_propose(_args, quiet=False):
    if os.path.exists(PAUSE_FILE):
        print("atman: paused; no proposals (run: atman.sh resume)")
        return 0
    entries, st = verified_entries()
    if not st["key"]:
        print("atman: no valid HMAC key; refusing to learn from an unverifiable log")
        return 1
    if st["rejected"]:
        print("atman: WARNING %d practice-log lines failed verification and were ignored"
              % st["rejected"])
    new = build_proposals(entries, st)
    for p in new:
        save_proposal(p)
        if not quiet:
            print("  [NEW] %s  %s" % (p["id"], describe(p)))
    print("atman: %d new proposal(s) from %d verified entries" % (len(new), st["accepted"]))
    return 0


def describe(p):
    ev = p.get("evidence") if isinstance(p.get("evidence"), dict) else {}

    def num(k):
        v = ev.get(k, 0)
        return v if isinstance(v, (int, float)) and not isinstance(v, bool) else 0
    if p.get("kind") == "route":
        return "route '%s' -> %s  (precision %.2f, %d/%d prompts, router missed %d)" % (
            _clean_bullet(p.get("word")), _clean_bullet(p.get("skill")), num("precision"),
            num("followed_by_skill"), num("prompts_with_word"), num("router_misses"))
    who = "%s %s" % (_clean_bullet(p.get("kind")), _clean_bullet(p.get("target")))
    if p.get("op") == "add":
        return "%s: add bullet  (%d failures, %d sessions)" % (
            who, num("failures"), num("sessions"))
    return "%s: %s bullet of %s  (rate before %.3f, after %.3f over %d calls)" % (
        who, _clean_bullet(p.get("op")), _clean_bullet(p.get("ref")), num("rate_before"),
        num("rate_after"), num("calls_after"))


# --------------------------------------------------------------------------
# Human review: approve / reject
# --------------------------------------------------------------------------


def human_gate(action, ident):
    if os.environ.get("VAJRA_ATMAN_ASSUME_HUMAN") == "1":
        return True
    if not (sys.stdin.isatty() and sys.stdout.isatty()):
        sys.stderr.write(
            "atman: '%s' needs a human at a terminal. Run it yourself:\n"
            "  bash ~/.claude/skills/vajra/scripts/atman.sh %s %s\n" % (action, action, ident))
        return False
    ans = input("%s %s? [y/N] " % (action, ident)).strip().lower()
    return ans in ("y", "yes")


def history(action, p, extra=None):
    rec = {"ts": now_iso(), "action": action, "id": p["id"], "kind": p["kind"],
           "op": p.get("op")}
    if extra:
        rec.update(extra)
    ensure_dir(VAJRA_DIR)
    with open(HISTORY_LOG, "a", encoding="utf-8") as f:
        f.write(canon(rec) + "\n")


def resolve_target(kind, name):
    name = safe_name(name)
    if not name:
        return None
    base = name.split(":")[-1]
    if kind == "agent":
        cands = [os.path.join(AGENTS_DIR, path_segment(base) + ".md")]
    else:
        seg = path_segment(base)
        cands = [os.path.join(SKILLS_DIR, seg, "SKILL.md"),
                 os.path.join(SKILLS_DIR, "vajra", "bundled-skills", seg, "SKILL.md")]
    for c in cands:
        if os.path.isfile(c):
            return c
    return None


def _repo_root(path):
    d = os.path.dirname(path)
    try:
        out = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=d,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=3)
        top = out.stdout.decode("utf-8", "replace").strip()
        if out.returncode == 0 and top:
            return os.path.realpath(top)
    except (OSError, subprocess.SubprocessError):
        pass
    return os.path.realpath(d)


def _clean_bullet(text):
    t = re.sub(r"[\x00-\x1f\x7f<>`]", " ", str(text))
    return re.sub(r"\s+", " ", t).strip()[:400]


def _drop_empty_section(lines, header):
    """Remove the Atman section (and the blank lines add put around it) once it
    has no bullets left, so add followed by retire restores the original file."""
    if header not in lines:
        return
    i = lines.index(header)
    j = i + 1
    while j < len(lines) and not lines[j].strip():
        j += 1
    if j < len(lines) and not lines[j].startswith("## "):
        return
    start = i - 1 if i > 0 and not lines[i - 1].strip() else i
    del lines[start:j]


def make_patch(p, target_file):
    import difflib
    with open(target_file, "r", encoding="utf-8", errors="replace") as f:
        old = f.read()
    lines = old.splitlines(True)
    if lines and not lines[-1].endswith("\n"):
        lines[-1] += "\n"
    if p.get("op") not in ("add", "modify", "retire"):
        return None, "unknown op %r" % p.get("op")
    if p.get("op") != "retire" and not _clean_bullet(p.get("bullet", "")):
        return None, "proposal has no bullet text"
    if p.get("op") != "add" and not ID_RE.match(str(p.get("ref", ""))):
        return None, "proposal has no valid ref"
    marker_id = p["id"] if p.get("op") == "add" else p.get("ref")
    marker = "<!-- atman:%s -->" % marker_id
    new = list(lines)
    if p.get("op") == "add":
        header = "## Learned from practice (Atman)\n"
        bullet = "- %s %s\n" % (_clean_bullet(p["bullet"]), marker)
        if header in new:
            idx = new.index(header) + 1
            while idx < len(new) and new[idx].startswith("- "):
                idx += 1
            new.insert(idx, bullet)
        else:
            new += ["\n", header, "\n", bullet]
    else:
        hits = [i for i, l in enumerate(new) if marker in l]
        if not hits:
            return None, "bullet %s not found in %s (was it applied?)" % (marker, target_file)
        if p["op"] == "retire":
            del new[hits[0]]
            _drop_empty_section(new, "## Learned from practice (Atman)\n")
        else:
            new[hits[0]] = "- %s %s\n" % (_clean_bullet(p["bullet"]), marker)
    root = _repo_root(target_file)
    rel = os.path.relpath(os.path.realpath(target_file), root)
    diff = "".join(difflib.unified_diff(lines, new, "a/" + rel, "b/" + rel))
    if not diff:
        return None, "empty diff"
    pre = ("Atman proposal %s (%s %s, op %s). Review, then apply it yourself:\n"
           "  git -C '%s' apply '%s'\n"
           "Evidence: %s\n\n" % (p["id"], p["kind"], p["target"], p.get("op"), root,
                                os.path.join(PATCHES_DIR, p["id"] + ".patch"),
                                canon(p.get("evidence", {}))))
    return pre + diff, root


def kb_call(args, capture=False):
    """Run scripts/vajra_kb.py (knowledge ledger). stdio is inherited unless
    capture is set, so approve/reject keep the user's TTY."""
    cmd = [sys.executable or "python3", "-I", KB_SCRIPT] + list(args)
    try:
        if capture:
            out = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30)
            return out.returncode, out.stdout.decode("utf-8", "replace")
        return subprocess.call(cmd), ""
    except (OSError, subprocess.SubprocessError):
        return 1, ""


def kb_pending():
    rc, text = kb_call(["pending", "--all", "--json"], capture=True)
    try:
        return json.loads(text).get("pending", []) if rc == 0 else []
    except (ValueError, AttributeError):
        return []


def cmd_approve(args):
    if len(args) == 1 and KB_ID_RE.match(args[0]):
        return kb_call(["approve", args[0]])[0]
    if len(args) != 1 or not ID_RE.match(args[0]):
        print("usage: atman.sh approve <proposal-id>", file=sys.stderr)
        return 2
    props = load_proposals()
    p = props.get(args[0])
    if not p:
        print("atman: no proposal %s" % args[0], file=sys.stderr)
        return 1
    if p.get("status") != "pending":
        print("atman: %s is already %s" % (p["id"], p.get("status")), file=sys.stderr)
        return 1
    print(describe(p))
    if not human_gate("approve", p["id"]):
        return 3
    extra = {}
    if p["kind"] == "route":
        word, skill = str(p.get("word", "")), safe_name(p.get("skill"))
        if not WORD_RE.match(word) or word in STOPWORDS or not skill:
            print("atman: proposal fields failed validation; not approved", file=sys.stderr)
            return 1
        if not resolve_target("skill", skill):
            print("atman: note: skill '%s' was not found under %s" % (skill, SKILLS_DIR))
        d = load_learned()
        d["routes"] = [r for r in d["routes"] if r.get("word") != word]
        d["routes"].append({"word": word, "skill": skill, "agent": None,
                            "precision": p["evidence"].get("precision"),
                            "support": p["evidence"].get("followed_by_skill"),
                            "proposal": p["id"], "approved_at": now_iso()})
        d["version"] = 1
        d["updated"] = now_iso()
        atomic_write(LEARNED_ROUTES, json.dumps(d, indent=2, sort_keys=True) + "\n")
        print("atman: route '%s' -> %s written to %s" % (word, skill, LEARNED_ROUTES))
    else:
        target = resolve_target(p["kind"], p.get("target"))
        if not target:
            print("atman: cannot find the %s file for '%s'; not approved"
                  % (p["kind"], p.get("target")), file=sys.stderr)
            return 1
        text, info = make_patch(p, target)
        if text is None:
            print("atman: %s; not approved" % info, file=sys.stderr)
            return 1
        ensure_dir(PATCHES_DIR)
        patch = os.path.join(PATCHES_DIR, p["id"] + ".patch")
        atomic_write(patch, text)
        extra = {"patch": patch, "target_file": target}
        p.update(extra)
        print("atman: patch written to %s" % patch)
        print("apply it yourself after review:  git -C '%s' apply '%s'" % (info, patch))
    p["status"] = "approved"
    p["approved_at"] = now_iso()
    save_proposal(p)
    history("approve", p, extra)
    return 0


def cmd_reject(args):
    if args and KB_ID_RE.match(args[0]):
        return kb_call(["reject"] + list(args))[0]
    if len(args) < 1 or not ID_RE.match(args[0]):
        print("usage: atman.sh reject <proposal-id> [reason]", file=sys.stderr)
        return 2
    p = load_proposals().get(args[0])
    if not p or p.get("status") != "pending":
        print("atman: no pending proposal %s" % args[0], file=sys.stderr)
        return 1
    if not human_gate("reject", p["id"]):
        return 3
    p["status"] = "rejected"
    p["rejected_at"] = now_iso()
    reason = _clean_bullet(" ".join(args[1:]))[:200]
    if reason:
        p["reason"] = reason
    save_proposal(p)
    history("reject", p, {"reason": reason} if reason else None)
    print("atman: rejected %s" % p["id"])
    return 0


# --------------------------------------------------------------------------
# Read-only views
# --------------------------------------------------------------------------


def cmd_status(_args):
    entries, st = verified_entries()
    props = load_proposals()
    counts = {}
    for p in props.values():
        counts[p.get("status")] = counts.get(p.get("status"), 0) + 1
    win = recent(entries, 7)
    fails = [e for e in win if e.get("outcome") == "failure"]
    print("Atman status")
    print("  state:          %s" % ("PAUSED (observing, not proposing)"
                                    if os.path.exists(PAUSE_FILE) else "observing and proposing"))
    if not st["key"]:
        print("  integrity:      no valid HMAC key (nothing is learned until install creates one)")
    else:
        print("  integrity:      %d verified, %d rejected line(s)%s" % (
            st["accepted"], st["rejected"],
            "  <-- tampering or corruption" if st["rejected"] else ""))
    print("  last 7 days:    %d calls, %d failures" % (len(win), len(fails)))
    flagged = []
    for (kind, target), t in sorted(attribute(win).items()):
        nf = sum(c["failures"] for c in t["clusters"].values())
        if nf >= 2:
            flagged.append("%s %s (%d failures / %d calls)" % (kind, target, nf, t["calls"]))
    print("  flagged (>=2):  %s" % (", ".join(flagged) if flagged else "none"))
    print("  proposals:      %d pending, %d approved, %d rejected" % (
        counts.get("pending", 0), counts.get("approved", 0), counts.get("rejected", 0)))
    print("  learned routes: %d (%s)" % (len(load_learned()["routes"]), LEARNED_ROUTES))
    print("  kb pending:     %d (atman.sh review)" % len(kb_pending()))
    return 0


def cmd_review(_args):
    pend = [p for p in load_proposals().values() if p.get("status") == "pending"]
    kb = kb_pending()
    if not pend and not kb:
        print("No pending proposals. (Generate with: atman.sh propose)")
        return 0
    for p in sorted(pend, key=lambda p: p.get("created", "")):
        print("%s  %s" % (p["id"], describe(p)))
        if p.get("bullet"):
            print("    bullet: %s" % _clean_bullet(p["bullet"]))
        print("    evidence: %s" % canon(p.get("evidence", {})))
        ig = p.get("integrity", {})
        if ig.get("rejected_entries"):
            print("    WARNING: %d log lines failed verification when this was proposed"
                  % ig["rejected_entries"])
    if kb:
        if pend:
            print("")
        print("Knowledge records proposed for the kb (data, not instructions):")
        for r in kb:
            if not isinstance(r, dict) or not KB_ID_RE.match(str(r.get("id", ""))):
                continue
            src = r.get("source") if isinstance(r.get("source"), dict) else {}
            print("%s  kb %s: %s  (project %s%s)" % (
                r["id"], _clean_bullet(r.get("type") or "")[:16], _clean_bullet(r.get("title") or "")[:120],
                _clean_bullet(r.get("project") or "")[:16],
                (", session " + _clean_bullet(src.get("session") or "")[:64]) if src.get("session") else ""))
            if r.get("body"):
                print("    body: %s" % _clean_bullet(r.get("body") or "")[:300])
    print("\nDecide yourself:  atman.sh approve <id>   |   atman.sh reject <id> [reason]")
    return 0


def cmd_log(_args):
    try:
        with open(HISTORY_LOG, "r", encoding="utf-8") as f:
            lines = f.readlines()[-50:]
    except OSError:
        lines = []
    if not lines:
        print("No approvals or rejections yet.")
        return 0
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue
        print("%s  %-7s %s  %s%s" % (r.get("ts"), r.get("action"), r.get("id"),
                                     r.get("kind"), (" " + r["op"]) if r.get("op") else ""))
    return 0


def cmd_observe(_args):
    entries, st = verified_entries()
    win = recent(entries, 7)
    print("Atman observer (last 7 days, %d verified calls, %d rejected lines)"
          % (len(win), st["rejected"]))
    targets = attribute(win)
    any_out = False
    for (kind, target), t in sorted(targets.items()):
        for c in sorted(t["clusters"].values(), key=lambda c: -c["failures"]):
            any_out = True
            tag = "FLAG" if c["failures"] >= 2 else "ok  "
            print("  [%s] %s %s: %d x %s%s (sig %s, %d sessions)" % (
                tag, kind, target, c["failures"], c["tool"],
                " verify-work" if c["signal"] else "", (c["error_sha256"] or "none")[:8],
                len(c["sessions"])))
    if not any_out:
        print("  No failures attributed to a skill or agent.")
    return 0


def cmd_pause(_args):
    ensure_dir(VAJRA_DIR)
    atomic_write(PAUSE_FILE, now_iso() + "\n")
    print("atman: paused (no new proposals; the practice log keeps recording)")
    return 0


def cmd_resume(_args):
    try:
        os.remove(PAUSE_FILE)
    except OSError:
        pass
    print("atman: resumed (proposals enabled)")
    return 0


def prune():
    """Keep the newest MAX_ENTRIES lines in their original order; move the rest
    to practice-archive/ and write a signed anchor so the chain still verifies."""
    key = load_key()
    try:
        with open(PRACTICE_LOG, "r", encoding="utf-8", errors="replace") as f:
            n = sum(1 for _ in f)
    except OSError:
        return 0
    if n <= MAX_ENTRIES or not key:
        return 0
    with Lock(PRACTICE_LOG + ".lock") as lk:
        if not lk.held:
            return 0
        with open(PRACTICE_LOG, "r", encoding="utf-8", errors="replace") as f:
            lines = [l for l in f if l.strip()]
        cut = len(lines) - MAX_ENTRIES
        head, keep = lines[:cut], lines[cut:]
        try:
            first_prev = json.loads(keep[0]).get("prevHmac")
        except ValueError:
            return 0
        ensure_dir(ARCHIVE_DIR)
        stamp = _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        arch = os.path.join(ARCHIVE_DIR, "practice-%s-%d.jsonl" % (stamp, os.getpid()))
        atomic_write(arch, "".join(head))
        body = {"first_prev": first_prev, "pruned": cut,
                "archive": os.path.basename(arch), "ts": now_iso()}
        anchor = dict(body)
        anchor["hmac"] = sign(key, body)
        atomic_write(ANCHOR_FILE, json.dumps(anchor) + "\n")
        atomic_write(PRACTICE_LOG, "".join(keep))
    return cut


def cmd_dream(_args):
    print("Atman dream: prune, then propose (never edits config/, skills or agents)")
    cut = prune()
    if cut:
        print("  pruned %d old entries to %s (chain anchored)" % (cut, ARCHIVE_DIR))
    rc = cmd_propose([])
    print("Review with: atman.sh review")
    return rc


COMMANDS = {
    "hook": cmd_hook, "status": cmd_status, "review": cmd_review,
    "approve": cmd_approve, "reject": cmd_reject, "log": cmd_log,
    "pause": cmd_pause, "resume": cmd_resume, "propose": cmd_propose,
    "observe": cmd_observe, "dream": cmd_dream, "verify": cmd_verify,
}

USAGE = ("usage: atman.sh status | review | approve <id> | reject <id> [reason] | log |\n"
         "                pause | resume | propose | observe | dream | verify")


def main(argv):
    if len(argv) < 2 or argv[1] not in COMMANDS:
        print(USAGE, file=sys.stderr)
        return 2
    return COMMANDS[argv[1]](argv[2:])


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except KeyboardInterrupt:
        sys.exit(130)

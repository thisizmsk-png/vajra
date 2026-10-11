#!/usr/bin/env python3
"""vajra_tasks.py: the task graph of a Vajra campaign (python3 stdlib only).

State: ~/.claude/vajra/campaigns/<slug>/tasks.json. This script is the only thing that changes a
task's status. Every write takes a mkdir lock and replaces the file atomically.

Task fields: id, title, owner_agent, deps[], files[globs], acceptance[], status, attempts, evidence,
branch, worktree (plus claimed_by, report, commit, base_commit, changed, updates).
Status: pending, ready, claimed, in_progress, done, blocked, question, failed. pending and ready are
derived: a pending or ready task is ready exactly when every dependency is done.

Usage (every command takes --campaign <slug> and --json):
  vajra_tasks.py add <id> --title T --owner AGENT [--deps a,b] [--files g1,g2] [--acceptance X]...
                         [--branch B] [--worktree W]
  vajra_tasks.py dep <id> <dep-id> [--remove]
  vajra_tasks.py validate
  vajra_tasks.py ready
  vajra_tasks.py claim <id> <worker> [--stacked]
  vajra_tasks.py report <id> progress|in_progress|done|blocked|question|failed [--summary TEXT]
  vajra_tasks.py close <id> --evidence <verify-work evidence.json> [--commit SHA] [--branch B]
                           [--base-commit SHA] [--changed f1,f2] [--head SHA]
  vajra_tasks.py status <id>
  vajra_tasks.py list
  vajra_tasks.py waves [--max-in-flight N]
  vajra_tasks.py graph

`report <id> done` is only a claim: it records the report and leaves the task in_progress. A task
becomes done only through `close`, which needs a claimed or in_progress task, every dependency done, and
green evidence. Green follows the shared contract (the same check as vajra_kb.py): every command exited 0,
git.treeStable is true, git.head is the current HEAD (or --head), the file is
<repo_root>/data/walkthroughs/<slug>/evidence.json, its hmac verifies with the Vajra key, and not every
command is trivial (true, :, echo, exit 0).
`report` with progress, in_progress or done needs the task claimed, in_progress or ready. A blocked,
question or failed task is retried by claiming it again (its dependencies must be done).
`claim --stacked` also accepts a pending task whose dependencies are done or in_progress with a `done`
report: vajra-deliver builds dependents on their dependencies' unverified commits within one run, and
closes the tasks in dependency order once the integration is verified.

`waves` prints a JSON list of waves (lists of task ids). Tasks are visited in topological order (a
depth-first post-order over deps, in insertion order); each task goes into the first wave after all
of its dependencies' waves that holds no task whose files globs overlap its own. With
--max-in-flight N each wave is then split into consecutive batches of at most N. workflows/
vajra-deliver.js implements the same algorithm in JavaScript; tests/workflows checks they agree.

Exit codes: 0 ok, 1 error (not found, lock, I/O), 2 usage or invalid graph, 3 refused by state
(claim conflict, illegal transition, close without green evidence).
"""
import argparse
import hashlib
import hmac
import json
import os
import re
import subprocess
import sys
import tempfile
import time

SCHEMA = "vajra.tasks/v1"
STATUSES = ("pending", "ready", "claimed", "in_progress", "done", "blocked", "question", "failed")
SLUG_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
AGENT_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
SHA_RE = re.compile(r"^[0-9a-f]{7,40}$")
BRANCH_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$")
# Same rule as SAFE_PATH in workflows/vajra-deliver.js: repo-relative, no ~, no absolute paths.
SAFE_PATH = re.compile(r"^(?!/)(?!~)[A-Za-z0-9._@+\-/*]+$")
GLOB_CH = re.compile(r"[*?\[]")
MAX_TASKS = 500


class Fail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code
        self.msg = msg


def clean(s, n):
    s = "" if s is None else str(s)
    s = re.sub(r"[\x00-\x08\x0b-\x1f\x7f]", " ", s).strip()
    return s[:n]


def one_line(s, n):
    return re.sub(r"\s+", " ", clean(s, 4000))[:n]


# ---------------------------------------------------------------- glob overlap and waves
def norm_path(p):
    x = re.sub(r"^(\./)+", "", p)
    x = re.sub(r"/\*\*?$", "", x)
    x = re.sub(r"/+$", "", x)
    return "" if x == "." else x


def static_prefix(x):
    m = GLOB_CH.search(x)
    return x if not m else x[:m.start()]


def overlaps(a, b):
    x, y = norm_path(a), norm_path(b)
    if x == "" or y == "":
        return True
    if GLOB_CH.search(x) or GLOB_CH.search(y):
        px, py = static_prefix(x), static_prefix(y)
        return px.startswith(py) or py.startswith(px)
    return x == y or y.startswith(x + "/") or x.startswith(y + "/")


def topo(tasks):
    by_id = {t["id"]: t for t in tasks}
    seen, out = set(), []

    def visit(i):
        if i in seen or i not in by_id:
            return
        seen.add(i)
        for d in by_id[i]["deps"]:
            visit(d)
        out.append(i)

    for t in tasks:
        visit(t["id"])
    return out


def compute_waves(tasks, max_in_flight=None):
    by_id = {t["id"]: t for t in tasks}
    wave_of, waves = {}, []
    for i in topo(tasks):
        t = by_id[i]
        w = 0
        for d in t["deps"]:
            if d in wave_of:
                w = max(w, wave_of[d] + 1)
        while w < len(waves) and any(overlaps(f, g) for o in waves[w] for f in t["files"] for g in by_id[o]["files"]):
            w += 1
        if w == len(waves):
            waves.append([])
        waves[w].append(i)
        wave_of[i] = w
    if max_in_flight:
        out = []
        for wv in waves:
            for k in range(0, len(wv), max_in_flight):
                out.append(wv[k:k + max_in_flight])
        return out
    return waves


def find_cycle(tasks):
    by_id = {t["id"]: t for t in tasks}
    state = {}

    def visit(i, path):
        if state.get(i) == 1:
            return path[path.index(i):] + [i]
        if state.get(i) == 2 or i not in by_id:
            return None
        state[i] = 1
        for d in by_id[i]["deps"]:
            c = visit(d, path + [i])
            if c:
                return c
        state[i] = 2
        return None

    for t in tasks:
        c = visit(t["id"], [])
        if c:
            return c
    return None


# ---------------------------------------------------------------- storage
def campaign_dir(slug):
    if not SLUG_RE.match(slug or ""):
        raise Fail(2, "invalid campaign slug %r (lowercase letters, digits and -)" % (slug,))
    return os.path.join(os.path.expanduser("~"), ".claude", "vajra", "campaigns", slug)


class Store:
    def __init__(self, slug, write=False):
        self.slug = slug
        self.dir = campaign_dir(slug)
        self.path = os.path.join(self.dir, "tasks.json")
        self.lock_dir = os.path.join(self.dir, ".tasks.lock")
        self.write = write
        self.locked = False

    def __enter__(self):
        if self.write:
            if not os.path.isdir(self.dir):
                raise Fail(1, "campaign %s not found; create it with vajra-campaign.sh create" % self.slug)
            self._lock()
        self.data = self._load()
        return self

    def __exit__(self, *exc):
        if self.locked:
            try:
                os.rmdir(self.lock_dir)
            except OSError:
                pass
        return False

    def _lock(self):
        for _ in range(100):
            try:
                os.mkdir(self.lock_dir, 0o700)
                self.locked = True
                return
            except FileExistsError:
                try:
                    if time.time() - os.stat(self.lock_dir).st_mtime > 120:
                        os.rmdir(self.lock_dir)   # stale: a crashed writer
                        continue
                except OSError:
                    pass
                time.sleep(0.05)
        raise Fail(1, "tasks.json is locked (%s); remove it if no other run is active" % self.lock_dir)

    def _load(self):
        if not os.path.exists(self.path):
            return {"schema": SCHEMA, "campaign": self.slug, "tasks": []}
        try:
            with open(self.path) as f:
                d = json.load(f)
        except (OSError, ValueError) as e:
            raise Fail(1, "cannot read %s: %s" % (self.path, e))
        if not isinstance(d, dict) or d.get("schema") != SCHEMA or not isinstance(d.get("tasks"), list):
            raise Fail(1, "%s is not a %s file" % (self.path, SCHEMA))
        return d

    def task(self, tid):
        for t in self.data["tasks"]:
            if t["id"] == tid:
                return t
        raise Fail(1, "unknown task %s" % tid)

    def save(self):
        derive_ready(self.data["tasks"])
        fd, tmp = tempfile.mkstemp(dir=self.dir, prefix=".tasks.", suffix=".tmp")
        try:
            with os.fdopen(fd, "w") as f:
                json.dump(self.data, f, indent=2, sort_keys=True)
                f.write("\n")
            os.replace(tmp, self.path)
        except BaseException:
            try:
                os.unlink(tmp)
            except OSError:
                pass
            raise


def derive_ready(tasks):
    done = {t["id"] for t in tasks if t["status"] == "done"}
    for t in tasks:
        if t["status"] in ("pending", "ready"):
            t["status"] = "ready" if all(d in done for d in t["deps"]) else "pending"


def csv(v, what, rx=None):
    if v is None or v == "":
        return []
    out = []
    for x in v.split(","):
        x = x.strip()
        if not x:
            continue
        if rx is not None and not rx.match(x):
            raise Fail(2, "invalid %s %r" % (what, x))
        if what == "path" and ".." in x.split("/"):
            raise Fail(2, "invalid path %r" % x)
        if x not in out:
            out.append(x)
    return out


def note(t, what):
    t.setdefault("updates", []).append(one_line(what, 300))
    t["updates"] = t["updates"][-20:]


# ---------------------------------------------------------------- commands
def cmd_add(a):
    if not ID_RE.match(a.id):
        raise Fail(2, "invalid task id %r" % a.id)
    if not AGENT_RE.match(a.owner or ""):
        raise Fail(2, "invalid owner agent %r" % a.owner)
    title = one_line(a.title, 200)
    if not title:
        raise Fail(2, "--title is required")
    deps = csv(a.deps, "dependency", ID_RE)
    if a.id in deps:
        raise Fail(2, "task %s cannot depend on itself" % a.id)
    files = csv(a.files, "path", SAFE_PATH)
    acc = [one_line(x, 300) for x in (a.acceptance or []) if one_line(x, 300)]
    if a.branch and not BRANCH_RE.match(a.branch):
        raise Fail(2, "invalid branch %r" % a.branch)
    defn = {"title": title, "owner_agent": a.owner, "deps": deps, "files": files, "acceptance": acc}
    with Store(a.campaign, write=True) as s:
        old = next((t for t in s.data["tasks"] if t["id"] == a.id), None)
        if old is not None:
            if old["status"] == "done":
                return {"ok": True, "id": a.id, "result": "exists", "status": "done"}
            changed = any(old.get(k) != v for k, v in defn.items())
            old.update(defn)
            if a.branch:
                old["branch"] = a.branch
            if a.worktree:
                old["worktree"] = one_line(a.worktree, 300)
            if changed:
                note(old, "definition updated")
            s.save()
            return {"ok": True, "id": a.id, "result": "updated" if changed else "unchanged", "status": old["status"]}
        if len(s.data["tasks"]) >= MAX_TASKS:
            raise Fail(3, "campaign already has %d tasks" % MAX_TASKS)
        t = dict(defn, id=a.id, status="pending", attempts=0, evidence=None,
                 branch=a.branch or "", worktree=one_line(a.worktree, 300) if a.worktree else "")
        s.data["tasks"].append(t)
        s.save()
        return {"ok": True, "id": a.id, "result": "added", "status": t["status"]}


def cmd_dep(a):
    for x in (a.id, a.dep):
        if not ID_RE.match(x):
            raise Fail(2, "invalid task id %r" % x)
    if a.id == a.dep:
        raise Fail(2, "task %s cannot depend on itself" % a.id)
    with Store(a.campaign, write=True) as s:
        t = s.task(a.id)
        if a.remove:
            if a.dep in t["deps"]:
                t["deps"].remove(a.dep)
        else:
            s.task(a.dep)
            if a.dep not in t["deps"]:
                t["deps"].append(a.dep)
            cyc = find_cycle(s.data["tasks"])
            if cyc:
                raise Fail(2, "dependency cycle: " + " -> ".join(cyc))
        s.save()
        return {"ok": True, "id": a.id, "deps": t["deps"], "status": t["status"]}


def known_agents():
    d = os.path.join(os.path.expanduser("~"), ".claude", "agents")
    try:
        return {f[:-3] for f in os.listdir(d) if f.endswith(".md")}
    except OSError:
        return set()


def cmd_validate(a):
    with Store(a.campaign) as s:
        tasks = s.data["tasks"]
    errors = []
    ids = {t["id"] for t in tasks}
    agents = known_agents()
    for t in tasks:
        if t["owner_agent"] not in agents:
            errors.append("task %s: unknown agent %s (not in ~/.claude/agents)" % (t["id"], t["owner_agent"]))
        for d in t["deps"]:
            if d not in ids:
                errors.append("task %s: unknown dependency %s" % (t["id"], d))
    cyc = find_cycle(tasks)
    if cyc:
        errors.append("dependency cycle: " + " -> ".join(cyc))
    res = {"ok": not errors, "tasks": len(tasks), "errors": errors}
    if errors:
        raise FailJSON(2, res)
    return res


class FailJSON(Fail):
    def __init__(self, code, payload):
        super().__init__(code, "; ".join(payload.get("errors", [])) or payload.get("error", ""))
        self.payload = payload


def cmd_ready(a):
    with Store(a.campaign) as s:
        tasks = s.data["tasks"]
    derive_ready(tasks)
    return {"ok": True, "ready": [t["id"] for t in tasks if t["status"] == "ready"]}


def cmd_claim(a):
    if not AGENT_RE.match(a.worker or "") and not ID_RE.match(a.worker or ""):
        raise Fail(2, "invalid worker name %r" % a.worker)
    with Store(a.campaign, write=True) as s:
        derive_ready(s.data["tasks"])
        t = s.task(a.id)
        if t["status"] in ("claimed", "in_progress") and t.get("claimed_by") == a.worker:
            return {"ok": True, "id": a.id, "result": "already claimed", "status": t["status"], "attempts": t["attempts"]}
        by_id = {x["id"]: x for x in s.data["tasks"]}

        def dep_ok(d):
            x = by_id.get(d)
            if x is None:
                return False
            if x["status"] == "done":
                return True
            # --stacked: build on a dependency whose worker reported done but that is not closed yet. close
            # still needs every dependency done, so the stacked task can only close after it.
            return bool(a.stacked) and x["status"] == "in_progress" and (x.get("report") or {}).get("status") == "done"

        deps_ok = all(dep_ok(d) for d in t["deps"])
        # A blocked, question or failed task is retried by claiming it again once its dependencies allow it.
        retry = t["status"] in ("blocked", "question", "failed") and deps_ok
        stacked = t["status"] == "pending" and a.stacked and deps_ok
        if t["status"] != "ready" and not retry and not stacked:
            who = (" by " + t["claimed_by"]) if t.get("claimed_by") and t["status"] in ("claimed", "in_progress") else ""
            raise Fail(3, "task %s is %s%s, not ready" % (a.id, t["status"], who))
        was = t["status"]
        t["status"] = "claimed"
        t["claimed_by"] = a.worker
        t["attempts"] = int(t.get("attempts") or 0) + 1
        note(t, "claimed by " + a.worker + (" (retry after %s)" % was if retry else "")
             + (" (stacked on reported-done dependencies)" if not all(by_id[d]["status"] == "done" for d in t["deps"]) else ""))
        s.save()
        return {"ok": True, "id": a.id, "result": "claimed", "status": "claimed", "attempts": t["attempts"]}


REPORT_MAP = {"progress": "in_progress", "in_progress": "in_progress", "blocked": "blocked",
              "question": "question", "failed": "failed", "done": "in_progress"}


def cmd_report(a):
    if a.status not in REPORT_MAP:
        raise Fail(2, "report status must be one of: " + ", ".join(REPORT_MAP))
    with Store(a.campaign, write=True) as s:
        derive_ready(s.data["tasks"])
        t = s.task(a.id)
        if t["status"] == "done":
            raise Fail(3, "task %s is already done (closed with evidence)" % a.id)
        new = REPORT_MAP[a.status]
        # Work can only be in progress on a task that is claimed, already in progress, or ready (deps done).
        # A blocked, question or failed task is retried through claim; a pending one waits for its deps.
        if new == "in_progress" and t["status"] not in ("claimed", "in_progress", "ready"):
            raise Fail(3, "task %s is %s; report %s needs it claimed, in_progress or ready (claim it to retry)"
                       % (a.id, t["status"], a.status))
        # A ready task reported in progress without a claim starts an attempt here.
        if new == "in_progress" and t["status"] == "ready":
            t["attempts"] = int(t.get("attempts") or 0) + 1
        t["status"] = new
        t["report"] = {"status": a.status, "summary": one_line(a.summary, 500)}
        note(t, "report %s" % a.status + (" (a claim only; closes with evidence)" if a.status == "done" else ""))
        s.save()
        return {"ok": True, "id": a.id, "status": t["status"], "reported": a.status, "attempts": t["attempts"]}


def vajra_key():
    """The Vajra HMAC key (hex in ~/.claude/vajra/.hmac-key), or None. Read only; never created here."""
    kf = os.path.join(os.environ.get("HOME") or os.path.expanduser("~"), ".claude", "vajra", ".hmac-key")
    try:
        with open(kf, "r", encoding="utf-8") as f:
            k = f.read().strip()
    except OSError:
        return None
    if not re.match(r"^[0-9a-fA-F]+$", k) or len(k) % 2:
        return None
    return bytes.fromhex(k)


def git(args, cwd=None):
    try:
        r = subprocess.run(["git"] + args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return r.stdout.decode("utf-8", "replace").strip() if r.returncode == 0 else None


def evidence_hmac_ok(ev, key):
    """Signed-evidence contract: hmac = HMAC-SHA256(key, canonical JSON of the object without hmac), canonical =
    sort_keys, separators (',', ':'), UTF-8. Both ensure_ascii spellings of that JSON are accepted: they encode
    the same data and both need the key."""
    h = ev.get("hmac")
    if not isinstance(h, str) or not re.match(r"^[0-9a-f]{64}$", h):
        return False
    body = {k: v for k, v in ev.items() if k != "hmac"}
    for ascii_ in (False, True):
        c = json.dumps(body, sort_keys=True, separators=(",", ":"), ensure_ascii=ascii_).encode("utf-8")
        if hmac.compare_digest(h, hmac.new(key, c, hashlib.sha256).hexdigest()):
            return True
    return False


TRIVIAL_CMD = re.compile(r"^(?:true|:|exit\s+0|echo(?:\s.*)?)$", re.S)


def trivial(cmd):
    c = re.sub(r"\s+", " ", str(cmd)).strip().rstrip(";").strip()
    return bool(TRIVIAL_CMD.match(c))


def read_evidence(path, want_head=None):
    """Accept only green, signed verify-work evidence for this repository. This is the shared green rule (the
    same check as vajra_kb.py check_evidence): every command exited 0, git.treeStable is true, git.head is the
    current HEAD of the repository this runs in (or want_head, a commit the caller names), the file is
    <repo_root>/data/walkthroughs/<slug>/evidence.json, the hmac verifies, and not every command is trivial."""
    def refuse(why):
        raise Fail(3, "evidence %s refused: %s" % (path, why))
    root = git(["rev-parse", "--show-toplevel"])
    if not root:
        refuse("not run inside a git repository")
    try:
        real = os.path.realpath(path)
        with open(real, "rb") as f:
            raw = f.read(5_000_000)
        ev = json.loads(raw.decode("utf-8"))
    except (OSError, ValueError, UnicodeDecodeError) as e:
        raise Fail(3, "evidence %s unreadable: %s" % (path, e))
    base = os.path.join(os.path.realpath(root), "data", "walkthroughs")
    rel = os.path.relpath(real, base)
    parts = rel.split(os.sep)
    if len(parts) != 2 or parts[1] != "evidence.json" or parts[0] in ("", ".", ".."):
        refuse("it must be <repo_root>/data/walkthroughs/<slug>/evidence.json")
    if not isinstance(ev, dict) or not isinstance(ev.get("commands"), list):
        refuse("not a verify-work evidence.json")
    cmds = ev["commands"]
    total = ev.get("commandsTotal")
    if ev.get("allPassed") is not True or not isinstance(total, int) or total < 1 or ev.get("commandsFailed") != 0 \
            or len(cmds) != total or not all(isinstance(c, dict) and c.get("exit") == 0 and c.get("passed") is not False
                                             for c in cmds):
        refuse("not a green verify-work run")
    if all(trivial(c.get("cmd", "")) for c in cmds):
        refuse("every check is trivial (true, :, echo, exit 0)")
    g = ev.get("git")
    if not isinstance(g, dict) or g.get("treeStable") is not True:
        refuse("the working tree was not stable while the checks ran")
    head = g.get("head")
    if not isinstance(head, str) or not re.match(r"^[0-9a-f]{40}$", head):
        refuse("no head commit recorded")
    want = git(["rev-parse", "--verify", "-q", (want_head or "HEAD") + "^{commit}"], cwd=root)
    if not want or want != head:
        refuse("head %s is not %s" % (head[:12], want_head or "the current HEAD"))
    key = vajra_key()
    if key is None:
        refuse("no Vajra key to check its signature")
    if not evidence_hmac_ok(ev, key):
        refuse("signature (hmac) missing or invalid")
    return {"path": real, "sha256": hashlib.sha256(raw).hexdigest(), "head": head, "commands": total}


def cmd_close(a):
    if a.commit and not SHA_RE.match(a.commit):
        raise Fail(2, "invalid --commit")
    if a.base_commit and not SHA_RE.match(a.base_commit):
        raise Fail(2, "invalid --base-commit")
    if a.branch and not BRANCH_RE.match(a.branch):
        raise Fail(2, "invalid --branch")
    changed = []
    if a.changed:
        changed = [one_line(x, 300) for x in a.changed.split(",") if one_line(x, 300)][:200]
    if a.head and not SHA_RE.match(a.head):
        raise Fail(2, "invalid --head")
    with Store(a.campaign, write=True) as s:
        derive_ready(s.data["tasks"])
        t = s.task(a.id)
        if t["status"] == "done":
            return {"ok": True, "id": a.id, "result": "already done", "status": "done"}
        if t["status"] not in ("claimed", "in_progress"):
            raise Fail(3, "task %s is %s; only a claimed or in_progress task can be closed" % (a.id, t["status"]))
        done = {x["id"] for x in s.data["tasks"] if x["status"] == "done"}
        open_deps = [d for d in t["deps"] if d not in done]
        if open_deps:
            raise Fail(3, "task %s has dependencies that are not done: %s" % (a.id, ", ".join(open_deps)))
        ev = read_evidence(a.evidence, a.head or None)
        t["status"] = "done"
        t["evidence"] = ev
        for k, v in (("commit", a.commit), ("branch", a.branch), ("base_commit", a.base_commit)):
            if v:
                t[k] = v
        if changed:
            t["changed"] = changed
        note(t, "closed with evidence %s" % ev["sha256"][:12])
        s.save()
        return {"ok": True, "id": a.id, "result": "closed", "status": "done"}


def cmd_status(a):
    with Store(a.campaign) as s:
        derive_ready(s.data["tasks"])
        t = s.task(a.id)
    return {"ok": True, "task": t}


def cmd_list(a):
    with Store(a.campaign) as s:
        derive_ready(s.data["tasks"])
        return {"ok": True, "tasks": s.data["tasks"]}


def cmd_waves(a):
    if a.max_in_flight is not None and not (1 <= a.max_in_flight <= 64):
        raise Fail(2, "--max-in-flight must be 1-64")
    with Store(a.campaign) as s:
        tasks = s.data["tasks"]
    cyc = find_cycle(tasks)
    if cyc:
        raise Fail(2, "dependency cycle: " + " -> ".join(cyc))
    return compute_waves(tasks, a.max_in_flight)


def mermaid_text(s):
    return re.sub(r"[^A-Za-z0-9 _.,:/@+-]", " ", s)[:80].strip()


def cmd_graph(a):
    with Store(a.campaign) as s:
        tasks = s.data["tasks"]
    derive_ready(tasks)
    node = {t["id"]: "n%d" % i for i, t in enumerate(tasks)}
    lines = ["graph TD"]
    for t in tasks:
        lines.append('  %s["%s: %s (%s, %s)"]' % (node[t["id"]], mermaid_text(t["id"]), mermaid_text(t["title"]),
                                                  mermaid_text(t["owner_agent"]), t["status"]))
    for t in tasks:
        for d in t["deps"]:
            if d in node:
                lines.append("  %s --> %s" % (node[d], node[t["id"]]))
    return "\n".join(lines)


# ---------------------------------------------------------------- output
def human(cmd, res):
    if cmd == "graph":
        return res
    if cmd == "waves":
        return json.dumps(res)
    if cmd == "status":
        t = res["task"]
        return "%s %s (%s, attempts %d): %s" % (t["id"], t["status"], t["owner_agent"], t.get("attempts", 0), t["title"])
    if cmd == "list":
        return "\n".join("%s  %s  %s  %s" % (t["id"], t["status"], t["owner_agent"], t["title"]) for t in res["tasks"]) or "no tasks"
    if cmd == "ready":
        return "\n".join(res["ready"])
    if cmd == "validate":
        return "ok: %d task(s), graph valid" % res["tasks"]
    return "%s %s%s" % (res.get("id", ""), res.get("result", res.get("status", "ok")),
                        (" (status %s)" % res["status"]) if "result" in res and "status" in res else "")


def main(argv):
    common = argparse.ArgumentParser(add_help=False, allow_abbrev=False)
    common.add_argument("--campaign", required=True, help="campaign slug")
    common.add_argument("--json", action="store_true", help="JSON output")
    p = argparse.ArgumentParser(prog="vajra_tasks.py", description="Vajra campaign task graph", allow_abbrev=False)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("add", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    s.add_argument("--title", required=True)
    s.add_argument("--owner", required=True)
    s.add_argument("--deps", default="")
    s.add_argument("--files", default="")
    s.add_argument("--acceptance", action="append")
    s.add_argument("--branch", default="")
    s.add_argument("--worktree", default="")
    s = sub.add_parser("dep", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    s.add_argument("dep")
    s.add_argument("--remove", action="store_true")
    sub.add_parser("validate", parents=[common], allow_abbrev=False)
    sub.add_parser("ready", parents=[common], allow_abbrev=False)
    s = sub.add_parser("claim", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    s.add_argument("worker")
    s.add_argument("--stacked", action="store_true",
                   help="also accept dependencies whose workers reported done but that are not closed yet")
    s = sub.add_parser("report", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    s.add_argument("status")
    s.add_argument("--summary", default="")
    s = sub.add_parser("close", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    s.add_argument("--evidence", required=True)
    s.add_argument("--commit", default="")
    s.add_argument("--branch", default="")
    s.add_argument("--base-commit", default="")
    s.add_argument("--changed", default="")
    s.add_argument("--head", default="", help="commit the evidence must be for (default: the current HEAD)")
    s = sub.add_parser("status", parents=[common], allow_abbrev=False)
    s.add_argument("id")
    sub.add_parser("list", parents=[common], allow_abbrev=False)
    s = sub.add_parser("waves", parents=[common], allow_abbrev=False)
    s.add_argument("--max-in-flight", type=int, default=None)
    sub.add_parser("graph", parents=[common], allow_abbrev=False)
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    os.umask(0o077)
    fn = globals()["cmd_" + a.cmd]
    try:
        res = fn(a)
    except FailJSON as e:
        if a.json:
            print(json.dumps(e.payload))
        print("vajra_tasks: " + e.msg, file=sys.stderr)
        return e.code
    except Fail as e:
        if a.json:
            print(json.dumps({"ok": False, "error": e.msg}))
        print("vajra_tasks: " + e.msg, file=sys.stderr)
        return e.code
    if a.json:
        print(json.dumps(res if a.cmd != "graph" else {"ok": True, "mermaid": res}))
    else:
        print(human(a.cmd, res))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

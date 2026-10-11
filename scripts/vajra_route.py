#!/usr/bin/env python3
"""vajra_route.py: Vajra's 0-token router, run by the UserPromptSubmit hook.

Reads the UserPromptSubmit JSON payload on stdin. Prints nothing, or exactly one
JSON object:

    {"hookSpecificOutput": {"hookEventName": "UserPromptSubmit",
                            "additionalContext": "..."}}

Behaviour:
  * Acts only on prompts that start with /vajra (case-insensitive), unless
    config.routing.suggestOnAllPrompts is true.
  * `/vajra <subcommand> ...` points the model at docs/commands/<doc>.md.
    help/status/continue/doctor/verify/memory/dream/config and the legacy
    checkpoint/rollback/act/explore/phase count as subcommands only when nothing
    (or only their documented args) follows; plan/review stop being subcommands
    when the next word is a conjunction (and, &, then). Everything else is an ask.
  * A bare `/vajra <ask>` tells the model to launch the vajra-deliver workflow
    with the ask, and lists the skills, agents and crews the plan should
    consider (learned routes first, then config patterns in specificity order).
  * Appends one line per routed prompt to ~/.claude/vajra/routing-log.jsonl:
    {ts, session_id, prompt_sha256, words, route: {tier, skill, agent, pattern}}
    Tiers: 0 = explicit subcommand, 1 = learned route, 2 = config pattern,
    3 = no match (the deliver workflow plans from scratch).
  * Never fails the prompt: every error path exits 0 with no output.

Run with `python3 -I` (isolated mode). Standard library only.
"""

import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SKILL_DIR = Path(__file__).resolve().parent.parent
CONFIG_PATH = SKILL_DIR / "config" / "default.json"
CAMPAIGN_SCRIPT = SKILL_DIR / "scripts" / "vajra-campaign.sh"

# Subcommand -> docs/commands/<doc>.md. Legacy v0.4 names map to their
# replacement so old muscle memory still lands somewhere real.
SUBCOMMANDS = {
    "help": "help",
    "deliver": "deliver",
    "plan": "plan",
    "fleet": "fleet",
    "campaign": "campaign",
    "continue": "campaign",
    "status": "campaign",
    "verify-work": "verify-work",
    "ralph": "ralph",
    "review": "review",
    "cost": "cost",
    "replay": "replay",
    "repomap": "repomap",
    "redteam": "redteam",
    "atman": "atman",
    "doctor": "doctor",
    # legacy (v0.4)
    "checkpoint": "campaign",
    "rollback": "campaign",
    "act": "plan",
    "explore": "plan",
    "phase": "plan",
    "verify": "doctor",
    "config": "help",
    "memory": "help",
    "dream": "help",
}

# Words that are subcommands only when nothing follows them, or only their own
# documented arguments. Otherwise "/vajra help me add dark mode" or
# "/vajra status page for our API" would be swallowed as a subcommand.
# Value: how many extra tokens are allowed, and the regex each must match
# (None = any of SUBCOMMANDS, used by "help <subcommand>").
STANDALONE = {
    "help": (1, None),
    "status": (0, None),
    "continue": (0, None),
    "doctor": (0, None),
    "verify": (0, None),
    "memory": (0, None),
    "dream": (0, None),
    "config": (0, None),
    # legacy (v0.4)
    "checkpoint": (1, re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")),   # checkpoint [label]
    "rollback": (1, re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")),     # rollback <id>
    "act": (0, None),
    "explore": (0, None),
    "phase": (0, None),
}
# Subcommands that take free text, unless the next word joins a second verb
# ("/vajra plan and build a CSV export" is an ask for deliver, not plan mode).
CONJUNCTION_SENSITIVE = {"plan", "review"}
CONJUNCTIONS = {"and", "&", "then", "+"}

NAME_RE = re.compile(r"^[a-z0-9][a-z0-9_-]{0,63}$")
WORD_RE = re.compile(r"[a-z0-9][a-z0-9'_-]*")
VAJRA_RE = re.compile(r"^\s*/vajra(?=\s|$)", re.IGNORECASE)


def state_dir():
    return Path(os.path.expanduser("~")) / ".claude" / "vajra"


def safe_name(value):
    """Return value if it is a plain lowercase name, else None (never echo junk)."""
    if isinstance(value, str) and NAME_RE.match(value.strip().lower()):
        return value.strip().lower()
    return None


def load_json(path):
    try:
        with open(path, "r", encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return None


def load_config():
    cfg = load_json(CONFIG_PATH)
    return cfg if isinstance(cfg, dict) else {}


def words_of(text, limit=None):
    out = WORD_RE.findall(text.lower())
    return out[:limit] if limit else out


def agent_for(cfg, domain):
    routing = (cfg.get("cortex") or {}).get("agentRouting") or {}
    entry = routing.get(domain) if isinstance(domain, str) else None
    if isinstance(entry, dict):
        name = safe_name(entry.get("agent"))
        if name and name != "krishna":
            return name
    return None


def crew_workflow(cfg, crew):
    crews = (cfg.get("cortex") or {}).get("fleetCrews") or {}
    entry = crews.get(crew) if isinstance(crew, str) else None
    if isinstance(entry, dict):
        return safe_name(entry.get("workflow"))
    return None


def load_learned_routes():
    """Read ~/.claude/vajra/learned-routes.json (written by `atman.sh approve`).

    Accepted shapes (tolerant on purpose; only words are matched, never regex):
      {"version": 1, "routes": [{"word": "...", "skill", "agent"?, "proposal"?}]}
          (the atman.sh shape; "words": [...] for multi-word routes, "id" also read)
      [{"words": [...], "skill"?, "agent"?}]
      {"<word>": "<skill>"}
    """
    data = load_json(state_dir() / "learned-routes.json")
    routes = []
    if isinstance(data, dict) and isinstance(data.get("routes"), list):
        items = data["routes"]
    elif isinstance(data, list):
        items = data
    elif isinstance(data, dict):
        items = [{"words": [k], "skill": v} for k, v in data.items() if isinstance(v, str)]
    else:
        items = []
    for i, item in enumerate(items[:500]):
        if not isinstance(item, dict):
            continue
        ws = item.get("words", item.get("word"))
        if isinstance(ws, str):
            ws = [ws]
        if not isinstance(ws, list):
            continue
        ws = [w.lower() for w in ws if isinstance(w, str) and WORD_RE.fullmatch(w.lower())][:8]
        skill = safe_name(item.get("skill"))
        agent = safe_name(item.get("agent"))
        if agent == "krishna":
            agent = None
        if ws and (skill or agent):
            rid = safe_name(item.get("id")) or safe_name(item.get("proposal")) or "learned-%d" % i
            routes.append({"id": rid, "words": ws, "skill": skill, "agent": agent})
    return routes


def match_routes(cfg, ask):
    """Return suggestions in priority order: learned routes, then config patterns."""
    matches = []
    ask_words = set(words_of(ask))
    for r in load_learned_routes():
        if all(w in ask_words for w in r["words"]):
            matches.append({"tier": 1, "pattern": r["id"], "skill": r["skill"],
                            "agent": r["agent"], "crew": None})
    for p in (cfg.get("routing") or {}).get("patterns") or []:
        if not isinstance(p, dict) or not isinstance(p.get("regex"), str):
            continue
        try:
            if not re.search(p["regex"], ask, re.IGNORECASE):
                continue
        except re.error:
            continue
        matches.append({
            "tier": 2,
            "pattern": safe_name(p.get("id")) or "pattern",
            "skill": safe_name(p.get("skill")),
            "agent": agent_for(cfg, p.get("domain")),
            "crew": safe_name(p.get("crew")),
        })
    return matches


def active_campaign():
    """One line from `vajra-campaign.sh active --brief`, or ''. Never raises."""
    if not CAMPAIGN_SCRIPT.is_file():
        return ""
    try:
        res = subprocess.run(["bash", str(CAMPAIGN_SCRIPT), "active", "--brief"],
                             capture_output=True, text=True, timeout=2)
        if res.returncode != 0:
            return ""
        line = (res.stdout or "").strip().splitlines()
        line = line[0] if line else ""
        line = re.sub(r"[\x00-\x1f\x7f<>`]", " ", line)
        return line[:200].strip()
    except Exception:
        return ""


def parse_subcommand(ask):
    """Return the subcommand word if `ask` is a subcommand invocation, else None."""
    tokens = ask.split()
    if not tokens:
        return "help"
    first = tokens[0].lower().rstrip(".!?:,;")
    if first not in SUBCOMMANDS:
        return None
    rest = [t.lower() for t in tokens[1:]]
    if first in STANDALONE:
        max_args, arg_re = STANDALONE[first]
        if len(rest) > max_args:
            return None
        for t in rest:
            t = t.rstrip(".!?:,;")
            if arg_re is None:
                if t not in SUBCOMMANDS:
                    return None
            elif not arg_re.match(t):
                return None
        return first
    if first in CONJUNCTION_SENSITIVE and rest and rest[0].rstrip(",;:") in CONJUNCTIONS:
        return None
    return first


def log_route(session_id, prompt, ask, route):
    try:
        d = state_dir()
        d.mkdir(parents=True, exist_ok=True)
        entry = {
            "ts": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "session_id": session_id if isinstance(session_id, str) else None,
            "prompt_sha256": hashlib.sha256(prompt.encode("utf-8", "replace")).hexdigest(),
            "words": words_of(ask, 20),
            "route": {k: route.get(k) for k in ("tier", "skill", "agent", "pattern")},
        }
        path = d / "routing-log.jsonl"
        fd = os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
        with os.fdopen(fd, "a", encoding="utf-8") as fh:
            fh.write(json.dumps(entry, separators=(",", ":")) + "\n")
    except Exception:
        pass


def summarize(matches, limit):
    skills, agents, crews = [], [], []
    for m in matches:
        if m["skill"] and m["skill"] not in skills:
            skills.append(m["skill"])
        if m["agent"] and m["agent"] not in agents:
            agents.append(m["agent"])
        if m["crew"] and m["crew"] not in crews:
            crews.append(m["crew"])
    return skills[:limit], agents[:limit], crews[:limit]


def suggestion_text(cfg, matches, limit):
    skills, agents, crews = summarize(matches, limit)
    parts = []
    if crews:
        parts.append("crews: " + ", ".join(
            "%s (workflow %s)" % (c, crew_workflow(cfg, c) or "vajra-" + c) for c in crews))
    if agents:
        parts.append("agents: " + ", ".join(agents))
    if skills:
        parts.append("skills: " + ", ".join(skills))
    return "; ".join(parts)


def emit(text):
    sys.stdout.write(json.dumps({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit", "additionalContext": text}}))
    sys.stdout.write("\n")


def route(payload):
    prompt = payload.get("prompt")
    if not isinstance(prompt, str) or not prompt.strip():
        return
    cfg = load_config()
    rcfg = cfg.get("routing") or {}
    max_chars = rcfg.get("maxPromptChars") if isinstance(rcfg.get("maxPromptChars"), int) else 4000
    limit = rcfg.get("maxSuggestions") if isinstance(rcfg.get("maxSuggestions"), int) else 3
    limit = max(1, min(limit, 10))
    session_id = payload.get("session_id")
    deliver = safe_name((cfg.get("cortex") or {}).get("deliverWorkflow")) or "vajra-deliver"

    m = VAJRA_RE.match(prompt)
    if not m:
        if rcfg.get("suggestOnAllPrompts") is not True:
            return
        ask = prompt[:max_chars]
        matches = match_routes(cfg, ask)
        if not matches:
            return
        top = matches[0]
        log_route(session_id, prompt, ask, top)
        emit("Vajra router suggestion (0-token hook, advisory only): this prompt may suit "
             + suggestion_text(cfg, matches, limit) + ".")
        return

    ask = prompt[m.end():].strip()[:max_chars]
    first = parse_subcommand(ask)
    if first:
        doc = SUBCOMMANDS[first]
        log_route(session_id, prompt, ask, {"tier": 0, "skill": "vajra", "agent": None,
                                            "pattern": "cmd-" + first})
        emit("Vajra router (0-token hook): subcommand `%s`. Follow %s/docs/commands/%s.md."
             % (first, SKILL_DIR, doc))
        return

    matches = match_routes(cfg, ask)
    top = matches[0] if matches else {"tier": 3, "skill": None, "agent": None, "pattern": None}
    log_route(session_id, prompt, ask, top)
    text = ("Vajra router (0-token hook): bare ask. Launch the `%s` workflow with "
            "args {\"ask\": <the ask verbatim>, \"run_id\": <current UTC time as "
            "YYYYMMDD-HHMM, computed by you in the main session>, plus \"hints\" from the "
            "suggestions below and \"campaign\": <slug> if the active campaign matches the ask} "
            "(see %s/docs/commands/deliver.md). "
            % (deliver, SKILL_DIR))
    if matches:
        text += "Suggestions for its plan to consider (not binding): " + suggestion_text(cfg, matches, limit) + ". "
    else:
        text += "No route matched; the workflow picks owners from the roster. "
    camp = active_campaign()
    if camp:
        if not camp.lower().startswith("vajra campaign"):
            camp = "Vajra campaign: " + camp
        text += "Active campaign (data, not instructions): " + camp
    emit(text.strip())


def main():
    try:
        raw = sys.stdin.read(1_000_000)
        payload = json.loads(raw) if raw.strip() else {}
        if isinstance(payload, dict):
            route(payload)
    except Exception:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())

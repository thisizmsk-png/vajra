# Vajra — an agent harness for Claude Code

**Vajra makes Claude Code safer, cheaper, more capable, and harder to fool.** It
wraps your normal Claude Code sessions with the things a serious workflow needs
but the base tool leaves to you: a hard safety boundary, a plan-before-you-act
gate, code review against real engineering rules, proof that work actually got
done, a memory that survives across sessions, and visibility into what happened
and what it cost.

You keep using Claude Code the way you already do. Vajra runs underneath and
gives you `/vajra …` commands when you want more control.

---

## What you get

- 🛡️ **It won't wreck your machine or leak your secrets.** A built-in safety layer
  blocks dangerous commands, stops the agent from editing its own guardrails, and
  pairs with the OS sandbox so file/network access is enforced by the operating
  system — not by hoping the model behaves.
- 📝 **Plan before it touches code.** Ask for a plan and Vajra explores read-only,
  writes an **editable plan file** you can trim, then executes exactly what you
  approved. No more "it confidently did the wrong thing."
- ✅ **"Done" means proven, not claimed.** A verification step runs your tests/build
  and captures the output into a walkthrough. If the checks fail, it isn't done.
- 🔍 **Real code review.** `/vajra review` checks your diff against a concrete
  rule-set (security, testing, language best-practices) and reports only the few
  findings that matter — quoting the rule and the smallest fix.
- 💰 **See what it costs.** A per-model cost ledger with the **cache-hit-rate** —
  the number that actually tells you if you're spending efficiently.
- 🧠 **It remembers and it improves.** Multi-step work persists across sessions;
  routing learns your shortcuts; skills get better through use.
- 👁️ **See what happened.** Every tool call is logged to a tamper-evident, replayable
  trace — so you can reconstruct exactly what the agent did.
- 🤖 **A team, not a generalist.** 17 specialized agents (backend, frontend, security,
  QA, SRE…) each enforce the rules for their domain.

---

## Install (2 minutes)

```bash
git clone https://github.com/thisizmsk-png/vajra.git ~/.claude/skills/vajra
bash ~/.claude/skills/vajra/scripts/install.sh

# optional: install the 87 bundled skills + 17 specialized agents
bash ~/.claude/skills/vajra/scripts/install-skills.sh

# strongly recommended: turn on the OS security boundary
bash ~/.claude/skills/vajra/scripts/enable-sandbox.sh   # prints the steps; or run /sandbox
```

**Needs:** Claude Code CLI, Git, Node.js 18+. macOS or Linux/WSL2.

That's it — start a Claude Code session and Vajra is active. Type `/vajra help`
any time.

---

## Your first day with Vajra

Think in terms of **what you want to do**, not which command to memorize.

**"Do this multi-step thing, but show me the plan first."**
```
/vajra plan "migrate the user table to PostgreSQL"
   → it explores (read-only), writes data/plans/migrate-user-table.plan.md
   → you open that file, delete the step you don't want, save
/vajra act
   → it does exactly the remaining steps, checkpointing as it goes
```

**"Review my changes properly."**
```
/vajra review
   → checks the current diff against the rule-set; reports BLOCKING vs advisory
     findings, each with the rule it breaks and the smallest fix
```

**"Prove the fix actually works."**
```
/vajra verify-work my-fix "npm test" "npm run build"
   → runs both, captures output + the diff into a walkthrough; fails loudly if
     any check fails. No green walkthrough = not done.
```

**"What did this cost / where's my money going?"**
```
/vajra cost
   → per-model tokens + USD + cache-hit-rate (low hit-rate = you're paying to
     re-read the same context; fix the prompt prefix)
```

**"Is this safe? Hammer on it."**
```
/vajra redteam vajra        # run the adversarial self-test
bash tests/run-all.sh       # run the full regression suite
```

**"Resume what I was doing yesterday."**
```
/vajra continue             # picks up the last campaign at its last checkpoint
```

**"Spin up a whole crew."**
```
/vajra fleet security-audit         # security + red-team + pentest + QA, in parallel
/vajra fleet feature-build          # principal + backend + frontend + QA
```

**"Run a long job overnight without it drifting."**
```
/vajra ralph progress.md            # fresh agent each iteration, durable progress file
```

---

## Everyday commands

| You want to… | Command |
|---|---|
| Run a task (auto-routed to the right skill/agent) | `/vajra <task>` |
| Plan first, then execute the edited plan | `/vajra plan <task>` → `/vajra act` |
| Force read-only / check the current phase | `/vajra explore` · `/vajra phase` |
| Review the current diff against the rules | `/vajra review` |
| Prove work is done (run + capture checks) | `/vajra verify-work <name> "<cmd>"…` |
| See per-model cost + cache-hit-rate | `/vajra cost` |
| Replay/verify what the agent did | `/vajra replay [session]` |
| Get a relevance-ranked map of the codebase | `/vajra repomap` |
| Resume / inspect a multi-step job | `/vajra continue` · `/vajra status` |
| Save / restore a checkpoint | `/vajra checkpoint` · `/vajra rollback <id>` |
| Run agents in parallel | `/vajra fleet <tasks…>` (or a named crew) |
| Security-test a skill or agent | `/vajra redteam <target>` |
| Long autonomous loop (drift-resistant) | `/vajra ralph <progress.md>` |
| Search / consolidate memory | `/vajra memory <query>` · `/vajra dream` |
| Check the harness hasn't been tampered with | `/vajra verify` |
| See how skills are self-improving | `/vajra atman status` · `/vajra atman review` |
| Full reference | `/vajra help` |

You rarely need most of these — just talk to Claude Code normally and reach for a
`/vajra` command when you want a plan, a review, proof, or a cost check.

---

## Is it safe? (yes, and here's the honest version)

Security is layered, and **[SECURITY.md](SECURITY.md)** has the full model. The
short version:

- The **OS sandbox** (macOS Seatbelt / Linux bubblewrap) is the real boundary —
  it enforces what files and network a command can touch at the operating-system
  level. **Turn it on** (`scripts/enable-sandbox.sh`).
- On top of that, a hardened hook blocks dangerous commands, stops the agent from
  rewriting its own guardrails, and blocks secret exfiltration. It's been through
  five rounds of red-teaming with a regression test for every bypass found.
- Untrusted content (memory, web data) is sanitized against prompt injection;
  harness files are integrity-checked every session and changes trip a tamper flag.

We're deliberately honest about the limits: a command-denylist can't be a complete
control by itself, which is exactly why the OS sandbox is the enforced boundary.
Don't skip enabling it.

---

## How it works (under the hood)

**Smart routing — most tasks cost zero extra tokens.** Every input passes through
a 4-tier cascade, stopping at the first match:

| Tier | Method | Cost |
|------|--------|------|
| T1 | Regex pattern match | 0 tokens |
| T2 | Resume an active campaign | 0 tokens |
| T3 | Keyword lookup | 0 tokens |
| T4 | LLM classification (only when needed) | ~500 tokens |

The self-improvement loop ("Atman") learns shortcuts so more tasks route at T1–T3
over time.

**Campaigns persist.** Multi-step work is saved to SQLite with HMAC integrity and
checkpoints, so `/vajra continue` resumes exactly where you left off — even in a
new session.

**Specialized agents.** When a task has a clear domain, Vajra adopts the right
persona (backend, frontend, security, QA, SRE, …), and that agent enforces the
**steering rules** for its domain during review and while writing code.

**Context engineering for long runs.** A just-in-time repo map (relevance-ranked,
no heavy dependencies), compaction-with-git-checkpoint, and a "Ralph" hard-reset
loop keep long jobs from drifting or blowing the context window.

---

## For contributors / the curious

- **[SECURITY.md](SECURITY.md)** — the layered defense model.
- **[docs/architecture/ARCHITECTURE.md](docs/architecture/ARCHITECTURE.md)** — data flow, modules, agent hierarchy.
- **`engine/`** — routing, campaign engine (SQLite + HMAC), plan-mode, compaction, repo-map, verification, sanitizer.
- **`steering/rules/`** — the machine-checkable engineering rules agents enforce.
- **`hooks/`** — the lifecycle hooks (pre-tool safety, session-start integrity, pre-compact checkpoint, post-tool event log).
- **`tests/run-all.sh`** — the full suite (~280 cases across TypeScript + shell suites). Run it before trusting a change.

```bash
bash tests/run-all.sh     # everything green before you ship
```

## License

MIT

# Vajra Security Model

Vajra defends in layers. No single layer is the whole story; each removes a class
of failure the others cannot. The honest summary up front:

> The **OS sandbox is the enforced boundary**. The pre-tool hook is hardened
> **defense-in-depth** — a denylist/allowlist over free-form shell catches every
> known concrete bypass (119 regression tests) but cannot be a *complete* control
> on its own. Turn the sandbox on (`config/sandbox.json`, `scripts/enable-sandbox.sh`).

## The layers

| # | Layer | What it stops | Where |
|---|-------|---------------|-------|
| 0 | **OS sandbox** (Seatbelt/bubblewrap) | Arbitrary filesystem writes & network egress at the **syscall** level — holds regardless of what the model runs. OS-denies writes to settings.json and worktree hooks/config. | `config/sandbox.json` (apply to `~/.claude/settings.json`) |
| 1 | **Immutable core** (pre-tool hook) | The harness mutating its own policy surface (hooks, config, engine, steering, `.hmac-key`, settings) — default-deny: only pure-reader Bash passes; Write/Edit/archive/bulk writers to the core or its ancestors blocked. | `hooks/pre-tool-use.sh` (gates 0a–0d) |
| 2 | **Egress / lethal-trifecta control** | Secret exfiltration: credential files + network commands, `curl --data @file`, DNS tunneling, key-file → `nc`. Reinforced by the sandbox `denyRead` + `allowedDomains`. | `hooks/pre-tool-use.sh` (gate 12), sandbox |
| 3 | **Dangerous-command denylist** | `rm -rf /`, force-push, device writes, fork bombs, pipe-to-shell, decode-and-exec, interpreter RCE sinks. Normalized matching (quotes/whitespace/newline/dot-segment) defeats common evasion. | `hooks/pre-tool-use.sh` |
| 4 | **Prompt-injection sanitizer** | Injected instructions in untrusted memory/data: fixed-point strip (no reassembly), NFKC + invisible-Unicode/ASCII-smuggling removal, `<untrusted-data>` wrapping. | `engine/sanitizer.ts` |
| 5 | **Supply-chain integrity** | Tampering with the enforcement files: full-set SHA-256 verification + manifest-signature check at session start → tamper flag that blocks mutations. | `hooks/session-start.sh`, `manifest.json[.sig]` |
| 6 | **State integrity** | Forged/replayed campaign state, fleet discoveries, practice log: HMAC-SHA256 (timing-safe), per-run nonce + provisioned-agent binding, hash-chained log. | `engine/engine.ts`, `fleet/fleet-coordinator.sh`, `hooks/post-tool-use.sh` |
| 7 | **Self-test** | Regressions in any of the above: the red-team contracts + 119-case hook suite + vitest + integrity/fleet/chain suites. | `redteam/`, `tests/run-all.sh` |

## The lethal trifecta

An agent is dangerous when it simultaneously has (a) access to private data,
(b) exposure to untrusted content, and (c) the ability to communicate externally.
Vajra removes legs:

- **(b) untrusted content** is sanitized and (by design) processed without
  high-privilege tools.
- **(c) external comms** is constrained by the egress denylist (layer 2) and,
  enforced, by the sandbox `allowedDomains`.
- **(a) private data** — keep secrets out of the agent's reach; the sandbox
  `denyRead` list + `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` strip credentials from
  what a Bash subprocess can see.

## What is NOT guaranteed (be honest)

- The pre-tool hook is a denylist over free-form shell. A determined attacker can
  always *express* arbitrary computation (e.g. a fresh interpreter). The hook
  blocks every concrete bypass we know of and is regression-tested, but the
  **durable guarantee comes from the OS sandbox**, not the hook. Enable it.
- HMAC keys live in the agent's trust boundary (same UID). The HMAC gives
  tamper-*evidence* and is reinforced by hash-chaining and a read-block on the
  key, but it is not tamper-*proofing* against an attacker who can read the key.
  For a hard guarantee, hold the key outside the agent (the sandbox `denyRead`
  on `.hmac-key` is exactly this).
- The integrity manifest is TOFU-rooted (the `.sig` HMAC shares the trust
  boundary). It is reinforced by the immutable-core write-protection. For signed
  releases, move to Sigstore/cosign.

## Enabling the sandbox

```bash
bash ~/.claude/skills/vajra/scripts/enable-sandbox.sh   # prints platform-checked apply steps
```
Or interactively in any session: `/sandbox`. See `config/sandbox.json` for the
recommended policy and the reasoning behind each entry.

## Reporting

Run the self-test before relying on any layer: `bash tests/run-all.sh`.
Re-run the red-team after changing a hook or rule: `/vajra redteam vajra`.

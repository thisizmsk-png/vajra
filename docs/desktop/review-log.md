# Vajra Desktop: review log

Review findings and how each was resolved, moved out of [spec/requirements.md](spec/requirements.md) so the spec holds only requirements (ponytail-review, owner tie-break 2026-11-05). Each row still names the acceptance ids that cover it.

## Review findings (formerly requirements 7b)

Sources: architecture review workflow run 20261105-1745 (Architect ADR at commit 6b0cedb on a local branch, three votes: two revise, one approve; its pre-mortem step returned no output, so a Critic pre-mortem was run separately: PM-1 to PM-8 below, also in the PRD); STRIDE (`vajra-3/stride.md`); ponytail review.

| Finding (review) | Decision | Requirement or design change | Acceptance id |
|---|---|---|---|
| B1 Steering needs the nonce tag; plain wrapper is ignored (arch review, tracer) | Fixed | design 5.3, 5.4 step 5.2, engine change E5 | AC-3.2.2 |
| B2 / F2 Stop blocks the final output; stop file cleared too early; resume via SendMessage | Fixed | design 5.3, 5.4 step 4, ADR-8 | AC-3.3.4 to 3.3.7 |
| B3 Control not proven through the Workflow tool | Fixed: validation gate G3b after E1 and E5, before C6 (no cycle) | tasks.md G3b; fallback stop-only (ADR-2) | G3b done-when |
| S1 voter: a stopped worker could still report done | Fixed | design 5.4 step 4 (only status blocked passes) | AC-3.3.4 |
| S2 voter: the verification gate was steerable | Fixed | design 5.4 step 5b, E5 (task briefs only) | AC-3.2.2 |
| S3 voter / M3: queue protection rests on guard regexes | Spec fixed, owner decision | design 5.4 E2 scope; sandbox denyWrite approved (Q10, E8) | E2 tests |
| F3 Lead text labelled human | Fixed | source from actor: human or lead-via-desktop | AC-3.2.2 |
| H1 design section 9 had stale budgets | Fixed | pointer to NFR-PERF-1 to 11; 256 KiB tail | NFR-PERF-2 |
| M1 D1 to D3 and 5.9 still say to verify | Fixed | tasks.md G3; design 5.9 note | G3 |
| M4 terminal-started runs promised steering | Fixed | POS-2.2 narrowed to stop only | POS-2.2 |
| M5 windowless menu bar app may be throttled by App Nap | Added | ADR-9: hold an activity assertion while runs are active; 30 min closed-window test in C3 | POS-2.1 |
| M6 control hook should check the dir before jq | Fixed | design 5.4 step 2 | NFR-PERF-8 |
| L1 app directory name | Fixed | `VajraDesktop` everywhere | none |
| L3 privacy prompts for repos under ~/Documents | Added | C3 test for the TCC prompt | AC-1.1.x |
| F1 forged steer with a leaked nonce (steering threat model) | Mitigated, residual stated | design 5.3 trust model; nonce read blocks (E2); nonce-echo block | AC-3.2.9, AC-3.2.11, AC-3.4.6 |
| F2 stop dodged through handback then SendMessage | Fixed | stop cleared only by a human | AC-3.3.5, AC-3.3.6 |
| F4 the tracer hook logs full payloads | Fixed | the shipped E1 logs ids and tool names only; tracer scripts never ship | AC-3.2.7 (`no_nonce_in_outputs`) |
| STRIDE M-S memory: an agent or any surviving same-user process forges an approval file | Adopted | approvals are Secure Enclave signatures checked against a human-enrolled key; the file's location is not trusted (ADR-12) | AC-7.1.8, NFR-SEC-17 |
| STRIDE M-T memory: pending record swapped after approval | Adopted | approval bound to the record sha256, single use, 5 min | AC-7.1.6 |
| STRIDE M-R memory: "I never approved that" | Adopted | owner authentication plus two audit logs | AC-7.1.4, NFR-SEC-17 |
| STRIDE M-I memory: secrets learned into a rule | Adopted | the ledger write gate on proposals and on edited text | AC-7.1.5 |
| STRIDE M-D memory: proposal flood | Adopted | inbox shows the newest 200 with a count; Atman `pause` | AC-7.1.1 |
| STRIDE M-E memory: an injected proposal grants itself `always` or global scope | Adopted | requested scope never preselected; `applies` comes only from the human (as `vajra_kb.py approve` already does) | AC-7.1.3 |
| STRIDE M-E2 memory: synthetic clicks drive the approve sheet (osascript, Accessibility) | Adopted | owner authentication is required per approval; a click alone approves nothing | AC-7.1.4 |
| STRIDE S1 socket spoofing | Adopted | NFR-SEC-10 | NFR-SEC-10 |
| STRIDE S2 forged inbox item | Adopted | AC-5.1.6 | AC-5.1.6 |
| STRIDE T1 agents edit app data | Adopted | E2 protects the whole app data dir | E2 tests |
| STRIDE T2 symlink escape | Adopted | NFR-SEC-6 amended | NFR-SEC-6 |
| STRIDE R1 human actions unrecorded | Adopted | AC-3.4.5 | AC-3.4.5 |
| STRIDE I1 secrets in spill files | Adopted | NFR-SEC-11 | NFR-SEC-11 |
| STRIDE I2 remote content | Adopted | NFR-SEC-12 | NFR-SEC-12 |
| STRIDE I3 clipboard escapes | Adopted | NFR-SEC-3 plus test | NFR-SEC-3 |
| STRIDE D1 parser floods | Adopted | NFR-SEC-13 | NFR-SEC-13 |
| STRIDE D2 control write blocks | Adopted | NFR-PERF-5 plus test | NFR-PERF-5 |
| STRIDE E1 broad Tauri commands | Adopted | NFR-SEC-14 | NFR-SEC-14 |
| STRIDE E2 re-run of reflowed command | Adopted | POS-1.2 amended | POS-1.2 |
| STRIDE E3 malicious update | Adopted | NFR-SEC-15 | NFR-SEC-15 |
| STRIDE E4 Lead tools widen | Adopted | AC-3.1.4 | AC-3.1.4 |
| STRIDE E5 supply chain | Adopted | NFR-SEC-7 amended | NFR-SEC-7 |
| PM-1 resume continued the wrong campaign (global `.active`) | Fixed | ADR-9 decision 5 resumes by slug | POS-2.1e |
| PM-2 no task owns the live end-to-end checks | Fixed | new task H8, owner-run live acceptance | H8 done-when |
| PM-3 performance gates close after the work they gate | Fixed | new task H0 (perf CI from C1); end-of-M3 budget rule; ADR-9 and design 5 time-boxed review | NFR-PERF-1, NFR-PERF-6 |
| PM-4 a Claude Code update breaks steering silently | Adopted | NFR-COMPAT-1 | NFR-COMPAT-1 |
| PM-5 approvals stranded after the session ends | Adopted | AC-5.2.5 | AC-5.2.5 |
| PM-6 the menu bar app turns a slow leak into a multi-day one | Adopted | ADR-9 6c; 72 h soak; Install now after 72 h | POS-2.1d, NFR-PERF-6 |
| PM-7 transcripts are a single point of failure | Adopted | E4 moves into M1; new E6 records the worktree in the claim so control matches on tasks.json | E6 done-when |
| PM-8 a crash makes resume unsafe | Adopted | ADR-9 6b (process group); fault test | POS-2.1f |
| Ponytail: resume typed into the PTY | Fixed (6db4529) | ADR-9 decision 5 uses argv | POS-2.1b |
| Ponytail: kb approvals prefilled in an app PTY | Fixed (6db4529) | copy to own terminal | POS-3.1 |

## 7c. GPT 6.1 Sol spec review (`vajra-7/spec-review.md`, REQUEST_CHANGES) and PR #2 review rounds

| Finding | Decision | Commit | Acceptance ids |
|---|---|---|---|
| GPT-1 Stop blocks handback and can be bypassed after completion | Fixed: stop exempts exactly `SubagentHandback`, is sticky across `SendMessage` resume, and only a human clears it | 916f4b9 | AC-3.3.4 to AC-3.3.6 |
| GPT-2 Steering uses the wrapper the tracer rejected; no nonce trust model | Fixed: `<vajra-steer nonce=N kind source id>`, nonce at least 128 bits, Execute-phase task workers only, trust model and residual risk stated | a8c02fc | AC-3.2.2, AC-3.2.7 to AC-3.2.11 |
| GPT-3 Queue write protection lacks guard read blocks and the sandbox boundary | Fixed: worker reads of the control dir and transcripts blocked, whole data dir write-protected, same-user limitation stated, sandbox `denyWrite` is Q10 | b251b96 | AC-3.4.6 to AC-3.4.8 |
| GPT-4 Freshness can't detect the demo's file edit | Fixed: "stale" compares the signed whole-tree id with the current one; "unverified" comes from E3 | 5d8a9b8 | POS-1.3 |
| GPT-5 Budget tables contradict the spike | Fixed: one canonical NFR-PERF table with observed values and status; the PRD points to it | d641f83 | NFR-PERF-1 to NFR-PERF-11 |
| GPT-6 Research coverage and MVP exclusions blank | Fixed: every MVP pain point names its task; F1, F2 and F4 added to 7b; OAuth refresh race P24 | e9c071d | AC-1.1.5 |
| GPT-7 Positioning still pass 1, with a contradicted competitor claim | Fixed: final positioning, memory killed as a claim, banned claims, "none runs verify/fix rounds" removed, copy time labelled inference | 870b3ce | POS-1.7, POS-3.4, POS-4.2 to POS-4.7, POS-5.1 to POS-5.5 |
| GPT-8 (optional) Separate the shipping slice | Adopted: two slices, with the evidence loop in the private beta | 074a894 | none (ordering only) |
| PR: Re-run must replay setup commands | Fixed | 5adfbcd | POS-1.2 |
| PR: G3b/E5 circular prerequisite | Fixed: G3b after E1 and E5, before C6 | 0e0928a | G3b done-when |
| PR: campaign-specific resume in POS-2.1b | Fixed | f7d84c1, 148c9f0 | POS-2.1b, POS-2.1e |
| PR: crash cleanup before "no process survives" | Fixed: startup cleanup barrier (ADR-9 6b') | f7d84c1 | POS-2.1f |
| PR: materialize the evidence tree before Re-run | Fixed: apply `diff.patch`, require the signed tree id, else refuse | f7d84c1 | POS-1.2 |
| PR: sandbox-off messaging policy inconsistent | Fixed: sandbox off means stop-only (kept after Q10 was approved, for users who leave the sandbox off) | f7d84c1 | AC-3.4.8 |
| PR: preserve the verification bundle before cleanup | Fixed: engine change E7 archives it with a signed manifest; E3 `--archived` | 6fa24c1 | POS-1.2 |
| PR: lossless executed setup list | Fixed: `setup.json` holds the exact executed `setupCmds`; report lines and raw `setup_cmds` are never sources | 6fa24c1 | POS-1.2 |

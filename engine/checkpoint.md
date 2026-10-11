# Vajra Campaigns — Checkpoint / Resume

Campaigns are how a long ask survives across sessions. In v0.5 the state is plain files
signed with HMAC, driven by `scripts/vajra-campaign.sh`. There is no SQLite database and
no Node engine; the old `engine.ts` (`getActiveCampaign`, `saveCampaign`,
`createCheckpoint`, `rollbackToCheckpoint`) is retired. The script reads the HMAC key
itself, so the key never enters model context.

State for a campaign `<slug>` lives in `~/.claude/vajra/campaigns/<slug>/`:

| File | Contents |
|------|----------|
| `campaign.json` | goal, status (`active`/`done`/`abandoned`), timestamps, and a hash of the feature definitions |
| `features.json` | `{"features":[{"id","description","passes"}]}` — only `passes` ever changes |
| `progress.md` | append-only progress notes |
| `state.sig` | HMAC-SHA256 over the three files above; written only by the script |
| `checkpoints/NNNN.json` | HMAC-signed snapshots, each chained to the previous (`prevHmac`) |

## Commands

```
vajra-campaign.sh create <slug> --goal TEXT --feature "id|description" ...
vajra-campaign.sh list
vajra-campaign.sh show [slug] [--json]
vajra-campaign.sh active [--brief]          # one line or nothing (used by SessionStart)
vajra-campaign.sh pass <feature> [--evidence TEXT] [--revoke]
vajra-campaign.sh progress "<text>"
vajra-campaign.sh checkpoint [label]
vajra-campaign.sh resume [slug] [--from-checkpoint N]
vajra-campaign.sh verify [slug]
vajra-campaign.sh close [slug] [--status done|abandoned]
```

`vajra-deliver` drives these for you: it creates the campaign from the acceptance
criteria (one feature per criterion), records progress and checkpoints each round, and
flips `passes` only when a criterion has verified evidence.

## Checkpointing (save progress)

Checkpoint before a risky or long step, at the start of each phase, and after each
delivered milestone. A checkpoint snapshots `campaign.json`, `features.json` and
`progress.md`, signs them, and chains to the previous checkpoint:

```
vajra-campaign.sh checkpoint "after fix round 1"
```

## Resuming (`/vajra continue`)

`resume` re-verifies the signature, re-marks the campaign active, and prints the goal,
the still-failing features and the recent progress wrapped in a `<campaign-state>` block
(treated as data, not instructions). To continue an ask, `vajra-deliver` is relaunched
with `{"campaign":"<slug>"}`: it reloads the features, keeps the criteria ids, and skips
what already passes.

## Tamper handling (HMAC mismatch)

Any state edited outside the script fails verification. The script then **refuses** to
`pass`, `progress`, `checkpoint` or `resume` (exit 3) and points at the latest valid
checkpoint:

```
state signature INVALID for '<slug>'. Refusing to resume.
  Latest valid checkpoint: N. Restore with: vajra-campaign.sh resume <slug> --from-checkpoint N
```

Do **not** suppress this. Restore the named checkpoint and continue, or, if no valid
checkpoint exists, close the campaign `--status abandoned` and start a new one. A
checkpoint whose own HMAC or chain fails is never restorable; `resume --from-checkpoint`
only restores the newest checkpoint that verifies.

## Security notes

- The HMAC key is at `~/.claude/vajra/.hmac-key` (mode 0600), created with
  `openssl rand -hex 32` on first use. It is never printed and never written into any
  state file.
- Signing and verification use `openssl dgst -sha256 -hmac`. Feature **definitions**
  (ids and descriptions) are additionally pinned by a hash in `campaign.json`, so
  re-signing a hand-edited `features.json` with the real key is still detected.
- A write lock (`<slug>/.lock`) serializes concurrent runs.

# /vajra campaign, /vajra continue, /vajra status

Long work that spans sessions lives in a campaign:
`~/.claude/vajra/campaigns/<slug>/` holds `campaign.json`, `features.json`,
`progress.md` and `checkpoints/`. The script signs state with the local HMAC
key itself, so the key never enters your context. Never read `.hmac-key`.

Always use the script, never edit the files:

```
S=~/.claude/skills/vajra/scripts/vajra-campaign.sh
bash $S create <slug> [--goal TEXT] [--feature "id|description"]...
bash $S list
bash $S show [slug] [--json]
bash $S active [--brief]                 # "<slug>: N/M features passing; ..." or nothing
bash $S pass <feature> [--campaign slug] [--evidence TEXT] [--revoke]
bash $S progress "<text>" [--campaign slug]
bash $S checkpoint [label] [--campaign slug]
bash $S resume [slug] [--from-checkpoint N]
bash $S verify [slug]                    # check the signatures
bash $S close [slug] [--status done|abandoned]
```

`pass` flips one feature's `passes` field; nothing else in `features.json`
changes.

- `/vajra continue` runs `resume` on the active campaign. If verification
  fails, report "Integrity check failed" and stop. Then:
  - For a `deliver-*` campaign (created by `vajra-deliver`), relaunch the
    `vajra-deliver` workflow with
    `args: {"campaign": "<slug>", "run_id": "<UTC now as YYYYMMDD-HHMM>"}`.
    Its goal stands in for the ask, its features are the acceptance criteria,
    and the workflow builds on the last integration branch recorded in the
    campaign progress (`integration vajra/<run>/integration-rN @ <sha> from
    <base>`) instead of starting again from `HEAD`. Pass `base` to choose a
    different commit. The workflow closes the campaign when it delivers.
  - For any other campaign, carry on from the first feature that isn't passing
    yet.
- `/vajra status` runs `show` on the active campaign.
- Mark a feature as passing only after `verify-work` is green for it, and
  pass the walkthrough path as `--evidence`.
- Legacy `checkpoint` and `rollback <id>` land here (use `checkpoint` and
  `resume --from-checkpoint N`).

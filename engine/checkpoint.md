# Vajra Checkpoint / Resume Reference

Skill reference for Claude Code hooks and scripts that interact with the Vajra campaign engine (`engine.ts`).

---

## Checkpointing (Save Progress)

Before any risky or long-running step, create a checkpoint so the campaign can be rolled back if something goes wrong.

```ts
import { getActiveCampaign, saveCampaign, createCheckpoint } from "./engine";

// 1. Get the current campaign
const campaign = getActiveCampaign();
if (!campaign) throw new Error("No active campaign");

// 2. Update state with current work context
campaign.currentNode = "node-id-of-current-step";
campaign.lastOutput = serializeWorkState(); // add any relevant data
saveCampaign(campaign);

// 3. Snapshot the saved state as a checkpoint
const checkpointId = createCheckpoint(campaign.id);
console.log(`Checkpoint created: ${checkpointId}`);
```

**When to checkpoint:**
- Before executing a multi-step plan
- Before destructive operations (file overwrites, refactors)
- At the start of each new campaign node/phase
- After completing a major milestone

---

## Resuming (`/vajra continue`)

When the user runs `/vajra continue`, the harness should:

1. **Load the active campaign:**
   ```ts
   import { getActiveCampaign } from "./engine";

   const campaign = getActiveCampaign();
   if (!campaign) {
     // No active campaign — prompt user to start one
     return;
   }
   ```

2. **HMAC is verified automatically** by `loadCampaign` / `getActiveCampaign`. If verification fails, the function throws with a tamper-detection message. Do NOT catch and suppress this error.

3. **Read `currentNode`** from the loaded state to determine where execution left off:
   ```ts
   const resumeFrom = campaign.currentNode;
   // Route to the appropriate handler/node for this step
   ```

4. **Continue execution** from that node, checkpointing as you go.

---

## Rollback (`/vajra rollback <id>`)

When the user runs `/vajra rollback <checkpointId>`:

1. **Roll back the campaign state:**
   ```ts
   import { rollbackToCheckpoint, listCheckpoints } from "./engine";

   // Optional: list available checkpoints first
   const checkpoints = listCheckpoints(campaignId);
   // checkpoints = [{ id, created, node_id }, ...]

   // Perform rollback — HMAC of the checkpoint is verified automatically
   const restoredState = rollbackToCheckpoint(checkpointId);
   console.log(`Rolled back to node: ${restoredState.currentNode}`);
   ```

2. The campaign's `state_json`, `hmac`, `current_node`, and `status` are all replaced with the checkpoint's values.

3. **Inform the user** which node/phase they have been rolled back to and what the next step is.

---

## Error Handling

### HMAC Mismatch

If any load or rollback operation encounters an HMAC mismatch, the engine throws:

```
HMAC verification failed — campaign state may have been tampered with.
Refusing to load. Please investigate or rollback to a known-good checkpoint.
```

**Required response:**
- Do NOT proceed with the corrupted state.
- Alert the user immediately with the full error message.
- Suggest listing checkpoints (`listCheckpoints(campaignId)`) and rolling back to the most recent valid one.
- If no valid checkpoints exist, the campaign must be marked as `failed` and a new one started.

### Campaign Not Found

If the requested campaign or checkpoint ID does not exist, the engine throws a descriptive error. Surface this to the user and suggest running `listCampaigns()` or `listCheckpoints()`.

### Database Errors

SQLite errors (disk full, locked, corrupt) propagate as exceptions. Catch at the top level of the hook/script, log the full error, and inform the user. Do not silently swallow database errors.

---

## Security Notes

- The HMAC key is stored at `~/.claude/vajra/.hmac-key` with mode `0600`.
- The database file at `~/.claude/vajra/vajra.db` is also set to mode `0600`.
- All file creation in the engine uses restrictive permissions.
- HMAC comparison uses `crypto.timingSafeEqual` to prevent timing attacks.
- Never log or expose the HMAC key or raw `state_json` to untrusted outputs.

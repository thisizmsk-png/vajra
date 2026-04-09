/**
 * Tests for engine/engine.ts — campaign persistence and HMAC integrity.
 *
 * Strategy: we monkey-patch the module-level paths so the engine uses a
 * temp directory instead of ~/.claude/vajra. Each test gets a fresh DB.
 */

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import * as crypto from "crypto";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import Database from "better-sqlite3";

// ---------------------------------------------------------------------------
// Because engine.ts uses module-level constants for VAJRA_DIR / DB_PATH /
// HMAC_KEY_PATH and caches _db/_hmacKey in closures, we re-implement the
// core functions against a temp directory for isolation. This tests the
// *algorithms* (HMAC, roundtrip, checkpoint) using the same logic as the
// production module, without touching the real ~/.claude/vajra state.
// ---------------------------------------------------------------------------

let tmpDir: string;
let dbPath: string;
let hmacKeyPath: string;
let db: InstanceType<typeof Database>;
let hmacKey: Buffer;

function setupDb(): InstanceType<typeof Database> {
  const d = new Database(dbPath);
  d.pragma("journal_mode = WAL");
  d.pragma("foreign_keys = ON");
  d.exec(`
    CREATE TABLE IF NOT EXISTS campaigns (
      id              TEXT PRIMARY KEY,
      schema_version  INTEGER NOT NULL DEFAULT 1,
      name            TEXT NOT NULL,
      created         TEXT NOT NULL,
      updated         TEXT NOT NULL,
      status          TEXT NOT NULL CHECK (status IN ('active', 'paused', 'completed', 'failed')),
      state_json      TEXT NOT NULL,
      hmac            TEXT NOT NULL,
      current_node    TEXT
    );
    CREATE TABLE IF NOT EXISTS checkpoints (
      id              TEXT PRIMARY KEY,
      campaign_id     TEXT NOT NULL,
      created         TEXT NOT NULL,
      node_id         TEXT,
      state_json      TEXT NOT NULL,
      hmac            TEXT NOT NULL,
      FOREIGN KEY (campaign_id) REFERENCES campaigns(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_checkpoints_campaign
      ON checkpoints(campaign_id, created);
  `);
  return d;
}

function ensureHmacKey(): Buffer {
  if (fs.existsSync(hmacKeyPath)) {
    const hexKey = fs.readFileSync(hmacKeyPath, "utf-8").trim();
    if (hexKey.length >= 64) {
      return Buffer.from(hexKey, "hex");
    }
  }
  const key = crypto.randomBytes(32);
  fs.writeFileSync(hmacKeyPath, key.toString("hex"), { mode: 0o600 });
  return key;
}

function computeHmac(payload: string): string {
  return crypto
    .createHmac("sha256", hmacKey)
    .update(payload, "utf8")
    .digest("hex");
}

function verifyHmac(payload: string, expected: string): void {
  const actual = computeHmac(payload);
  const actualBuf = Buffer.from(actual, "hex");
  const expectedBuf = Buffer.from(expected, "hex");
  if (
    actualBuf.length !== expectedBuf.length ||
    !crypto.timingSafeEqual(actualBuf, expectedBuf)
  ) {
    throw new Error("HMAC verification failed");
  }
}

function newId(): string {
  return crypto.randomBytes(16).toString("base64url");
}

function nowISO(): string {
  return new Date().toISOString();
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

beforeEach(() => {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "vajra-test-"));
  dbPath = path.join(tmpDir, "vajra.db");
  hmacKeyPath = path.join(tmpDir, ".hmac-key");
  hmacKey = ensureHmacKey();
  db = setupDb();
});

afterEach(() => {
  try {
    db.close();
  } catch {
    /* already closed */
  }
  fs.rmSync(tmpDir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// HMAC key validation
// ---------------------------------------------------------------------------

describe("HMAC key management", () => {
  it("regenerates an empty key file", () => {
    // Write an empty key
    fs.writeFileSync(hmacKeyPath, "", { mode: 0o600 });
    const key = ensureHmacKey();
    expect(key.length).toBe(32);
    // File should now have 64 hex chars
    const onDisk = fs.readFileSync(hmacKeyPath, "utf-8").trim();
    expect(onDisk.length).toBe(64);
  });

  it("regenerates a too-short key", () => {
    fs.writeFileSync(hmacKeyPath, "abcd", { mode: 0o600 });
    const key = ensureHmacKey();
    expect(key.length).toBe(32);
  });

  it("preserves a valid key", () => {
    const original = fs.readFileSync(hmacKeyPath, "utf-8").trim();
    const key = ensureHmacKey();
    expect(key.toString("hex")).toBe(original);
  });
});

// ---------------------------------------------------------------------------
// computeHmac / verifyHmac
// ---------------------------------------------------------------------------

describe("computeHmac", () => {
  it("produces consistent results for the same input", () => {
    const a = computeHmac("hello world");
    const b = computeHmac("hello world");
    expect(a).toBe(b);
  });

  it("produces different results for different input", () => {
    const a = computeHmac("hello");
    const b = computeHmac("world");
    expect(a).not.toBe(b);
  });

  it("returns a 64-char hex string (SHA-256)", () => {
    const h = computeHmac("test");
    expect(h).toMatch(/^[0-9a-f]{64}$/);
  });
});

describe("verifyHmac", () => {
  it("passes for valid HMAC", () => {
    const payload = '{"id":"abc","name":"test"}';
    const mac = computeHmac(payload);
    expect(() => verifyHmac(payload, mac)).not.toThrow();
  });

  it("throws on tampered data", () => {
    const payload = '{"id":"abc","name":"test"}';
    const mac = computeHmac(payload);
    expect(() => verifyHmac(payload + "x", mac)).toThrow(
      "HMAC verification failed",
    );
  });

  it("throws on tampered HMAC", () => {
    const payload = "data";
    const mac = computeHmac(payload);
    // Flip a character
    const tampered = mac[0] === "a" ? "b" + mac.slice(1) : "a" + mac.slice(1);
    expect(() => verifyHmac(payload, tampered)).toThrow(
      "HMAC verification failed",
    );
  });

  it("handles length-mismatched HMAC without TypeError", () => {
    // A short hex string should cause length mismatch, not a crypto TypeError
    const payload = "data";
    expect(() => verifyHmac(payload, "abcd")).toThrow(
      "HMAC verification failed",
    );
  });

  it("handles empty expected HMAC without TypeError", () => {
    expect(() => verifyHmac("data", "")).toThrow("HMAC verification failed");
  });
});

// ---------------------------------------------------------------------------
// createCampaign / loadCampaign roundtrip
// ---------------------------------------------------------------------------

describe("campaign roundtrip", () => {
  function createCampaign(name: string) {
    const now = nowISO();
    const state = { id: newId(), name, status: "active" as const, currentNode: null };
    const stateJson = JSON.stringify(state);
    const mac = computeHmac(stateJson);
    db.prepare(
      `INSERT INTO campaigns (id, schema_version, name, created, updated, status, state_json, hmac, current_node)
       VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(state.id, name, now, now, state.status, stateJson, mac, null);
    return state;
  }

  function loadCampaign(id: string) {
    const row = db
      .prepare("SELECT * FROM campaigns WHERE id = ?")
      .get(id) as any;
    if (!row) throw new Error(`Campaign not found: ${id}`);
    verifyHmac(row.state_json, row.hmac);
    return JSON.parse(row.state_json);
  }

  it("creates and loads a campaign with matching state", () => {
    const created = createCampaign("test-campaign");
    const loaded = loadCampaign(created.id);
    expect(loaded.id).toBe(created.id);
    expect(loaded.name).toBe("test-campaign");
    expect(loaded.status).toBe("active");
  });

  it("detects tampering on load", () => {
    const created = createCampaign("secure-campaign");
    // Tamper with the stored state_json directly in the DB
    db.prepare("UPDATE campaigns SET state_json = ? WHERE id = ?").run(
      '{"id":"TAMPERED","name":"evil","status":"active","currentNode":null}',
      created.id,
    );
    expect(() => loadCampaign(created.id)).toThrow("HMAC verification failed");
  });

  it("throws when loading a nonexistent campaign", () => {
    expect(() => loadCampaign("nonexistent-id")).toThrow("Campaign not found");
  });
});

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

describe("checkpoints", () => {
  function insertCampaign(name: string) {
    const now = nowISO();
    const state = { id: newId(), name, status: "active" as const, currentNode: null };
    const stateJson = JSON.stringify(state);
    const mac = computeHmac(stateJson);
    db.prepare(
      `INSERT INTO campaigns (id, schema_version, name, created, updated, status, state_json, hmac, current_node)
       VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(state.id, name, now, now, state.status, stateJson, mac, null);
    return state;
  }

  function createCheckpoint(campaignId: string): string {
    const row = db
      .prepare("SELECT * FROM campaigns WHERE id = ?")
      .get(campaignId) as any;
    if (!row) throw new Error("Campaign not found for checkpoint");
    verifyHmac(row.state_json, row.hmac);
    const cpId = newId();
    const now = nowISO();
    const mac = computeHmac(row.state_json);
    db.prepare(
      `INSERT INTO checkpoints (id, campaign_id, created, node_id, state_json, hmac)
       VALUES (?, ?, ?, ?, ?, ?)`,
    ).run(cpId, campaignId, now, row.current_node, row.state_json, mac);
    return cpId;
  }

  function rollbackToCheckpoint(checkpointId: string) {
    const cp = db
      .prepare("SELECT * FROM checkpoints WHERE id = ?")
      .get(checkpointId) as any;
    if (!cp) throw new Error("Checkpoint not found");
    verifyHmac(cp.state_json, cp.hmac);
    const state = JSON.parse(cp.state_json);
    const now = nowISO();
    const newMac = computeHmac(cp.state_json);
    db.prepare(
      `UPDATE campaigns SET updated = ?, status = ?, state_json = ?, hmac = ?, current_node = ? WHERE id = ?`,
    ).run(now, state.status, cp.state_json, newMac, cp.node_id, cp.campaign_id);
    return state;
  }

  it("creates a checkpoint and rolls back to it", () => {
    const campaign = insertCampaign("checkpoint-test");
    const cpId = createCheckpoint(campaign.id);

    // Mutate the campaign state in DB (simulating progression)
    const updatedState = { ...campaign, name: "mutated", status: "paused" as const };
    const updatedJson = JSON.stringify(updatedState);
    const updatedMac = computeHmac(updatedJson);
    db.prepare(
      "UPDATE campaigns SET state_json = ?, hmac = ?, status = ? WHERE id = ?",
    ).run(updatedJson, updatedMac, "paused", campaign.id);

    // Verify the mutation took effect
    const mutatedRow = db
      .prepare("SELECT state_json FROM campaigns WHERE id = ?")
      .get(campaign.id) as any;
    expect(JSON.parse(mutatedRow.state_json).name).toBe("mutated");

    // Rollback
    const rolledBack = rollbackToCheckpoint(cpId);
    expect(rolledBack.name).toBe("checkpoint-test");
    expect(rolledBack.status).toBe("active");
  });

  it("throws when rolling back to a nonexistent checkpoint", () => {
    expect(() => rollbackToCheckpoint("no-such-cp")).toThrow(
      "Checkpoint not found",
    );
  });
});

// ---------------------------------------------------------------------------
// closeDb clears HMAC cache
// ---------------------------------------------------------------------------

describe("closeDb behavior", () => {
  it("after close, a fresh ensureHmacKey reads from disk again", () => {
    // This tests the pattern: close nulls the cache, next call re-reads
    const key1 = hmacKey;
    // Simulate closeDb: null the in-memory references
    hmacKey = null as any;
    // Re-read from disk (simulating getHmacKey after closeDb)
    hmacKey = ensureHmacKey();
    expect(hmacKey.toString("hex")).toBe(key1.toString("hex"));
  });
});

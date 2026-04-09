/**
 * Vajra Campaign Engine
 *
 * SQLite-backed campaign state management for the Vajra agent harness.
 * Provides HMAC-verified persistence, checkpointing, and rollback.
 *
 * Usage: import functions from this module in Claude Code hooks/scripts.
 * Requires: better-sqlite3, Node.js 18+
 */

import Database, { type Database as DatabaseType } from "better-sqlite3";
import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const VAJRA_DIR = path.join(
  process.env.HOME ?? process.env.USERPROFILE ?? "~",
  ".claude",
  "vajra",
);
const DB_PATH = path.join(VAJRA_DIR, "vajra.db");
const HMAC_KEY_PATH = path.join(VAJRA_DIR, ".hmac-key");

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CampaignRow {
  id: string;
  schema_version: number;
  name: string;
  created: string;
  updated: string;
  status: "active" | "paused" | "completed" | "failed";
  state_json: string;
  hmac: string;
  current_node: string | null;
}

export interface CheckpointRow {
  id: string;
  campaign_id: string;
  created: string;
  node_id: string | null;
  state_json: string;
  hmac: string;
}

export interface CampaignState {
  id: string;
  name: string;
  status: "active" | "paused" | "completed" | "failed";
  currentNode: string | null;
  [key: string]: unknown;
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

let _db: DatabaseType | null = null;
let _hmacKey: Buffer | null = null;

function ensureDir(dir: string): void {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
}

function secureWrite(filePath: string, data: string | Buffer): void {
  fs.writeFileSync(filePath, data, { mode: 0o600 });
}

/**
 * Load or generate the HMAC signing key.
 */
function getHmacKey(): Buffer {
  if (_hmacKey) return _hmacKey;

  ensureDir(VAJRA_DIR);

  if (fs.existsSync(HMAC_KEY_PATH)) {
    _hmacKey = fs.readFileSync(HMAC_KEY_PATH);
  } else {
    _hmacKey = crypto.randomBytes(32);
    secureWrite(HMAC_KEY_PATH, _hmacKey);
  }

  return _hmacKey;
}

/**
 * Compute HMAC-SHA256 for a given payload string.
 */
function computeHmac(payload: string): string {
  return crypto
    .createHmac("sha256", getHmacKey())
    .update(payload, "utf8")
    .digest("hex");
}

/**
 * Verify HMAC and throw on mismatch.
 */
function verifyHmac(payload: string, expected: string): void {
  const actual = computeHmac(payload);
  if (!crypto.timingSafeEqual(Buffer.from(actual, "hex"), Buffer.from(expected, "hex"))) {
    throw new Error(
      "HMAC verification failed — campaign state may have been tampered with. " +
        "Refusing to load. Please investigate or rollback to a known-good checkpoint.",
    );
  }
}

/**
 * Generate a compact random ID (URL-safe, 16 bytes → 22 chars base64url).
 */
function newId(): string {
  return crypto.randomBytes(16).toString("base64url");
}

function nowISO(): string {
  return new Date().toISOString();
}

// ---------------------------------------------------------------------------
// Database initialisation
// ---------------------------------------------------------------------------

/**
 * Open (or create) the SQLite database and ensure schema is up to date.
 */
export function getDb(): DatabaseType {
  if (_db) return _db;

  ensureDir(VAJRA_DIR);

  _db = new Database(DB_PATH);

  // Harden: WAL mode for concurrent reads, strict foreign keys.
  _db.pragma("journal_mode = WAL");
  _db.pragma("foreign_keys = ON");

  _db.exec(`
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

    CREATE INDEX IF NOT EXISTS idx_campaigns_status
      ON campaigns(status);
  `);

  // Restrict DB file permissions (best-effort on non-POSIX).
  try {
    fs.chmodSync(DB_PATH, 0o600);
  } catch {
    // Ignore on platforms that don't support chmod.
  }

  return _db;
}

/**
 * Close the database connection. Useful for cleanup in scripts.
 */
export function closeDb(): void {
  if (_db) {
    _db.close();
    _db = null;
  }
}

// ---------------------------------------------------------------------------
// Campaign CRUD
// ---------------------------------------------------------------------------

/**
 * Create a new campaign with the given name. Returns the initial CampaignState.
 */
export function createCampaign(name: string): CampaignState {
  const db = getDb();
  const now = nowISO();

  const state: CampaignState = {
    id: newId(),
    name,
    status: "active",
    currentNode: null,
  };

  const stateJson = JSON.stringify(state);
  const hmac = computeHmac(stateJson);

  db.prepare(
    `INSERT INTO campaigns (id, schema_version, name, created, updated, status, state_json, hmac, current_node)
     VALUES (?, 1, ?, ?, ?, ?, ?, ?, ?)`,
  ).run(state.id, name, now, now, state.status, stateJson, hmac, state.currentNode);

  return state;
}

/**
 * Load a campaign by ID. Verifies HMAC before returning.
 * Throws if the campaign does not exist or HMAC fails.
 */
export function loadCampaign(id: string): CampaignState {
  const db = getDb();

  const row = db.prepare("SELECT * FROM campaigns WHERE id = ?").get(id) as
    | CampaignRow
    | undefined;

  if (!row) {
    throw new Error(`Campaign not found: ${id}`);
  }

  verifyHmac(row.state_json, row.hmac);

  return JSON.parse(row.state_json) as CampaignState;
}

/**
 * Persist updated campaign state. Recomputes HMAC.
 */
export function saveCampaign(state: CampaignState): void {
  const db = getDb();
  const now = nowISO();

  const stateJson = JSON.stringify(state);
  const hmac = computeHmac(stateJson);

  const result = db
    .prepare(
      `UPDATE campaigns
          SET updated      = ?,
              status       = ?,
              state_json   = ?,
              hmac         = ?,
              current_node = ?
        WHERE id = ?`,
    )
    .run(now, state.status, stateJson, hmac, state.currentNode ?? null, state.id);

  if (result.changes === 0) {
    throw new Error(`Campaign not found for save: ${state.id}`);
  }
}

/**
 * Return the currently active campaign, or null if none.
 * Verifies HMAC before returning.
 */
export function getActiveCampaign(): CampaignState | null {
  const db = getDb();

  const row = db
    .prepare("SELECT * FROM campaigns WHERE status = 'active' ORDER BY updated DESC LIMIT 1")
    .get() as CampaignRow | undefined;

  if (!row) return null;

  verifyHmac(row.state_json, row.hmac);

  return JSON.parse(row.state_json) as CampaignState;
}

/**
 * List all campaigns (summary only — does not deserialise state_json).
 */
export function listCampaigns(): Array<{
  id: string;
  name: string;
  status: string;
  created: string;
  updated: string;
  current_node: string | null;
}> {
  const db = getDb();

  return db
    .prepare(
      "SELECT id, name, status, created, updated, current_node FROM campaigns ORDER BY updated DESC",
    )
    .all() as Array<{
    id: string;
    name: string;
    status: string;
    created: string;
    updated: string;
    current_node: string | null;
  }>;
}

// ---------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------

/**
 * Create a checkpoint from the current campaign state.
 * Returns the checkpoint ID.
 */
export function createCheckpoint(campaignId: string): string {
  const db = getDb();

  const row = db.prepare("SELECT * FROM campaigns WHERE id = ?").get(campaignId) as
    | CampaignRow
    | undefined;

  if (!row) {
    throw new Error(`Campaign not found for checkpoint: ${campaignId}`);
  }

  // Verify current state is untampered before snapshotting.
  verifyHmac(row.state_json, row.hmac);

  const checkpointId = newId();
  const now = nowISO();
  const hmac = computeHmac(row.state_json);

  db.prepare(
    `INSERT INTO checkpoints (id, campaign_id, created, node_id, state_json, hmac)
     VALUES (?, ?, ?, ?, ?, ?)`,
  ).run(checkpointId, campaignId, now, row.current_node, row.state_json, hmac);

  return checkpointId;
}

/**
 * Rollback a campaign to a previous checkpoint.
 * Verifies the checkpoint HMAC, then replaces the campaign state.
 */
export function rollbackToCheckpoint(checkpointId: string): CampaignState {
  const db = getDb();

  const cp = db.prepare("SELECT * FROM checkpoints WHERE id = ?").get(checkpointId) as
    | CheckpointRow
    | undefined;

  if (!cp) {
    throw new Error(`Checkpoint not found: ${checkpointId}`);
  }

  verifyHmac(cp.state_json, cp.hmac);

  const state = JSON.parse(cp.state_json) as CampaignState;
  const now = nowISO();
  const newHmac = computeHmac(cp.state_json);

  const result = db
    .prepare(
      `UPDATE campaigns
          SET updated      = ?,
              status       = ?,
              state_json   = ?,
              hmac         = ?,
              current_node = ?
        WHERE id = ?`,
    )
    .run(now, state.status, cp.state_json, newHmac, cp.node_id, cp.campaign_id);

  if (result.changes === 0) {
    throw new Error(`Campaign not found during rollback: ${cp.campaign_id}`);
  }

  return state;
}

/**
 * List checkpoints for a campaign, newest first.
 */
export function listCheckpoints(
  campaignId: string,
): Array<{ id: string; created: string; node_id: string | null }> {
  const db = getDb();

  return db
    .prepare(
      "SELECT id, created, node_id FROM checkpoints WHERE campaign_id = ? ORDER BY created DESC",
    )
    .all(campaignId) as Array<{ id: string; created: string; node_id: string | null }>;
}

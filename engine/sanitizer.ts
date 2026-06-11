/**
 * Prompt injection sanitizer for Vajra agent harness.
 *
 * All untrusted content (memory files, external data, user-supplied context)
 * MUST pass through sanitize() before being loaded into the prompt context.
 */

import { readFileSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SanitizationConfig {
  security: {
    sanitization: {
      stripPatterns: string[];
      maxContentLength: number;
      escapeControlChars: boolean;
      wrapInTags: boolean;
      tagName: string;
    };
  };
}

// ---------------------------------------------------------------------------
// Config loader (cached)
// ---------------------------------------------------------------------------

let _cachedConfig: SanitizationConfig | null = null;

/**
 * Load the default security/sanitization configuration from
 * `config/default.json` (resolved relative to this file's directory).
 * Result is cached after first load.
 */
export function loadConfig(): SanitizationConfig {
  if (_cachedConfig) return _cachedConfig;

  // ESM-compatible __dirname equivalent
  const __filename_compat = typeof __filename !== "undefined"
    ? __filename
    : fileURLToPath(import.meta.url);
  const configPath = resolve(dirname(__filename_compat), "..", "config", "default.json");
  const raw = readFileSync(configPath, "utf-8");
  _cachedConfig = JSON.parse(raw) as SanitizationConfig;
  return _cachedConfig;
}

// ---------------------------------------------------------------------------
// Sanitizer
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Invisible / smuggling character classes
// ---------------------------------------------------------------------------
//
// ASCII smuggling and homoglyph evasion hide live instructions in codepoints
// that are invisible to humans and to naive regex filters but are still read by
// the model (see the Unicode Tags block, variation selectors, bidi overrides).
// These MUST be removed before pattern matching, otherwise an attacker can
// interleave them through a trigger phrase to slip past the strip list.
const INVISIBLE_CHARS = new RegExp(
  "[" +
    "\\u00AD" + // soft hyphen
    "\\u200B-\\u200F" + // zero-width space/joiners + LRM/RLM
    "\\u202A-\\u202E" + // bidi embeddings/overrides
    "\\u2060-\\u2064" + // word joiner + invisible math operators
    "\\u2066-\\u2069" + // bidi isolates
    "\\uFEFF" + // BOM / zero-width no-break space
    "\\uFFF9-\\uFFFB" + // interlinear annotation anchors
    "\\uFE00-\\uFE0F" + // variation selectors 1-16
    "]",
  "g",
);
// Astral-plane smuggling: Unicode Tags block (U+E0000\u2013E007F) and the
// supplementary variation selectors (U+E0100\u2013E01EF). Matched on codepoints.
const INVISIBLE_ASTRAL = /[\u{E0000}-\u{E007F}\u{E0100}-\u{E01EF}]/gu;

/**
 * Sanitize untrusted content before injecting it into prompt context.
 *
 * 0. NFKC-normalize and strip invisible/smuggling codepoints.
 * 1. Strip known injection patterns (from config) to a fixed point.
 * 2. Escape control characters.
 * 3. Truncate to max allowed length.
 * 4. Wrap in <untrusted-data> tags with source attribution.
 *
 * @param content - Raw content string to sanitize
 * @param source  - Human-readable label for the content origin
 *                  (e.g. "memory/topics/hardware.md", "user-upload")
 * @returns Sanitized and wrapped string safe for context injection
 */
export function sanitize(content: string, source: string): string {
  const config = loadConfig();
  const { stripPatterns, maxContentLength, escapeControlChars } =
    config.security.sanitization;

  let sanitized = content;

  // --- 0. Normalize unicode to prevent pattern evasion ---------------------
  // NFKC folds compatibility/fullwidth/ligature forms so homoglyph variants of
  // a trigger phrase normalize to their canonical (matchable) form.
  try {
    sanitized = sanitized.normalize("NFKC");
  } catch {
    // Malformed surrogate sequences \u2014 fall through with the raw string.
  }
  // Strip invisible/smuggling characters (BMP + astral).
  sanitized = sanitized.replace(INVISIBLE_CHARS, "");
  sanitized = sanitized.replace(INVISIBLE_ASTRAL, "");
  // Collapse multiple whitespace to single space (preserving newlines)
  sanitized = sanitized.replace(/[ \t]+/g, " ");

  // --- 1. Strip known injection patterns (to a fixed point) ----------------
  // A SINGLE pass is unsafe: removing an inner occurrence can reassemble an
  // outer one (e.g. "ignore previous ignore previous instructions instructions"
  // -> "ignore previous instructions"). Iterate until stable, then re-collapse
  // any whitespace the removals left behind. Capped to bound pathological input.
  const compiled = stripPatterns.map((p) => new RegExp(p, "gi"));
  const tagRe = /<\/?untrusted-data[^>]*>/gi;
  const MAX_PASSES = 16;
  for (let pass = 0; pass < MAX_PASSES; pass++) {
    const before = sanitized;
    for (const regex of compiled) {
      sanitized = sanitized.replace(regex, "");
    }
    // --- 1b. Strip wrapper tag from content to prevent tag breakout --------
    sanitized = sanitized.replace(tagRe, "");
    // Re-collapse so reassembly residue ("ignore  previous") cannot survive.
    sanitized = sanitized.replace(/[ \t]+/g, " ");
    if (sanitized === before) break;
  }

  // --- 2. Escape control characters ----------------------------------------
  if (escapeControlChars) {
    sanitized = sanitized
      // Remove null bytes
      .replace(/\0/g, "")
      // Escape backspace, form-feed, vertical tab
      .replace(/[\b\f\v]/g, "")
      // Escape non-printable ASCII control chars (0x01-0x08, 0x0E-0x1F, 0x7F)
      .replace(/[\x01-\x08\x0E-\x1F\x7F]/g, "")
      // Normalize line endings
      .replace(/\r\n/g, "\n")
      .replace(/\r/g, "\n");
  }

  // --- 3. Truncate to max allowed length -----------------------------------
  if (maxContentLength > 0 && sanitized.length > maxContentLength) {
    sanitized = sanitized.slice(0, maxContentLength) + "\n[TRUNCATED]";
  }

  // --- 4. Wrap in untrusted-data tags --------------------------------------
  const escapedSource = source
    .replace(/&/g, "&amp;")
    .replace(/"/g, "&quot;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");

  return `<untrusted-data source="${escapedSource}">\n${sanitized}\n</untrusted-data>`;
}

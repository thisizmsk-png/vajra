/**
 * Prompt injection sanitizer for Vajra agent harness.
 *
 * All untrusted content (memory files, external data, user-supplied context)
 * MUST pass through sanitize() before being loaded into the prompt context.
 */

import { readFileSync } from "fs";
import { resolve, dirname } from "path";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SanitizationConfig {
  security: {
    sanitization: {
      stripPatterns: string[];
      maxContentLength: number;
      escapeControlChars: boolean;
    };
  };
}

// ---------------------------------------------------------------------------
// Config loader
// ---------------------------------------------------------------------------

/**
 * Load the default security/sanitization configuration from
 * `config/default.json` (resolved relative to this file's directory).
 */
export function loadConfig(): SanitizationConfig {
  const configPath = resolve(dirname(__filename), "..", "config", "default.json");
  const raw = readFileSync(configPath, "utf-8");
  return JSON.parse(raw) as SanitizationConfig;
}

// ---------------------------------------------------------------------------
// Sanitizer
// ---------------------------------------------------------------------------

/**
 * Sanitize untrusted content before injecting it into prompt context.
 *
 * 1. Strip known injection patterns (from config).
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

  // --- 1. Strip known injection patterns -----------------------------------
  for (const pattern of stripPatterns) {
    const regex = new RegExp(pattern, "gi");
    sanitized = sanitized.replace(regex, "");
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
  const escapedSource = source.replace(/"/g, "&quot;").replace(/</g, "&lt;");

  return `<untrusted-data source="${escapedSource}">\n${sanitized}\n</untrusted-data>`;
}

#!/usr/bin/env node
"use strict";

/**
 * sarif-convert.js — Converts a Vajra JSON security report to SARIF 2.1.0 format.
 *
 * Usage:
 *   node sarif-convert.js <report.json>
 *   cat report.json | node sarif-convert.js
 *
 * Output: SARIF JSON to stdout.
 */

const fs = require("fs");
const path = require("path");

const SARIF_SCHEMA =
  "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json";
const SARIF_VERSION = "2.1.0";
const TOOL_NAME = "Vajra";
const TOOL_VERSION = "0.2.0";
const TOOL_URI = "https://github.com/vajra-security/vajra";

/**
 * Map Vajra severity to SARIF level.
 */
function toSarifLevel(severity, verdict) {
  if (verdict === "fail") return "error";
  if (verdict === "warning") return "warning";
  if (severity === "critical" && verdict !== "pass") return "error";
  if (severity === "high" && verdict !== "pass") return "warning";
  return "note";
}

/**
 * Sanitize a behavior name into a valid SARIF ruleId.
 */
function toRuleId(name) {
  return name
    .replace(/[^a-zA-Z0-9_.-]/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

/**
 * Build a SARIF rule descriptor from a Vajra behavior result.
 */
function buildRule(behavior) {
  const ruleId = toRuleId(behavior.name);
  const rule = {
    id: ruleId,
    name: behavior.name,
    shortDescription: {
      text: `Security behavior: ${behavior.name}`,
    },
    fullDescription: {
      text: behavior.reasoning || `Evaluation of ${behavior.name} behavior.`,
    },
    defaultConfiguration: {
      level: toSarifLevel(behavior.severity, behavior.verdict),
    },
    properties: {
      severity: behavior.severity,
      score: behavior.score,
      verdict: behavior.verdict,
    },
  };

  if (
    behavior.complianceMapping &&
    (behavior.complianceMapping.owaspLlmTop10 ||
      behavior.complianceMapping.nistAi100)
  ) {
    rule.properties.complianceMapping = behavior.complianceMapping;
  }

  return rule;
}

/**
 * Build a SARIF result from a Vajra behavior result.
 */
function buildResult(behavior) {
  const ruleId = toRuleId(behavior.name);
  const level = toSarifLevel(behavior.severity, behavior.verdict);

  const evidenceText =
    behavior.evidence && behavior.evidence.length > 0
      ? behavior.evidence.join("\n---\n")
      : "No evidence collected.";

  const mitigationText =
    behavior.mitigations && behavior.mitigations.length > 0
      ? "\n\nMitigations:\n" +
        behavior.mitigations.map((m) => `- ${m}`).join("\n")
      : "";

  const messageText = `[${behavior.verdict.toUpperCase()}] ${behavior.name} (${behavior.score}/100)\n\n${behavior.reasoning || ""}${mitigationText}\n\nEvidence:\n${evidenceText}`;

  const result = {
    ruleId,
    level,
    message: {
      text: messageText,
    },
    properties: {
      score: behavior.score,
      verdict: behavior.verdict,
      severity: behavior.severity,
    },
  };

  return result;
}

/**
 * Convert a Vajra JSON report to SARIF 2.1.0.
 */
function convertToSarif(vajraReport) {
  const rules = [];
  const results = [];

  const behaviors = vajraReport.behaviors || [];
  for (const behavior of behaviors) {
    rules.push(buildRule(behavior));
    results.push(buildResult(behavior));
  }

  const sarif = {
    $schema: SARIF_SCHEMA,
    version: SARIF_VERSION,
    runs: [
      {
        tool: {
          driver: {
            name: TOOL_NAME,
            version: TOOL_VERSION,
            informationUri: TOOL_URI,
            rules,
          },
        },
        results,
        invocations: [
          {
            executionSuccessful: true,
            startTimeUtc:
              vajraReport.generatedAt ||
              vajraReport.timestamp ||
              new Date().toISOString(),
            properties: {
              campaignId: vajraReport.campaignId || "unknown",
              target: vajraReport.target || "unknown",
              overallScore: vajraReport.overallScore,
              overallVerdict: vajraReport.overallVerdict,
            },
          },
        ],
      },
    ],
  };

  return sarif;
}

/**
 * Read input from file argument or stdin.
 */
function readInput() {
  return new Promise((resolve, reject) => {
    const fileArg = process.argv[2];

    if (fileArg) {
      const filePath = path.resolve(fileArg);
      try {
        const data = fs.readFileSync(filePath, "utf8");
        resolve(data);
      } catch (err) {
        reject(new Error(`Failed to read file ${filePath}: ${err.message}`));
      }
      return;
    }

    // Read from stdin
    let data = "";
    process.stdin.setEncoding("utf8");

    process.stdin.on("readable", () => {
      let chunk;
      while ((chunk = process.stdin.read()) !== null) {
        data += chunk;
      }
    });

    process.stdin.on("end", () => {
      if (!data.trim()) {
        reject(
          new Error(
            "No input received. Provide a file argument or pipe JSON to stdin.",
          ),
        );
      } else {
        resolve(data);
      }
    });

    process.stdin.on("error", reject);
  });
}

async function main() {
  try {
    const input = await readInput();
    const vajraReport = JSON.parse(input);
    const sarif = convertToSarif(vajraReport);
    process.stdout.write(JSON.stringify(sarif, null, 2) + "\n");
  } catch (err) {
    process.stderr.write(`sarif-convert: ${err.message}\n`);
    process.exit(1);
  }
}

main();

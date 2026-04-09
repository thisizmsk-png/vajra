# Red-Team Report Generation

## When to Use

After the scorer produces results for all 6 security behaviors, generate a report in the user's chosen format.

## Input

Scored results matching this schema:

```json
{
  "id": "UUIDv4",
  "target": "skill-name",
  "timestamp": "ISO8601",
  "overallScore": 0-100,
  "overallVerdict": "pass|warning|fail",
  "behaviors": [
    {
      "name": "behavior-name",
      "severity": "critical|high|medium",
      "score": 0-100,
      "verdict": "pass|warning|fail",
      "evidence": [...],
      "mitigation": "recommended fix"
    }
  ],
  "attacks": [...]
}
```

## Output Formats

### HTML Report

1. Read `scripts/report-template.html`
2. Replace template placeholders:
   - `{{TARGET}}` → target skill name
   - `{{DATE}}` → current ISO8601 date
   - `{{OVERALL_SCORE}}` → numeric score
   - `{{OVERALL_VERDICT}}` → pass/warning/fail
   - `{{BEHAVIOR_ROWS}}` → HTML table rows for each behavior
   - `{{DETAIL_SECTIONS}}` → expandable sections with evidence
3. Write to `~/.claude/vajra/campaigns/{id}/reports/report.html`

### JSON Report

1. Serialize the full SecurityReport object
2. Write to `~/.claude/vajra/campaigns/{id}/reports/report.json`

### SARIF Report

1. Pass JSON report through `scripts/sarif-convert.js`:
   ```bash
   node scripts/sarif-convert.js report.json > report.sarif
   ```
2. Output is SARIF 2.1.0 compatible with GitHub Code Scanning
3. Write to `~/.claude/vajra/campaigns/{id}/reports/report.sarif`

## Encryption

Per config (`security.encryption.enabled`):
- If enabled (default): encrypt report files with AES-256-GCM before writing
- Security reports are in `security.encryption.scope`, so they are always encrypted when encryption is on
- Key derived from HMAC key at `~/.claude/vajra/.hmac-key`

## Directory Setup

Before writing:
1. Create `~/.claude/vajra/campaigns/{id}/reports/` if it doesn't exist
2. Set permissions to `0700` on the reports directory
3. Set `0600` on each report file

## User Summary

After generating, print to user:

```
Red-team complete: X/6 behaviors passed.
Overall score: Y/100 (verdict)

Findings:
  - [CRITICAL] prompt-injection-resistance: PASS (95/100)
  - [CRITICAL] sandbox-isolation: WARNING (72/100)
  - [HIGH] tool-policy-enforcement: PASS (88/100)
  - [HIGH] session-boundary-integrity: PASS (91/100)
  - [MEDIUM] configuration-drift-detection: FAIL (35/100)
  - [MEDIUM] protocol-security: PASS (85/100)

Reports saved to: ~/.claude/vajra/campaigns/{id}/reports/
  - report.html (human-readable)
  - report.json (machine-parseable)
  - report.sarif (GitHub Code Scanning)
```

## Promptfoo Enhancement

If Promptfoo results are available (from scorer), include them in the report:
- Add OWASP LLM Top 10 mapping
- Add NIST AI RMF compliance status
- Add MITRE ATLAS technique references
- Store raw Promptfoo output in `promptfooResults` field of JSON report

---
name: codebase-health
description: Automated codebase health scanning — dead code, complexity, TODO/FIXME, dependency age, test coverage gaps. Activate for health check, code quality, cleanup, refactor, dead code, complexity tasks.
user-invocable: true
---

# Codebase Health Scanner

## When to Activate
- User mentions: health check, code quality, cleanup, refactor, dead code, complexity, tech debt scan
- Starting work on a codebase after a break
- Pre-release quality gate
- Quarterly codebase hygiene

## Scan Checklist (Run These)

### 1. Dead Code Detection
```bash
# Python: find unused imports and variables
ruff check --select F401,F841 .

# TypeScript: find unused exports
npx ts-prune

# Find unreachable functions (grep for definitions never called)
grep -rn "def " src/ | while read line; do
  func=$(echo "$line" | sed 's/.*def \([a-zA-Z_]*\).*/\1/')
  count=$(grep -rn "$func" src/ | wc -l)
  [ "$count" -le 1 ] && echo "UNUSED: $line"
done
```

### 2. Complexity Hotspots
```bash
# Python: cyclomatic complexity
ruff check --select C901 . --statistics

# Count functions over 50 lines
grep -c "def \|function " src/**/*.{py,ts,tsx} | awk -F: '$2 > 50'
```
**Rule**: Any function with cyclomatic complexity >10 or >50 lines is a refactor candidate.

### 3. TODO/FIXME/HACK Inventory
```bash
grep -rn "TODO\|FIXME\|HACK\|XXX\|WORKAROUND" src/ --include="*.py" --include="*.ts" --include="*.tsx"
```
**Action**: Each one is a micro-decision — fix it, delete it, or promote it to an issue.

### 4. Dependency Health
```bash
# Python: check for outdated packages
pip list --outdated

# Node: check for outdated + vulnerabilities
npm outdated && npm audit

# Check for known vulnerabilities
pip-audit  # Python
npm audit   # Node
```

### 5. Test Coverage Gaps
```bash
# Python
pytest --cov=src --cov-report=term-missing | grep "MISS"

# Node
npx jest --coverage --coverageReporters=text | grep "Uncovered"
```
**Focus on**: untested error paths, edge cases, integration points.

### 6. File Size / Module Boundaries
```bash
# Find files over 500 lines (splitting candidates)
find src/ -name "*.py" -o -name "*.ts" | xargs wc -l | sort -rn | head -20
```
**Rule**: Files over 500 lines usually do too many things.

## Health Report Template
```
## Codebase Health Report — [Date]

### Summary
| Metric | Value | Status |
|--------|-------|--------|
| Dead code items | X | 🟢/🟡/🔴 |
| High complexity functions | X | 🟢/🟡/🔴 |
| TODO/FIXME count | X | 🟢/🟡/🔴 |
| Outdated dependencies | X | 🟢/🟡/🔴 |
| Known vulnerabilities | X | 🟢/🟡/🔴 |
| Test coverage | X% | 🟢/🟡/🔴 |
| Files >500 lines | X | 🟢/🟡/🔴 |

### Top 5 Action Items
1. [Most impactful fix]
2. ...
```

## Guardrails
- NEVER refactor without tests covering the changed code
- NEVER update all dependencies at once (one at a time, test between)
- Fix vulnerabilities first, dead code second, complexity third
- This is a SCAN, not a rewrite. Flag issues, don't fix everything in one session.

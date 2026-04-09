---
name: docs-validator
description: Validates documentation against actual code — finds stale docs, broken references, outdated examples. Activate for docs check, stale docs, documentation audit, verify docs tasks.
user-invocable: true
---

# Documentation Validator

## When to Activate
- User mentions: docs check, stale docs, documentation audit, verify docs, docs out of date
- Before a release (ensure docs match current code)
- After significant refactoring
- Periodic maintenance

## What This Skill Does
Validates EXISTING documentation against ACTUAL code. Does NOT generate new docs.

## Validation Checklist

### 1. CLAUDE.md Accuracy
```
For each command listed in CLAUDE.md:
  - Does the file/script referenced still exist?
  - Does the command actually work when run?
  - Are the described arguments still valid?

For each pipeline stage described:
  - Does the corresponding script still exist?
  - Are the input/output file paths still correct?
  - Are the described config fields still in the YAML?
```

### 2. Code Reference Validation
```bash
# Extract all file paths mentioned in *.md files
grep -roh '[a-zA-Z_/]*\.\(py\|ts\|tsx\|js\|yaml\|json\)' docs/ README.md CLAUDE.md | sort -u | while read path; do
  [ ! -f "$path" ] && echo "BROKEN REF: $path"
done

# Extract all function names mentioned in docs
grep -roh '[a-z_]*()' docs/ README.md | sort -u | while read func; do
  name="${func%()*}"
  count=$(grep -rl "def $name\|function $name\|const $name" src/ scripts/ | wc -l)
  [ "$count" -eq 0 ] && echo "STALE FUNCTION REF: $func"
done
```

### 3. Config Documentation Sync
```
For each config field documented:
  - Is it still used in the code?
  - Has its default changed?
  - Are there new fields not in the docs?
```

### 4. Example Validation
```
For each code example in docs:
  - Does it compile/parse?
  - Does it use current API signatures?
  - Are imports still valid?
```

## Report Template
```
## Documentation Audit — [Date]

### Broken References
| Doc File | Line | Referenced Path | Status |
|----------|------|----------------|--------|
| CLAUDE.md | 42 | scripts/old_module.py | DELETED |

### Stale Descriptions
| Doc File | Section | Issue |
|----------|---------|-------|
| README.md | Setup | Missing new env var TOGETHER_API_KEY |

### Missing Documentation
| Feature/File | What's Undocumented |
|-------------|-------------------|
| scripts/post_upload_optimizer.py | No CLAUDE.md section |

### Score: X/Y checks passing
```

## Guardrails
- This skill VALIDATES docs, it does NOT generate them
- Fix broken references immediately (they mislead future sessions)
- Stale docs are worse than no docs (they create false confidence)

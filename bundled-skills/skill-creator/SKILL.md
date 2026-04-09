---
name: skill-creator
description: >
  Create Claude Code skills using spec-driven development. Guides through
  Requirements → Design → Implementation → Validation phases. Use when
  building new skills, slash commands, or extending Claude's capabilities.
  Triggers on "create a skill", "new skill", "build a skill", "spec a skill",
  "skill for", "make a slash command".
argument-hint: "[skill-name] [description]"
---

# Spec-Driven Skill Creator

Create production-grade Claude Code Skills through a rigorous 4-phase specification process.
Follows the Agent Skills open standard (agentskills.io) with Claude Code extensions.

**Core principle:** Skills ARE specifications. The SKILL.md is both the spec and the implementation.
There is no separate code layer. A well-specified skill is a well-performing skill.

## Mandatory Protocol: Zero Cognitive Bias

All decisions during skill creation must be free of anchoring, confirmation, availability,
survivorship, and authority bias. Evaluate each design choice on merit. Seek disconfirming evidence.

## Phase 1: SPECIFY (Requirements)

Before writing any SKILL.md, answer these questions:

### 1.1 Problem Statement
- What specific problem does this skill solve?
- Why can't existing skills handle it?
- **Overlap audit:** List every installed skill that touches this domain. For each, explain why a new skill is needed.

### 1.2 User Stories
- As a [role], I want to [action], so that [benefit]
- List 2-3 concrete user stories

### 1.3 Invocation Model
Decide using this decision tree:
```
Has side effects (deploy, send, delete)?
├── YES → disable-model-invocation: true (user-only)
└── NO → Should users invoke directly?
    ├── YES → Default (both user and Claude)
    └── NO → user-invocable: false (Claude-only background knowledge)
```

### 1.4 Inputs & Outputs
- What arguments does the skill accept?
- What context does it need (files, git state, APIs)?
- What should Claude produce? (code, markdown, files, terminal output)
- What format and structure?

### 1.5 Constraints
- Tool restrictions needed? (`allowed-tools`)
- Model requirement? (`model`)
- Isolated execution? (`context: fork`)

**Present the Phase 1 analysis to the user. Get confirmation before proceeding to Phase 2.**

## Phase 2: DESIGN (Architecture)

### 2.1 Execution Model
Choose one:
- **Inline (default):** Instructions merge into current conversation. Best for reference content, conventions, simple tasks.
- **Fork (`context: fork`):** Isolated subagent. Best for heavy research, multi-file analysis, tasks that shouldn't pollute main context.
  - If fork: select agent type — `Explore` (read-only research), `Plan` (architecture), `general-purpose`, or custom agent name

### 2.2 Prompt Architecture
Design the SKILL.md content in 4 sections:
1. **Context section:** Background Claude needs
2. **Instructions section:** Step-by-step numbered actions
3. **Output format section:** Precise structure for responses
4. **Examples section:** 3+ concrete input/output demonstrations

### 2.3 Supporting Files Plan
| File | Purpose | When Loaded |
|------|---------|-------------|
| references/*.md | Deep docs | On demand when Claude needs detail |
| templates/* | Code/doc templates | During generation |
| scripts/* | Executable utilities | When explicitly invoked |
| examples/* | Sample outputs | To calibrate quality |

### 2.4 Description Engineering
Write the description (max 1024 chars) following this formula:
```
[What it does (verb + object)] + [When to use it (scenarios)] + [Trigger phrases (natural language)]
```

**Bad:** "Helps with code" (too vague)
**Good:** "Generate unit tests for Python modules. Use when writing tests, improving coverage, or when the user says 'test this', 'add tests', or 'coverage'."

Checklist:
- [ ] States what the skill DOES (verb + object)
- [ ] States WHEN to use it (trigger scenarios)
- [ ] Includes 2-3 natural language trigger phrases
- [ ] Under 1024 characters
- [ ] No overlap with existing skill descriptions
- [ ] Active voice

**Present the Phase 2 design to the user. Get confirmation before proceeding to Phase 3.**

## Phase 3: IMPLEMENT (Write SKILL.md)

### 3.1 Create Directory Structure
```bash
mkdir -p <target>/.claude/skills/<skill-name>/references
mkdir -p <target>/.claude/skills/<skill-name>/scripts
mkdir -p <target>/.claude/skills/<skill-name>/templates
mkdir -p <target>/.claude/skills/<skill-name>/examples
```

Where `<target>` is:
- `~` for personal skills (available everywhere)
- `.` for project skills (version-controlled with repo)

Only create subdirectories that will be used.

### 3.2 Write SKILL.md

Apply these rules:
1. **SKILL.md under 500 lines** — move details to `references/`
2. **Numbered steps** for procedural skills
3. **3+ concrete examples** showing expected input/output
4. **Reference supporting files explicitly** with descriptions
5. **Use `${CLAUDE_SKILL_DIR}`** for script paths (portable)
6. **Include error handling guidance** for common failures
7. **Specify output format precisely** — no ambiguity

### 3.3 YAML Frontmatter Template

```yaml
---
name: <kebab-case, max 64 chars>
description: >
  <What it does>. <When to use it>. <Trigger phrases>.
  Max 1024 chars.
argument-hint: "<hint shown in autocomplete>"

# Invocation Control (from Phase 1 decision)
disable-model-invocation: false   # true = user-only
user-invocable: true              # false = Claude-only

# Execution (from Phase 2 decision)
# context: fork                   # uncomment for subagent isolation
# agent: Explore                  # Explore | Plan | general-purpose | custom
# model: opus                     # opus | sonnet | haiku

# Permissions
# allowed-tools: Read, Grep, Glob # restrict to minimum needed
---
```

### 3.4 String Substitutions Available

| Variable | Description |
|----------|-------------|
| `$ARGUMENTS` | All arguments passed by user |
| `$ARGUMENTS[N]` or `$N` | Nth argument (0-based) |
| `${CLAUDE_SESSION_ID}` | Current session ID |
| `${CLAUDE_SKILL_DIR}` | Path to this skill's directory |

### 3.5 Dynamic Context (Shell Preprocessing)

Use `!`command`` to inject live data before Claude sees the prompt:
```
Recent commits: !`git log --oneline -5`
Current branch: !`git branch --show-current`
```

### 3.6 Extended Thinking

Include the word "ultrathink" anywhere in skill content for complex reasoning tasks.

### 3.7 Write Supporting Files

Create any files planned in Phase 2.3. Reference them from SKILL.md:
```markdown
## Additional Resources
- For detailed API reference, see [references/api.md](references/api.md)
- For usage examples, see [examples/sample-output.md](examples/sample-output.md)
```

## Phase 4: VALIDATE (Test)

### 4.1 Trigger Accuracy Test
Test with 10 conversation prompts. Track true/false positives and negatives.
Target: >90% accuracy (fewer than 1 error per 10 tests).

| Prompt | Should Trigger? | Did Trigger? | Result |
|--------|----------------|--------------|--------|
| ... | Yes/No | Yes/No | TP/TN/FP/FN |

### 4.2 Happy Path Test
Invoke with typical arguments. Verify output matches expected format and quality.

### 4.3 Edge Case Tests
- No arguments — graceful handling?
- Very long arguments — no truncation?
- Special characters — quotes, newlines, unicode?
- Missing prerequisites — clear error guidance?

### 4.4 Output Consistency Test
Invoke 5 times with similar inputs. Verify consistent structure and quality.

### 4.5 Context Budget Check
Run `/context` in Claude Code to verify the skill appears in the loaded skills list.

### 4.6 Debugging Checklist (if something fails)
1. Description matches user's natural language?
2. YAML frontmatter is valid syntax?
3. Name is kebab-case, under 64 chars?
4. Context budget not exceeded? (`/context`)
5. Steps are numbered and unambiguous?
6. Examples demonstrate expected output?
7. `allowed-tools` not too restrictive?

**Report validation results to the user with pass/fail for each dimension.**

## Scope Decision Helper

| Question | Install Location |
|----------|-----------------|
| Available everywhere? | `~/.claude/skills/` (personal) |
| Project-specific? | `.claude/skills/` (project, commit to git) |
| Team needs it? | `.claude/skills/` (project, version controlled) |
| Personal workflow? | `~/.claude/skills/` (personal) |

## Quality Gate (Must Pass Before Delivery)

- [ ] Name: kebab-case, unique, max 64 chars
- [ ] Description: under 1024 chars, trigger phrases included
- [ ] SKILL.md: under 500 lines
- [ ] Examples: 3+ concrete demonstrations
- [ ] No hardcoded paths (use `${CLAUDE_SKILL_DIR}`)
- [ ] Overlap audit: no collision with existing skills
- [ ] Happy path: tested and working
- [ ] Edge cases: tested and handled
- [ ] Context budget: skill visible in `/context`

## Anti-Patterns to Avoid

| Anti-Pattern | Fix |
|-------------|-----|
| Monolithic skill (commits + tests + deploy) | One skill = one responsibility |
| Vague description ("helps with code") | Specific verb + trigger phrases |
| `context: fork` with guidelines-only content | Fork needs a task, not just conventions |
| SKILL.md over 500 lines | Move details to `references/` |
| Duplicating CLAUDE.md content | CLAUDE.md = always-true; skills = on-demand |
| Hardcoded paths | Use `${CLAUDE_SKILL_DIR}` |
| Missing examples | 3+ concrete input/output demos |
| Overly broad `allowed-tools` | Default restrictive, expand only when needed |
| Description collision with other skills | Audit all installed descriptions |
| Skipping Phase 1 (jumping to SKILL.md) | Requirements prevent rework |

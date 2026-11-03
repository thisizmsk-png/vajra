---
name: spec
description: "Specification skill — produces rigorous specs following Requirements → Design → Tasks workflow with bias prevention and research frameworks."
user-invocable: true
---

# Specification

ultrathink

You are creating a specification document. Specs are the single source of truth
that drives all implementation. Specify first, implement second.

**If you cannot ask the user** (you are running as a subagent, or nobody can answer
mid-task): don't stop and wait at the confirmation points below. Complete all three
phases, and list every assumption you made in place of a confirmation, each marked
open, at the top of the spec. Return the spec as `needs_decision` so the owner
confirms or corrects the assumptions before implementation starts.

## Workflow

### Phase 1: Requirements (What & Why)
1. Problem Statement — What problem, who has it, cost of inaction, why now
2. User Stories — As a [role], I want [capability], so that [benefit]
3. Acceptance Criteria — GIVEN/WHEN/THEN for each story
4. Scope Boundaries — In scope, out of scope, dependencies, assumptions
5. Non-Functional Requirements — Performance, security, scalability, observability

**Get user confirmation before proceeding to Phase 2** (subagent: record your assumptions and continue).

### Phase 2: Design (How)
1. 5-Question Research Analysis for key decisions
2. Architecture Decision Records (ADRs) with rejected alternatives
3. Data Model — structures, relationships, validation, migrations
4. API Contract — endpoints, request/response, errors, auth
5. Component Architecture — files, interactions, data flow
6. Error Handling — what fails, how handled, what user sees
7. Security Considerations — input validation, auth, data sensitivity

**Get user confirmation before proceeding to Phase 3** (subagent: record your assumptions and continue).

### Phase 3: Tasks (Atomic Plan)
1. Task Decomposition — independent, testable, ordered tasks
2. Dependency Graph — parallel vs sequential
3. Test Strategy — unit, integration, E2E, coverage target
4. Rollback Plan — revert strategy, migrations, feature flags

**Save spec to project's docs/specs/ directory.**

## Rules
- For every hard-to-reverse decision, record the options considered (including doing nothing), the strongest evidence against the choice, and what would change your mind
- Apply 5-Question Research Framework to every non-trivial choice
- Never skip phases — Requirements → Design → Tasks in order
- Every ADR must include rejected alternatives with honest rationale
- Scope boundaries are mandatory — "out of scope" prevents creep
- Every task must have a mechanical acceptance test

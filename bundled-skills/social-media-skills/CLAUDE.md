# Social Media Skills — Agent Guidelines

## Repository Overview

Social media skills for AI agents — 14 skills across strategy, creation, and analysis for text-first platforms (LinkedIn, Twitter/X, Threads, Bluesky).

Each skill lives in `skills/<name>/SKILL.md` and provides structured guidance for AI agents performing social media tasks.

## Skill Organization

Skills are organized into four layers, each building on the previous.

### Foundation
- **social-media-context-sms** — establishes shared context, platform knowledge, and user preferences used by all other skills

### Strategy
- **content-strategy-sms** — defines content pillars, audience targeting, and positioning
- **content-calendar-sms** — plans publishing cadence, themes, and scheduling across platforms
- **platform-strategy-sms** — tailors approach per platform based on audience and format strengths

### Creation
- **post-writer-sms** — writes single standalone posts optimized per platform
- **thread-writer-sms** — writes multi-post threads with narrative arc and strong hooks
- **carousel-writer-sms** — writes slide-by-slide carousel scripts for LinkedIn and similar formats
- **content-repurposer-sms** — transforms existing content into new formats and platforms
- **hook-writer-sms** — crafts high-performing opening lines to maximize engagement

### Analysis
- **performance-analyzer-sms** — interprets post and account metrics to surface insights
- **audience-growth-tracker-sms** — tracks follower trends and identifies growth drivers
- **content-pattern-analyzer-sms** — identifies what content types and topics perform best
- **optimization-advisor-sms** — recommends specific improvements based on data patterns

## Key Conventions

### YAML Frontmatter
Every `SKILL.md` must open with YAML frontmatter containing:
- `name`: kebab-case skill identifier (1–64 chars)
- `description`: plain-English summary of what the skill does (1–1024 chars)
- `metadata.version`: semver string (e.g. `"1.0.0"`)

### Naming
- **kebab-case** for all skill directory and file names
- Lowercase, alphanumeric characters and hyphens only
- No spaces, underscores, or special characters

### File Size
- Each `SKILL.md` must be **under 500 lines**
- Keep instructions focused; link to related skills rather than duplicating guidance

### Writing Style
- **Active voice** throughout — tell the agent what to do, not what might happen
- **Clarity over cleverness** — straightforward language is more reliable than clever phrasing
- **Bold key terms** when first introduced or when emphasis aids scanning
- Short paragraphs and bullet lists over dense prose

## Foundation Dependency

All skills (except `social-media-context-sms` itself) **check for `.agents/social-media-context-sms.md` first** before proceeding. This file contains user-specific context: platforms used, audience description, content pillars, tone preferences, and account handles.

If `.agents/social-media-context-sms.md` is not present, the skill should prompt the agent to run `social-media-context-sms` first, or proceed with reasonable defaults while noting the limitation.

## BlackTwist MCP Integration

**BlackTwist** is the primary tool integration for this skill set. It provides an MCP server with tools for posting, scheduling, and analytics across supported platforms.

- **When BlackTwist MCP is available**: use its tools directly for all posting, scheduling, and analytics operations
- **When BlackTwist MCP is not available**: fall back to advisory mode — output the content and instructions for the user to post manually; note what metrics to check and where to find them

Skills should detect MCP availability by attempting a lightweight call (e.g. `get_user_settings`) and adapting behavior accordingly.

See `tools/REGISTRY.md` for the full list of available BlackTwist tools.

## Cross-References

Skills should reference related skills where relevant. Use the format:

```
See also: **thread-writer-sms**, **hook-writer-sms**
```

Common cross-reference patterns:
- Creation skills reference **hook-writer-sms** for opening lines
- Creation skills reference **content-strategy-sms** for pillar alignment
- Analysis skills reference **optimization-advisor-sms** for next steps
- **content-repurposer-sms** references all creation skills as output targets

## Validation

Run `./validate-skills.sh` to check all skills against conventions before committing.

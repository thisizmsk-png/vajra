---
name: release-engineering
description: Release management, semantic versioning, changelogs, release notes, deployment coordination. Activate for release, version, changelog, deploy, ship, tag tasks.
user-invocable: true
---

# Release Engineering

## When to Activate
- User mentions: release, version, changelog, deploy, ship, tag, cut a release
- Preparing a product release or version bump
- Writing release notes or changelogs
- Coordinating deployment across environments

## Semantic Versioning
```
MAJOR.MINOR.PATCH  (e.g., 2.1.3)
```
| Bump | When | Example |
|------|------|---------|
| MAJOR | Breaking changes (API contract changes, removed features) | 1.0.0 → 2.0.0 |
| MINOR | New features (backwards compatible) | 1.0.0 → 1.1.0 |
| PATCH | Bug fixes (backwards compatible) | 1.0.0 → 1.0.1 |

**Pre-release**: `1.0.0-beta.1`, `1.0.0-rc.1`

## Release Process

### Phase 1: Pre-Release Checklist
- [ ] All tests passing on main branch
- [ ] No critical/high security vulnerabilities
- [ ] CHANGELOG.md updated with all changes since last release
- [ ] Version bumped in package.json / pyproject.toml / setup.py
- [ ] Documentation updated for new features
- [ ] Breaking changes documented with migration guide
- [ ] Performance regression check (no >10% degradation)

### Phase 2: Changelog Format
```markdown
## [1.2.0] - 2026-03-30

### Added
- Feature X that does Y (#123)
- Support for Z integration

### Changed
- Improved performance of A by 40%
- Updated dependency B to v3.0

### Fixed
- Bug where C caused D (#456)
- Memory leak in E component

### Removed
- Deprecated F endpoint (use G instead)

### Security
- Patched CVE-2026-XXXX in dependency H
```

### Phase 3: Git Tag + Release
```bash
git tag -a v1.2.0 -m "Release v1.2.0: [one-line summary]"
git push origin v1.2.0
gh release create v1.2.0 --title "v1.2.0" --notes-file RELEASE_NOTES.md
```

### Phase 4: Post-Release
- [ ] Verify deployment succeeded
- [ ] Monitor error rates for 1 hour post-deploy
- [ ] Announce release (changelog link, key highlights)
- [ ] Tag issues as "released in v1.2.0"

## Rollback Plan
Every release must have a rollback path:
- **Simple rollback**: `git revert` the release merge commit
- **Database migration rollback**: backward migration must exist
- **Feature flag rollback**: disable flag, no code change needed
- **Full rollback**: deploy previous version tag

## Guardrails
- NEVER release without passing tests
- NEVER skip the changelog (your future self needs it)
- ALWAYS have a rollback plan before deploying
- ALWAYS monitor after release (not "deploy and forget")

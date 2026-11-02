#!/usr/bin/env python3
"""Write the PR body for the skill-pins Action.

Only names, hashes, licenses and finding counts go in. Upstream text stays out of
the PR, because a later agent reading the PR would treat it as content to act on.
"""

import json
import re
import sys
from pathlib import Path

NAME_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,63}")
SHA_RE = re.compile(r"[0-9a-f]{40}")
REPO_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*")


def counts(scan_dir, name):
    try:
        findings = json.loads((Path(scan_dir) / (name + ".json")).read_text()).get("findings") or []
    except (OSError, ValueError, AttributeError):
        return "scan failed, treat as unreviewed"
    tally = {}
    for f in findings:
        sev = f.get("severity") if isinstance(f, dict) else None
        sev = sev if sev in ("critical", "high", "medium", "low", "info") else "other"
        tally[sev] = tally.get(sev, 0) + 1
    return ", ".join("%s %d" % kv for kv in sorted(tally.items())) or "no findings"


def main(report_path, scan_dir):
    report = json.loads(Path(report_path).read_text())
    print("Read each compare link before merging; nothing updates until you do. "
          "CI does not run on this PR, so run tests/run-all.sh locally.\n")
    print("| Skill | Pin | License | Scan |\n| --- | --- | --- | --- |")
    for c in report.get("changed", []):
        name = str(c.get("name"))
        pinned, head = str(c.get("pinned")), str(c.get("head"))
        repo = str(c.get("repo"))
        if not (NAME_RE.fullmatch(name) and SHA_RE.fullmatch(pinned) and SHA_RE.fullmatch(head)
                and REPO_RE.fullmatch(repo)):
            print("A row failed validation. Read the job log before merging.")
            continue
        url = "https://github.com/%s/compare/%s...%s" % (repo, pinned, head)
        lic = "CHANGED, stop and read" if c.get("license_changed") else "same"
        print("| %s | [%s to %s](%s) | %s | %s |" % (name, pinned[:7], head[:7], url, lic, counts(scan_dir, name)))
    errors = [e for e in report.get("errors", []) if NAME_RE.fullmatch(str(e.get("name")))]
    if errors:
        print("\nCould not check, so the path may have moved or a check failed: "
              + ", ".join(e["name"] for e in errors) + ".")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])

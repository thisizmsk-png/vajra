#!/usr/bin/env python3
"""Install third-party skills from the pinned catalog, config/skills.json.

Only names in the catalog can be installed. There is no way to pass a URL, so a
page or an agent cannot steer this at a repo of its choosing. Each install
fetches the pinned commit, checks its sha256 and copies it in.

Usage:
    vajra_skills.py add <name>      Install or update one catalog entry.
    vajra_skills.py list            Show entries and whether each is installed.
    vajra_skills.py verify          Re-hash installed entries against the catalog.
    vajra_skills.py pin <name>      Print the sha256 of the pinned commit.
    vajra_skills.py upstream        JSON report of skills changed upstream.

Exit codes: 0 done, 1 refused or mismatch, 2 network or tool failure.
"""

import hashlib
import json
import os
import pwd
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unicodedata
import urllib.request
from pathlib import Path

CATALOG = Path(__file__).resolve().parent.parent / "config" / "skills.json"
MARKER = ".vajra-skill.json"
# Local files that are not part of a skill.
LOCAL_FILES = {MARKER, ".pinned", ".DS_Store"}
REPO_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*")
COMMIT_RE = re.compile(r"[0-9a-f]{40}")
NAME_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,63}")
SHA256_RE = re.compile(r"[0-9a-f]{64}")
MAX_FILES = 3000
MAX_BYTES = 100 * 1024 * 1024
GIT_TIMEOUT = 120


class Refused(Exception):
    """The catalog or the fetched content failed a check. Exit 1."""


class Fetch(Exception):
    """The network or git failed. Exit 2; the run carries on without the skill."""


def skills_dir():
    home = os.environ.get("HOME", "")
    if not os.path.isabs(home):
        home = pwd.getpwuid(os.getuid()).pw_dir
    return Path(home) / ".claude" / "skills"


def load_catalog():
    try:
        with open(CATALOG, encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, ValueError) as e:
        raise Refused("cannot read %s: %s" % (CATALOG, e))
    skills = data.get("skills") if isinstance(data, dict) else None
    if not isinstance(skills, dict):
        raise Refused("catalog has no skills object")
    # Names become paths, so a crafted key must not reach `list` or `verify`.
    for name, entry in skills.items():
        if not NAME_RE.fullmatch(name) or not isinstance(entry, dict):
            raise Refused("bad catalog entry %r" % name)
    return skills


def entry_for(name, skills=None):
    skills = load_catalog() if skills is None else skills
    if not isinstance(name, str) or not NAME_RE.fullmatch(name):
        raise Refused("bad skill name")
    # Exact key lookup: a name that is not in the catalog never reaches git.
    entry = skills.get(name)
    if not isinstance(entry, dict):
        raise Refused("%s is not in the catalog" % name)
    if not REPO_RE.fullmatch(str(entry.get("repo"))) or ".." in str(entry.get("repo")):
        raise Refused("%s: bad repo in catalog" % name)
    if not COMMIT_RE.fullmatch(str(entry.get("commit"))):
        raise Refused("%s: commit must be a full 40-character sha" % name)
    path = entry.get("path")
    if not isinstance(path, str) or path.startswith("/") or ".." in path.split("/"):
        raise Refused("%s: bad path in catalog" % name)
    return entry


def git_env():
    # No user or system git config: url rewrites and hooks there must not apply.
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1",
               GIT_TERMINAL_PROMPT="0", GIT_ASKPASS="true")
    return env


def git(cwd, *args, binary=False):
    cmd = ["git", "-c", "core.symlinks=false", "-c", "core.hooksPath=/dev/null",
           "-c", "protocol.allow=never", "-c", "protocol.https.allow=always",
           "-c", "transfer.fsckObjects=true", "-c", "http.followRedirects=false"] + list(args)
    try:
        res = subprocess.run(cmd, cwd=cwd, env=git_env(), capture_output=True,
                             timeout=GIT_TIMEOUT, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Fetch("git failed: %s" % e)
    if res.returncode != 0:
        raise Fetch("git %s failed: %s" % (args[0], res.stderr.decode("utf-8", "replace").strip()[:200]))
    return res.stdout if binary else res.stdout.decode("utf-8", "surrogateescape")


def check_skill(files, entry):
    """Refuse what would run or widen permissions when the skill loads.

    A skill can pre-approve tools or run shell commands from its own frontmatter,
    so a clean hash is not enough for a later pin.
    """
    if "SKILL.md" not in files:
        raise Refused("no SKILL.md at %s" % (entry["path"] or "repo root"))
    text = files["SKILL.md"][1].decode("utf-8", "replace").lstrip("\ufeff \t\r\n")
    match = re.match(r"---\s*\n(.*?)\n---", text, re.S)
    front = match.group(1) if match else (text if text.startswith("---") else "")
    # Claude Code's own patterns; JS \\s also matches U+FEFF, Python's does not.
    if "```!" in text or re.search(r"(?:^|[\s\ufeff])!`", text, re.M):
        raise Refused("SKILL.md runs a command at load time")
    tools = re.search(r"^allowed-tools[ \t]*:(.*(?:\n[ \t-].*)*)", front, re.M)
    if tools:
        items = [t.strip(" \t-") for t in re.split(r"[,\n]", tools.group(1))]
        if ", ".join(t for t in items if t) != entry.get("allowed_tools"):
            raise Refused("SKILL.md allowed-tools is not the value in the catalog")
        front = front[:tools.start()] + front[tools.end():]
    # YAML has many spellings of a key: quoted, indented, flow, escaped. Without a
    # parser here, refuse them all and let a person read that frontmatter.
    if re.search(r"""\b(?:hooks|allowed[-_]tools)\b["']?\s*:""", front, re.I) \
            or re.search(r"""["'][^"'\n]*\\[^"'\n]*["']\s*:""", front):
        raise Refused("SKILL.md declares hooks or tools in its frontmatter")


def fetch_files(entry):
    """Return {relative path: (mode, bytes)} for the pinned commit."""
    commit, path = entry["commit"], entry["path"].strip("/")
    with tempfile.TemporaryDirectory(prefix="vajra-skill-") as tmp:
        git(tmp, "init", "-q")
        git(tmp, "fetch", "-q", "--depth=1", "--no-tags",
            "https://github.com/%s.git" % entry["repo"], commit)
        if git(tmp, "rev-parse", "FETCH_HEAD").strip() != commit:
            raise Refused("fetched commit is not the pinned one")
        args = ["ls-tree", "-r", "-l", "-z", commit]
        if path:
            args += ["--", path + "/"]
        files, total, seen = {}, 0, set()
        for row in git(tmp, *args).split("\0"):
            if not row:
                continue
            meta, _, full = row.partition("\t")
            mode, kind, blob, size = meta.split()
            if kind != "blob" or mode not in ("100644", "100755"):
                raise Refused("refusing %s entry %s" % (mode, full))
            rel = full[len(path) + 1:] if path else full
            parts = rel.split("/")
            if ".." in parts or "" in parts or any(ord(c) < 32 for c in rel):
                raise Refused("refusing path %r" % rel)
            # Case- and form-insensitive disks would merge these, so the install could
            # differ from what was hashed and scanned.
            folded = unicodedata.normalize("NFD", rel).casefold()
            if ".git" in [p.lower() for p in parts] or folded in seen or rel in LOCAL_FILES:
                raise Refused("refusing path %r" % rel)
            if max(len(p.encode("utf-8", "surrogateescape")) for p in parts) > 255:
                raise Refused("refusing long path %r" % rel[:80])
            seen.add(folded)
            total += int(size)
            if len(files) >= MAX_FILES or total > MAX_BYTES:
                raise Refused("skill is larger than the install limit")
            files[rel] = (mode, git(tmp, "cat-file", "blob", blob, binary=True))
    check_skill(files, entry)
    return files


def tree_hash(files):
    h = hashlib.sha256()
    for rel in sorted(files):
        mode, data = files[rel]
        h.update(("%s %s %d\n" % (mode, rel, len(data))).encode("utf-8", "surrogateescape"))
        h.update(data)
    return h.hexdigest()


def read_tree(root):
    """Hash input for an installed copy. What no pin can hold gets a mode no pin has."""
    files = {}

    def unreadable(err):
        files[os.path.relpath(err.filename, root)] = ("unreadable", b"")

    for base, dirs, names in os.walk(root, onerror=unreadable):
        for name in dirs + names:
            path = Path(base) / name
            rel = path.relative_to(root).as_posix()
            try:
                info = os.lstat(path)
                if stat.S_ISLNK(info.st_mode):
                    files[rel] = ("120000", os.fsencode(os.readlink(path)))
                elif stat.S_ISDIR(info.st_mode):
                    continue
                elif not stat.S_ISREG(info.st_mode) or any(ord(c) < 32 for c in rel):
                    files[rel] = ("special", b"")
                elif not (rel in LOCAL_FILES and not info.st_mode & 0o111):
                    files[rel] = ("100755" if info.st_mode & 0o100 else "100644", path.read_bytes())
            except OSError as e:
                e.filename = str(path)
                unreadable(e)
    return files


def marker_hash(target):
    """The hash Vajra recorded when it installed this copy, or None."""
    try:
        return json.loads((target / MARKER).read_text(encoding="utf-8")).get("sha256")
    except (OSError, ValueError, AttributeError):
        return None


def install(name, entry, files, sha):
    root = skills_dir()
    target = root / name
    try:
        root.mkdir(parents=True, exist_ok=True)
        tmp = Path(tempfile.mkdtemp(prefix=".vajra-add-", dir=str(root)))
    except OSError as e:
        raise Fetch("cannot write to %s (%s). Run this command yourself, outside the sandbox." % (root, e))
    old = None
    try:
        for rel, (mode, data) in files.items():
            dest = tmp / rel
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
            dest.chmod(0o755 if mode == "100755" else 0o644)
        tmp.chmod(0o755)
        (tmp / MARKER).write_text(json.dumps(
            {"name": name, "repo": entry["repo"], "commit": entry["commit"],
             "sha256": sha}, indent=2) + "\n", encoding="utf-8")
        if target.exists():
            old = Path(tempfile.mkdtemp(prefix=".vajra-old-", dir=str(root)))
            os.rename(target, old / name)
        os.rename(tmp, target)
    except OSError as e:
        shutil.rmtree(tmp, ignore_errors=True)
        # Parallel workers share hints, so another one may have just installed it.
        if target.is_dir() and not target.is_symlink() and tree_hash(read_tree(target)) == sha:
            return
        raise Fetch("install failed: %s" % e)
    finally:
        if old:
            shutil.rmtree(old, ignore_errors=True)


def report_installed(name, entry, note):
    print("%s %s: %s" % (note, name, skills_dir() / name / "SKILL.md"))
    if entry.get("conditions"):
        print("conditions: %s" % entry["conditions"])


def cmd_add(name):
    skills = load_catalog()
    entry = entry_for(name, skills)
    target = skills_dir() / name
    want = entry.get("sha256")
    if not SHA256_RE.fullmatch(str(want)):
        raise Refused("%s has no sha256 in the catalog; run `pin` and record it" % name)
    if target.is_symlink():
        raise Refused("%s is a symlink; move it aside first: %s" % (name, target))
    if target.exists():
        current = tree_hash(read_tree(target))
        if current == want:
            report_installed(name, entry, "already pinned")
            return 0
        # Replace only a copy Vajra installed and nobody changed since. Anything
        # else may be the user's own.
        if current != marker_hash(target):
            raise Refused("%s exists and differs from the pin; move it aside first: %s" % (name, target))
    files = fetch_files(entry)
    got = tree_hash(files)
    if got != want:
        raise Refused("%s hash mismatch: catalog %s, fetched %s" % (name, want, got))
    install(name, entry, files, got)
    report_installed(name, entry, "installed at %s" % entry["commit"][:7])
    return 0


def cmd_list():
    for name, entry in sorted(load_catalog().items()):
        state = "installed" if (skills_dir() / name).exists() else "missing"
        print("%-28s %-9s %-8s %s" % (name, state, entry.get("verdict", "?"), entry.get("repo", "")))
    return 0


def cmd_verify():
    bad = 0
    for name, entry in sorted(load_catalog().items()):
        target = skills_dir() / name
        if not target.exists() and not target.is_symlink():
            continue
        same = not target.is_symlink() and tree_hash(read_tree(target)) == entry.get("sha256")
        if not same:
            bad += 1
        print("%-28s %s" % (name, "ok" if same else "DIFFERS from the pin"))
    return 1 if bad else 0


def cmd_pin(name):
    print(tree_hash(fetch_files(entry_for(name))))
    return 0


def upstream_license(repo):
    url = "https://api.github.com/repos/%s/license" % repo
    req = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            return json.load(res).get("license", {}).get("spdx_id")
    except (OSError, ValueError):
        return None


def cmd_upstream(write=False, dump=None):
    """Report skills whose own files changed upstream. A person merges any new pin.

    With --write, new pins go into the catalog. With --dump DIR, the new files go
    to DIR/<name> for the scanner. Nothing is installed.
    """
    # An installed copy never rewrites its own pins: `add` trusts this file. HOME
    # is the caller's to set, so this looks at the path itself.
    for path in [CATALOG.resolve()] + ([Path(dump).resolve()] if dump else []):
        if (write or dump) and ".claude" in path.parts:
            raise Refused("upstream --write and --dump do not run under a .claude directory")
    data = json.loads(CATALOG.read_text(encoding="utf-8"))
    changed, errors = [], []
    for name in sorted(data["skills"]):
        try:
            entry = entry_for(name, data["skills"])
            with tempfile.TemporaryDirectory(prefix="vajra-ls-") as tmp:
                out = git(tmp, "ls-remote", "https://github.com/%s.git" % entry["repo"], "HEAD")
            head = out.split()[0] if out.split() else ""
            if not COMMIT_RE.fullmatch(head):
                raise Fetch("no HEAD for %s" % entry["repo"])
            if head == entry["commit"]:
                continue
            files = fetch_files(dict(entry, commit=head))
        except (Fetch, Refused, OSError, ValueError) as e:
            # A skill that moved, or broke a check, is exactly what a person must see.
            errors.append({"name": name, "error": str(e)})
            continue
        sha = tree_hash(files)
        if sha == entry.get("sha256"):
            continue
        lic = upstream_license(entry["repo"])
        changed.append({"name": name, "repo": entry["repo"], "pinned": entry["commit"], "head": head,
                        "sha256": sha, "license": lic, "license_changed": lic != entry.get("license")})
        if dump:
            for rel, (_, blob) in files.items():
                target = Path(dump) / name / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(blob)
        if write:
            data["skills"][name].update(commit=head, sha256=sha)
    if write and changed:
        CATALOG.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"changed": changed, "errors": errors}, indent=2))
    return 0


def main(argv):
    if len(argv) == 2 and argv[1] == "list":
        return cmd_list()
    if len(argv) == 2 and argv[1] == "verify":
        return cmd_verify()
    if len(argv) >= 2 and argv[1] == "upstream":
        rest = argv[2:]
        dump = rest[rest.index("--dump") + 1] if "--dump" in rest[:-1] else None
        return cmd_upstream(write="--write" in rest, dump=dump)
    if len(argv) == 3 and argv[1] == "add":
        return cmd_add(argv[2])
    if len(argv) == 3 and argv[1] == "pin":
        return cmd_pin(argv[2])
    print(__doc__.strip(), file=sys.stderr)
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Refused as e:
        print("refused: %s" % e, file=sys.stderr)
        sys.exit(1)
    except Fetch as e:
        print("skipped, could not fetch: %s" % e, file=sys.stderr)
        sys.exit(2)

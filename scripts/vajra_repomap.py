#!/usr/bin/env python3
"""vajra_repomap.py — just-in-time repository map for context orientation.

A dependency-light implementation of aider's repo-map idea: rank files by a
PageRank over the definition/reference graph, seed the ranking toward the files
the current task touches, and emit a budget-capped map of the most relevant
definitions. Pure stdlib + git — no tree-sitter/networkx required, so it runs
anywhere. (Upgrade path to a tree-sitter version is documented in
engine/repo-map.md.)

Usage:
  vajra_repomap.py [--budget N] [--seed file_or_ident ...] [--root DIR]
  vajra_repomap.py --json ...

Output: top-ranked files, each with the definitions that fit the token budget.
"""
import argparse, json, os, re, subprocess, sys
from collections import defaultdict

# Definition patterns per extension: capture the defined symbol name.
DEF_PATTERNS = {
    ".py":  [r'^\s*(?:async\s+)?def\s+(\w+)', r'^\s*class\s+(\w+)'],
    ".ts":  [r'^\s*export\s+(?:default\s+)?(?:async\s+)?function\s+(\w+)',
             r'^\s*export\s+(?:abstract\s+)?class\s+(\w+)',
             r'^\s*export\s+(?:const|let|type|interface|enum)\s+(\w+)',
             r'^\s*(?:async\s+)?function\s+(\w+)', r'^\s*class\s+(\w+)'],
    ".go":  [r'^\s*func\s+(?:\([^)]*\)\s*)?(\w+)', r'^\s*type\s+(\w+)'],
    ".rs":  [r'^\s*(?:pub\s+)?fn\s+(\w+)', r'^\s*(?:pub\s+)?(?:struct|enum|trait)\s+(\w+)'],
    ".java":[r'^\s*(?:public|private|protected).*\s(\w+)\s*\(', r'^\s*(?:public\s+)?(?:class|interface|enum)\s+(\w+)'],
}
DEF_PATTERNS[".tsx"] = DEF_PATTERNS[".ts"]
DEF_PATTERNS[".js"]  = DEF_PATTERNS[".ts"]
DEF_PATTERNS[".jsx"] = DEF_PATTERNS[".ts"]
WORD = re.compile(r'[A-Za-z_]\w{2,}')  # identifiers, ≥3 chars (skip noise)


def git(args, root):
    try:
        return subprocess.run(["git", "-C", root, *args], capture_output=True, text=True, timeout=20).stdout
    except Exception:
        return ""


def repo_files(root):
    out = git(["ls-files"], root)
    files = [f for f in out.splitlines() if os.path.splitext(f)[1] in DEF_PATTERNS]
    if files:
        return files
    # Not a git repo — walk the tree.
    res = []
    for dp, dns, fns in os.walk(root):
        dns[:] = [d for d in dns if d not in (".git", "node_modules", "venv", "__pycache__", "dist", "build")]
        for fn in fns:
            if os.path.splitext(fn)[1] in DEF_PATTERNS:
                res.append(os.path.relpath(os.path.join(dp, fn), root))
    return res


def extract(root, rel):
    """Return (defs:set[str], words:set[str]) for a file."""
    ext = os.path.splitext(rel)[1]
    pats = [re.compile(p) for p in DEF_PATTERNS.get(ext, [])]
    defs, words = set(), set()
    try:
        with open(os.path.join(root, rel), "r", errors="ignore") as fh:
            for line in fh:
                for p in pats:
                    m = p.search(line)
                    if m:
                        defs.add(m.group(1))
                words.update(WORD.findall(line))
    except OSError:
        pass
    return defs, words


def pagerank(nodes, edges, personalization, d=0.85, iters=40):
    """edges: dict[src] -> list[dst] (referencer -> definer). Pure-Python."""
    N = len(nodes)
    if N == 0:
        return {}
    rank = {n: 1.0 / N for n in nodes}
    psum = sum(personalization.values())
    if psum > 0:
        pers = {n: personalization.get(n, 0.0) / psum for n in nodes}
    else:
        pers = {n: 1.0 / N for n in nodes}  # unseeded → uniform teleport
    out_deg = {n: len(edges.get(n, [])) for n in nodes}
    # Precompute in-links once (referencer -> definer); avoids O(N^2) per iter.
    in_links = defaultdict(list)
    for src in nodes:
        for dst in edges.get(src, ()):
            in_links[dst].append(src)
    for _ in range(iters):
        dangling = sum(rank[n] for n in nodes if out_deg[n] == 0)
        nxt = {}
        for n in nodes:
            inflow = sum(rank[src] / out_deg[src] for src in in_links.get(n, ()) if out_deg[src])
            nxt[n] = (1 - d) * pers[n] + d * (inflow + dangling * pers[n])
        rank = nxt
    return rank


def build(root, seeds, budget):
    files = repo_files(root)
    fdefs, fwords = {}, {}
    for f in files:
        fdefs[f], fwords[f] = extract(root, f)
    # symbol -> defining files
    sym2def = defaultdict(set)
    for f, ds in fdefs.items():
        for s in ds:
            sym2def[s].add(f)
    # edges: file references a symbol defined elsewhere -> edge file -> definer
    edges = defaultdict(list)
    for f, ws in fwords.items():
        for w in ws:
            for definer in sym2def.get(w, ()):
                if definer != f:
                    edges[f].append(definer)
    # personalization: seed files + files defining a seeded identifier
    seed_files = set()
    for s in seeds:
        if s in fdefs:
            seed_files.add(s)
        for definer in sym2def.get(s, ()):
            seed_files.add(definer)
        seed_files.update(f for f in files if s in f)
    pers = {f: (100.0 if f in seed_files else 0.0) for f in files} if seed_files else {}
    ranks = pagerank(files, edges, pers)
    ordered = sorted(files, key=lambda f: ranks.get(f, 0), reverse=True)
    # budget by estimated tokens (~4 chars/token): include top files' def lists
    out, used = [], 0
    for f in ordered:
        ds = sorted(fdefs[f])
        if not ds:
            continue
        block = f + ": " + ", ".join(ds[:40])
        cost = len(block) // 4 + 1
        if used + cost > budget and out:
            break
        out.append({"file": f, "rank": round(ranks.get(f, 0), 5), "defs": ds[:40]})
        used += cost
    return out, len(files)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--budget", type=int, default=1024)
    ap.add_argument("--seed", nargs="*", default=[])
    ap.add_argument("--root", default=".")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    root = os.path.abspath(a.root)
    mapping, total = build(root, a.seed, a.budget)
    if a.json:
        print(json.dumps({"root": root, "filesScanned": total, "map": mapping}, indent=2))
        return
    if not mapping:
        print("(no mappable source files found)")
        return
    print(f"Repo map — {root}  ({total} files scanned, top {len(mapping)} by relevance)")
    print("-" * 70)
    for m in mapping:
        print(f"{m['file']}")
        print(f"    {', '.join(m['defs'])}")


if __name__ == "__main__":
    main()

"""No waiting calls: count what can block the window, and let the count only go down.

usage: waiting_calls.py [--write]      (run from anywhere; paths are found from this file)

Three counts, each compared with `waiting-calls.json` next to this file:

1. main-thread commands: a Tauri command declared as a plain `fn` with `#[tauri::command]` runs on the main thread.
   `#[tauri::command(async)]` and `async fn` do not. The list is held by NAME, so swapping one for another fails too.
2. blocking calls written directly in the body of one of those commands (file reads, sleeps, child processes, channel
   waits). Only the body is read: a callee in another module is not followed, so this is a floor, not a proof.
3. synchronous calls in the UI source (XMLHttpRequest, Atomics.wait, anything named `...Sync(`), test files excluded.

The rule that the number only goes down is enforced both ways. Above the allowance fails. Below it fails too, with the
line to write, so the allowance is always exact and a later PR cannot spend room an earlier one freed.
"""
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent.parent
ALLOWANCE = HERE / "waiting-calls.json"
CORE_SRC = ROOT / "app" / "core" / "src"
UI_SRC = ROOT / "app" / "ui" / "src"

# What blocks if it sits in the body of a main-thread command.
BLOCKING = re.compile(
    r"std::fs::|\bfs::(?:read|write|copy|rename|remove|metadata|create)|File::(?:open|create)|read_to_string|read_dir"
    r"|thread::sleep|Command::new|\.recv\(\)|\.recv_timeout\(|\.join\(\)|\.wait\(\)|block_on\("
)
# The UI has no synchronous call into the core (Tauri's invoke is a promise); these are the ways to wait anyway.
UI_SYNC = re.compile(r"XMLHttpRequest|Atomics\.wait|\b[A-Za-z_]+Sync\(")
ATTR = re.compile(r"#\[tauri::command(\((?P<args>[^)]*)\))?\]")
# Any visibility (`pub`, `pub(crate)`, `pub(super)`, `pub(in path)`), and the qualifiers that can precede `fn`.
FN = re.compile(r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:const|unsafe|extern(?:\s+\"[^\"]*\")?)\s+)*(?P<async>async\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")


def commands(text):
    """[(name, is_main_thread, body)] for every `#[tauri::command]` in `text`. Commented-out ones are skipped.

    A command attribute with no recognisable `fn` after it comes back with name None and the first line that followed, so
    the caller can stop instead of silently not counting it."""
    lines = text.splitlines()
    out = []
    for i, line in enumerate(lines):
        if line.lstrip().startswith("//"):
            continue
        m = ATTR.search(line)
        if not m:
            continue
        attr_async = "async" in (m.group("args") or "")
        j = i + 1
        while j < len(lines) and (lines[j].lstrip().startswith(("#[", "//")) or not lines[j].strip()):
            j += 1
        f = FN.match(lines[j]) if j < len(lines) else None
        if not f:
            out.append((None, False, (lines[j] if j < len(lines) else "<end of file>").strip()))
            continue
        out.append((f.group("name"), not (attr_async or f.group("async")), body_of(lines, j)))
    return out


def body_of(lines, start):
    """The text from the first `{` at or after `start` to its matching `}`. Braces inside strings are not special-cased."""
    depth, seen, buf = 0, False, []
    for line in lines[start:]:
        for ch in line:
            if ch == "{":
                depth += 1
                seen = True
            elif ch == "}":
                depth -= 1
        buf.append(line)
        if seen and depth <= 0:
            break
    return "\n".join(buf)


def count_core(sources):
    """(sorted names of main-thread commands, blocking calls written in their bodies, command attributes it cannot parse)."""
    names, blocking, unparsed = [], 0, []
    for text in sources:
        for name, main, body in commands(text):
            if name is None:
                unparsed.append(body)
            elif main:
                names.append(name)
                # One per line, not per token: `std::fs::read_to_string` is one call that matches two patterns.
                blocking += sum(1 for l in strip_comments(body).splitlines() if BLOCKING.search(l))
    return sorted(names), blocking, unparsed


def strip_comments(text):
    return "\n".join(l.split("//", 1)[0] for l in text.splitlines())


def count_ui(sources):
    return sum(len(UI_SYNC.findall(strip_comments(t))) for t in sources)


def read_repo():
    core = [p.read_text() for p in sorted(CORE_SRC.rglob("*.rs"))]
    ui = [p.read_text() for p in sorted(UI_SRC.rglob("*")) if p.suffix in (".ts", ".tsx") and ".test." not in p.name]
    names, blocking, unparsed = count_core(core)
    return {"main_thread_commands": names, "blocking_calls_in_main_thread_commands": blocking, "sync_ui_calls": count_ui(ui),
            "unparsed_commands": unparsed}


def problems(actual, allowed):
    """Failure lines. Empty when the actual counts equal the allowance."""
    out = []
    # Never part of the allowance: a command the scanner cannot read is a command it cannot count.
    for line in actual.get("unparsed_commands", []):
        out.append(f"a `#[tauri::command]` is followed by {line!r}, which this scanner cannot read as a function: it would not be counted. Write it as `fn name(` (any visibility), or teach waiting_calls.py the shape")
    new = sorted(set(actual["main_thread_commands"]) - set(allowed["main_thread_commands"]))
    gone = sorted(set(allowed["main_thread_commands"]) - set(actual["main_thread_commands"]))
    if new:
        out.append(f"new main-thread command(s) {new}: make them `#[tauri::command(async)]` (or `async fn`) instead of adding to the allowance")
    if gone:
        out.append(f"main-thread command(s) {gone} are gone: remove them from `main_thread_commands` in waiting-calls.json so the allowance goes down")
    for key in ("blocking_calls_in_main_thread_commands", "sync_ui_calls"):
        if actual[key] > allowed[key]:
            out.append(f"{key} is {actual[key]}, allowance {allowed[key]}: remove the new waiting call")
        elif actual[key] < allowed[key]:
            out.append(f"{key} is {actual[key]}, allowance {allowed[key]}: lower the allowance to {actual[key]}")
    return out


if __name__ == "__main__":
    actual = read_repo()
    if "--write" in sys.argv:
        if actual["unparsed_commands"]:
            print("\n".join(problems(actual, actual)))
            sys.exit(1)
        ALLOWANCE.write_text(json.dumps({k: v for k, v in actual.items() if k != "unparsed_commands"}, indent=2) + "\n")
        print(f"wrote {ALLOWANCE.name}: {len(actual['main_thread_commands'])} main-thread commands")
        sys.exit(0)
    found = problems(actual, json.loads(ALLOWANCE.read_text()))
    print("\n".join(found) or "waiting calls: ok")
    sys.exit(1 if found else 0)

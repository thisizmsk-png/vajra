import { useMemo, useState } from "react";
import { sanitizeLine } from "./ansi";
import { ListBox } from "../base/ListBox";
import { ListRow } from "../base/ListRow";
import { VirtualList } from "./VirtualList";

type Kind = "add" | "del" | "ctx" | "hunk";
type Line = { kind: Kind; text: string; cut: boolean };
export type FileDiff = { path: string; added: number; removed: number; lines: Line[] };

const plain = (s: string) => {
  const r = sanitizeLine(s);
  return { text: r.segments.map((x) => x.text).join(""), cut: r.truncated };
};

const ESCAPES: Record<string, number> = { t: 9, n: 10, r: 13, '"': 34, "\\": 92 };

/** A path as git writes it when it needs quoting: "\303\251vil.sh" with octal bytes and \t \n \" \\ escapes. */
function unquote(body: string): string {
  const bytes: number[] = [];
  const enc = new TextEncoder();
  for (let i = 0; i < body.length; i++) {
    if (body[i] !== "\\") {
      bytes.push(...enc.encode(body[i]));
      continue;
    }
    const oct = /^[0-7]{3}/.exec(body.slice(i + 1, i + 4));
    if (oct) {
      bytes.push(parseInt(oct[0], 8));
      i += 3;
    } else {
      bytes.push(ESCAPES[body[i + 1]] ?? 63);
      i += 1;
    }
  }
  return new TextDecoder("utf-8", { fatal: false }).decode(Uint8Array.from(bytes));
}

/** The b-side path of a `diff --git` header (plain or quoted), or "(unnamed file)" if it cannot be read. */
function headerPath(line: string): string {
  const quoted = /"b\/((?:[^"\\]|\\.)*)"\s*$/.exec(line);
  const at = line.lastIndexOf(" b/");
  const path = quoted ? unquote(quoted[1]) : at >= 0 ? line.slice(at + 3) : "";
  return plain(path).text || "(unnamed file)";
}

/**
 * Parse a unified diff (text from git) into files. Every `diff ` line starts a new file, so a file can never be merged
 * into another or vanish. File names and lines are untrusted: controls are removed and long lines are marked as cut.
 * Inside a hunk a line is classified by its first character only (`+++ x` there is an added line `++ x`, not a
 * header), so nothing can be hidden by looking like one.
 */
export function parseDiff(diff: string): FileDiff[] {
  const files: FileDiff[] = [];
  let cur: FileDiff | null = null;
  let inHunk = false;
  for (const raw of diff.split("\n")) {
    if (raw.startsWith("diff ")) { // a hunk line starts with space, + or -, so this can only be a file header
      cur = { path: headerPath(raw), added: 0, removed: 0, lines: [] };
      files.push(cur);
      inHunk = false;
      continue;
    }
    if (!cur) continue;
    if (raw.startsWith("@@")) {
      inHunk = true;
      cur.lines.push({ kind: "hunk", ...plain(raw) });
    } else if (!inHunk) {
      continue; // header lines (index, ---, +++, modes, renames) before the first hunk
    } else if (raw.startsWith("+")) {
      cur.added++;
      cur.lines.push({ kind: "add", ...plain(raw.slice(1)) });
    } else if (raw.startsWith("-")) {
      cur.removed++;
      cur.lines.push({ kind: "del", ...plain(raw.slice(1)) });
    } else if (raw.startsWith(" ")) {
      cur.lines.push({ kind: "ctx", ...plain(raw.slice(1)) });
    }
  }
  return files;
}

const GLYPH: Record<Kind, string> = { add: "+", del: "-", ctx: " ", hunk: "@" };
const SAYS: Record<Kind, string> = { add: "added", del: "removed", ctx: "unchanged", hunk: "section" };

/** File list plus the selected file's lines, virtualized. Added and removed lines carry a +/- glyph and a screen-reader word, not only color. */
export function DiffView({ diff, height = 360 }: { diff: string; height?: number }) {
  const files = useMemo(() => parseDiff(diff), [diff]);
  const [sel, setSel] = useState("");
  const f = files.find((x) => x.path === sel) ?? files[0];
  if (!f) return <p className="v-text v-text--body-sm">No changes against the base.</p>;
  return (
    <div className="v-diff">
      <ListBox label="Changed files">
        {files.map((x, i) => (
          <ListRow key={`${i}-${x.path}`} selected={x === f} onSelect={() => setSel(x.path)} meta={`+${x.added} -${x.removed}`}>
            {x.path}
          </ListRow>
        ))}
      </ListBox>
      <VirtualList
        key={f.path}
        className="v-difflines"
        role="region"
        aria-label={`Diff of ${f.path}`}
        tabIndex={0}
        count={f.lines.length}
        rowHeight={18}
        height={height}
        renderRow={(i) => {
          const l = f.lines[i];
          return (
            <span className={`v-dl v-dl--${l.kind}`}>
              <span className="v-sr">{SAYS[l.kind]}: </span>
              <span aria-hidden="true" className="v-dl__glyph">
                {GLYPH[l.kind]}
              </span>
              <span>{l.text}</span>
              {l.cut && <span className="v-dl__cut"> [line cut]</span>}
            </span>
          );
        }}
      />
    </div>
  );
}

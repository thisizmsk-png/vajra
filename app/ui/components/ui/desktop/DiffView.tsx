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

/**
 * The b-side path of a `diff --git` header (plain or quoted), or "" if it cannot be read. Git does not quote a path
 * that merely contains spaces, so `a/dir b/f.ts b/dir b/f.ts` cannot be split at the last " b/": when a file is not
 * renamed both halves are the same path, which fixes where the middle is. A rename's real names come from the
 * `rename to` / `+++` header lines (see `refine`).
 */
function headerPath(line: string): string {
  const quoted = /"b\/((?:[^"\\]|\\.)*)"\s*$/.exec(line);
  if (quoted) return unquote(quoted[1]);
  const rest = line.replace(/^diff --git /, "");
  const n = (rest.length - 5) / 2; // "a/" + P + " b/" + P
  if (Number.isInteger(n) && n > 0 && rest.startsWith("a/") && rest.slice(2 + n, 2 + n + 3) === " b/" && rest.slice(2, 2 + n) === rest.slice(2 + n + 3)) {
    return rest.slice(2 + n + 3);
  }
  const at = line.lastIndexOf(" b/");
  return at >= 0 ? line.slice(at + 3) : "";
}

/** A path from a `rename to`, `+++ b/` or `--- a/` header line (git adds a tab after a path with spaces). */
function refine(raw: string): string | null {
  if (raw.startsWith("rename to ")) return raw.slice(10).split("\t")[0];
  const m = /^(?:\+\+\+|---) (?:"[ab]\/((?:[^"\\]|\\.)*)"|[ab]\/(.*))$/.exec(raw);
  return m ? (m[1] !== undefined ? unquote(m[1]) : m[2].split("\t")[0]) : null;
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
      cur = { path: plain(headerPath(raw)).text || "(unnamed file)", added: 0, removed: 0, lines: [] };
      files.push(cur);
      inHunk = false;
      continue;
    }
    if (!cur) continue;
    if (raw.startsWith("@@")) {
      inHunk = true;
      cur.lines.push({ kind: "hunk", ...plain(raw) });
    } else if (!inHunk) {
      // header lines (index, ---, +++, modes, renames) before the first hunk: the real path fields refine the name
      const p = refine(raw);
      if (p) cur.path = plain(p).text || cur.path;
      continue;
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
  // Files are selected by position, never by name: two files can show the same label (`(unnamed file)`, or names that
  // differ only in removed control characters).
  const [sel, setSel] = useState(0);
  const fi = sel < files.length ? sel : 0;
  const f = files[fi];
  if (!f) return <p className="v-text v-text--body-sm">No changes against the base.</p>;
  return (
    <div className="v-diff">
      <ListBox label="Changed files">
        {files.map((x, i) => (
          <ListRow key={i} selected={i === fi} onSelect={() => setSel(i)} meta={`+${x.added} -${x.removed}`}>
            {x.path}
          </ListRow>
        ))}
      </ListBox>
      <VirtualList
        key={fi}
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

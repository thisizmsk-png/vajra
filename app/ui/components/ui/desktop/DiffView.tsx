import { useMemo, useState } from "react";
import { sanitizeLine } from "./ansi";
import { ListRow } from "../base/ListRow";
import { VirtualList } from "./VirtualList";

type Kind = "add" | "del" | "ctx" | "hunk";
type Line = { kind: Kind; text: string };
export type FileDiff = { path: string; added: number; removed: number; lines: Line[] };

const plain = (s: string) => sanitizeLine(s).segments.map((x) => x.text).join("");

/** Parse a unified diff (text from git) into files. File names and lines are untrusted: controls are removed. */
export function parseDiff(diff: string): FileDiff[] {
  const files: FileDiff[] = [];
  let cur: FileDiff | null = null;
  for (const raw of diff.split("\n")) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(raw);
    if (m) { cur = { path: plain(m[2]), added: 0, removed: 0, lines: [] }; files.push(cur); continue; }
    if (!cur || /^(index |--- |\+\+\+ |new file|deleted file|old mode|new mode|similarity|rename )/.test(raw)) continue;
    if (raw.startsWith("@@")) cur.lines.push({ kind: "hunk", text: plain(raw) });
    else if (raw.startsWith("+")) { cur.added++; cur.lines.push({ kind: "add", text: plain(raw.slice(1)) }); }
    else if (raw.startsWith("-")) { cur.removed++; cur.lines.push({ kind: "del", text: plain(raw.slice(1)) }); }
    else if (raw.startsWith(" ")) cur.lines.push({ kind: "ctx", text: plain(raw.slice(1)) });
  }
  return files;
}

const GLYPH: Record<Kind, string> = { add: "+", del: "-", ctx: " ", hunk: "@" };
const SAYS: Record<Kind, string> = { add: "added", del: "removed", ctx: "unchanged", hunk: "section" };

/** File list plus the selected file's lines, virtualized. Added and removed lines carry a +/- glyph and a screen-reader word, not only color. */
export function DiffView({ diff, height = 360 }: { diff: string; height?: number }) {
  const files = useMemo(() => parseDiff(diff), [diff]);
  const [sel, setSel] = useState(0);
  const f = files[Math.min(sel, files.length - 1)];
  if (!f) return <p className="v-text v-text--body-sm">No changes against the base.</p>;
  return (
    <div className="v-diff">
      <div role="listbox" aria-label="Changed files">
        {files.map((x, i) => (
          <ListRow key={x.path} selected={i === sel} onSelect={() => setSel(i)} meta={`+${x.added} -${x.removed}`}>{x.path}</ListRow>
        ))}
      </div>
      <VirtualList
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
              <span aria-hidden="true" className="v-dl__glyph">{GLYPH[l.kind]}</span>
              <span>{l.text}</span>
            </span>
          );
        }}
      />
    </div>
  );
}

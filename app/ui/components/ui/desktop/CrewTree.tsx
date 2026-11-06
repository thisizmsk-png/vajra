import { useState } from "react";
import type { KeyboardEvent } from "react";
import type { StatusKey } from "../../../strings";
import { StatusDot } from "../base/StatusDot";

export type CrewNode = { id: string; label: string; status: StatusKey; children?: CrewNode[] };

type Flat = { node: CrewNode; level: number; parent: string | null };

function flatten(nodes: CrewNode[], open: ReadonlySet<string>, level = 1, parent: string | null = null, out: Flat[] = []): Flat[] {
  for (const n of nodes) {
    out.push({ node: n, level, parent });
    if (n.children?.length && open.has(n.id)) flatten(n.children, open, level + 1, n.id, out);
  }
  return out;
}

/** Lead, run, agents, sub-workers as an ARIA tree: roving tabindex, Up/Down, Right opens or enters, Left closes or goes to the parent. */
export function CrewTree({ label, nodes, selected, onSelect, defaultOpen = [] }: { label: string; nodes: CrewNode[]; selected: string; onSelect: (id: string) => void; defaultOpen?: string[] }) {
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set(defaultOpen));
  const [focus, setFocus] = useState(selected);
  const rows = flatten(nodes, open);
  const idx = Math.max(0, rows.findIndex((r) => r.node.id === focus));
  const toggle = (id: string, to: boolean) => setOpen((s) => { const n = new Set(s); if (to) n.add(id); else n.delete(id); return n; });
  const key = (e: KeyboardEvent, r: Flat, i: number) => {
    const has = !!r.node.children?.length;
    const go = (j: number) => { e.preventDefault(); const t = rows[Math.min(rows.length - 1, Math.max(0, j))]; setFocus(t.node.id); document.getElementById(`crew-${t.node.id}`)?.focus(); };
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(rows.length - 1);
    else if (e.key === "ArrowRight" && has) { e.preventDefault(); if (!open.has(r.node.id)) toggle(r.node.id, true); else go(i + 1); }
    else if (e.key === "ArrowLeft") {
      e.preventDefault();
      if (has && open.has(r.node.id)) toggle(r.node.id, false);
      else if (r.parent) go(rows.findIndex((x) => x.node.id === r.parent));
    } else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(r.node.id); }
  };
  return (
    <ul role="tree" aria-label={label} className="v-tree">
      {rows.map((r, i) => (
        <li
          key={r.node.id}
          id={`crew-${r.node.id}`}
          role="treeitem"
          aria-level={r.level}
          aria-selected={r.node.id === selected}
          aria-expanded={r.node.children?.length ? open.has(r.node.id) : undefined}
          tabIndex={i === idx ? 0 : -1}
          className="v-row"
          style={{ paddingLeft: 8 + (r.level - 1) * 16 }}
          onClick={() => { setFocus(r.node.id); onSelect(r.node.id); }}
          onKeyDown={(e) => key(e, r, i)}
        >
          <span>{r.node.label}</span>
          <span className="v-row__meta"><StatusDot status={r.node.status} /></span>
        </li>
      ))}
    </ul>
  );
}

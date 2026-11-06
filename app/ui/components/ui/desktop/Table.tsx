import { useState } from "react";
import type { ReactNode } from "react";

export type Column<R> = { key: string; header: string; sort?: (a: R, b: R) => number; cell: (row: R) => ReactNode; numeric?: boolean };

/** A real `<table>`: header cells, `aria-sort` on the sorted column, optional expandable rows with `aria-expanded`. */
export function Table<R>({ caption, columns, rows, rowKey, expand }: { caption: string; columns: ReadonlyArray<Column<R>>; rows: readonly R[]; rowKey: (r: R) => string; expand?: (r: R) => ReactNode }) {
  const [sort, setSort] = useState<{ key: string; dir: 1 | -1 } | null>(null);
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const col = sort && columns.find((c) => c.key === sort.key);
  const shown = col?.sort && sort ? [...rows].sort((a, b) => col.sort!(a, b) * sort.dir) : rows;
  const toggle = (k: string) => setOpen((s) => { const n = new Set(s); if (n.has(k)) n.delete(k); else n.add(k); return n; });
  return (
    <table className="v-table">
      <caption className="v-sr">{caption}</caption>
      <thead>
        <tr>
          {expand && <th scope="col"><span className="v-sr">Details</span></th>}
          {columns.map((c) => (
            <th key={c.key} scope="col" aria-sort={sort?.key === c.key ? (sort.dir === 1 ? "ascending" : "descending") : c.sort ? "none" : undefined} className={c.numeric ? "v-num" : undefined}>
              {c.sort ? (
                <button type="button" className="v-th-button" onClick={() => setSort(sort?.key === c.key && sort.dir === 1 ? { key: c.key, dir: -1 } : { key: c.key, dir: 1 })}>
                  {c.header}
                </button>
              ) : (
                c.header
              )}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {shown.flatMap((r) => {
          const k = rowKey(r);
          const isOpen = open.has(k);
          const cells = (
            <tr key={k}>
              {expand && (
                <td>
                  <button type="button" className="v-th-button" aria-expanded={isOpen} aria-label={`Details for ${k}`} onClick={() => toggle(k)}>
                    {isOpen ? "Hide" : "Show"}
                  </button>
                </td>
              )}
              {columns.map((c) => (
                <td key={c.key} className={c.numeric ? "v-num" : undefined}>{c.cell(r)}</td>
              ))}
            </tr>
          );
          return expand && isOpen
            ? [cells, <tr key={`${k}-x`}><td colSpan={columns.length + 1}>{expand(r)}</td></tr>]
            : [cells];
        })}
      </tbody>
    </table>
  );
}

import type { ReactNode } from "react";
import { cn } from "../cn";

/**
 * The Graphite icon set (DESIGN.md): one authored set on a 16 px grid, 1.5 px stroke, round caps and joins,
 * `currentColor`. Icons are always decorative here: the text beside them, or the control's label, names the thing.
 * The set holds only icons the app uses; add one when a screen needs it, drawn on the same grid.
 */
export const ICONS = {
  home: <path d="M2.5 7 8 2.5 13.5 7v6.5h-4V10h-3v3.5h-4Z" />,
  run: <path d="M5 3.2v9.6l7.5-4.8Z" />,
  crew: <><circle cx="8" cy="3.6" r="1.6" /><circle cx="3.8" cy="12.4" r="1.6" /><circle cx="12.2" cy="12.4" r="1.6" /><path d="M8 5.2V8M3.8 10.8V8h8.4v2.8" /></>,
  chat: <path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2Z" />,
  inbox: <><path d="M2.5 9.5 4 3.5h8l1.5 6v4h-11Z" /><path d="M2.5 9.5h3.2l.8 1.5h3l.8-1.5h3.2" /></>,
  history: <><path d="M2.8 8a5.2 5.2 0 1 0 1.5-3.7" /><path d="M2.5 2.8v2.4h2.4" /><path d="M8 5.2V8l2 1.3" /></>,
  settings: <><circle cx="8" cy="8" r="2" /><path d="M8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1" /></>,
  search: <><circle cx="7" cy="7" r="4.2" /><path d="m10.2 10.2 3.3 3.3" /></>,
  chevron: <path d="m6 4.5 3.5 3.5L6 11.5" />,
  check: <path d="m3.5 8.3 2.8 2.7 6.2-6.5" />,
  redirect: <><path d="M3 12.5V9a3 3 0 0 1 3-3h7" /><path d="m10.5 3.5 2.5 2.5-2.5 2.5" /></>,
  stop: <rect x="4" y="4" width="8" height="8" rx="1.5" />,
  terminal: <><rect x="1.8" y="2.8" width="12.4" height="10.4" rx="1.5" /><path d="m4.5 6.3 2 1.7-2 1.7M8 10h3.5" /></>,
  file: <><path d="M4 1.8h5l3 3v9.4H4Z" /><path d="M9 1.8v3h3" /></>,
  branch: <><circle cx="4.5" cy="3.8" r="1.5" /><circle cx="4.5" cy="12.2" r="1.5" /><circle cx="11.5" cy="5.5" r="1.5" /><path d="M4.5 5.3v5.4M11.5 7c0 2.5-3 2.6-6.4 4.1" /></>,
} satisfies Record<string, ReactNode>;
export type IconName = keyof typeof ICONS;

export function Icon({ name, size = 16, className }: { name: IconName; size?: 12 | 16; className?: string }) {
  return (
    <svg className={cn("v-icon", className)} width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor"
      strokeWidth={size === 12 ? 2 : 1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {ICONS[name]}
    </svg>
  );
}

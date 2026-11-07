// One authored icon set for Graphite v2: 16 px grid, 1.5 px stroke, round joins, currentColor.
// Replaces the Unicode stand-ins (✓, ▲) that v1 used outside the status vocabulary.
import type { ReactNode } from "react";

function I({ children, size = 16 }: { children: ReactNode; size?: number }) {
  return (
    <svg className="g-ic" width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {children}
    </svg>
  );
}

export const IcHome = () => <I><path d="M2.5 7 8 2.5 13.5 7v6.5h-4V10h-3v3.5h-4Z" /></I>;
export const IcInbox = () => <I><path d="M2.5 9.5 4 3.5h8l1.5 6v4h-11Z" /><path d="M2.5 9.5h3.2l.8 1.5h3l.8-1.5h3.2" /></I>;
export const IcHistory = () => <I><path d="M2.8 8a5.2 5.2 0 1 0 1.5-3.7" /><path d="M2.5 2.8v2.4h2.4" /><path d="M8 5.2V8l2 1.3" /></I>;
export const IcSettings = () => <I><circle cx="8" cy="8" r="2" /><path d="M8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1" /></I>;
export const IcSearch = () => <I><circle cx="7" cy="7" r="4.2" /><path d="m10.2 10.2 3.3 3.3" /></I>;
export const IcCheck = () => <I><path d="m3.5 8.3 2.8 2.7 6.2-6.5" /></I>;
export const IcChevron = () => <I size={12}><path d="m4.5 6 3.5 3.5L11.5 6" /></I>;
export const IcList = () => <I><path d="M5.5 4h8M5.5 8h8M5.5 12h8" /><path d="M2.5 4h.01M2.5 8h.01M2.5 12h.01" /></I>;
export const IcBoard = () => <I><rect x="2.5" y="2.5" width="3.2" height="11" rx="1" /><rect x="6.4" y="2.5" width="3.2" height="7" rx="1" /><rect x="10.3" y="2.5" width="3.2" height="9" rx="1" /></I>;
export const IcMessage = () => <I><path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2Z" /></I>;
export const IcRedirect = () => <I><path d="M3 12.5V9a3 3 0 0 1 3-3h7" /><path d="m10.5 3.5 2.5 2.5-2.5 2.5" /></I>;
export const IcStop = () => <I><rect x="4" y="4" width="8" height="8" rx="1.5" /></I>;
export const IcFile = () => <I><path d="M4 1.8h5l3 3v9.4H4Z" /><path d="M9 1.8v3h3" /></I>;
export const IcTerminal = () => <I><rect x="1.8" y="2.8" width="12.4" height="10.4" rx="1.5" /><path d="m4.5 6.3 2 1.7-2 1.7M8 10h3.5" /></I>;
export const IcFlask = () => <I><path d="M6 1.8h4M6.6 1.8v4L3 12.6a1 1 0 0 0 .9 1.6h8.2a1 1 0 0 0 .9-1.6L9.4 5.8v-4" /><path d="M4.7 9.6h6.6" /></I>;
export const IcTarget = () => <I><circle cx="8" cy="8" r="5.5" /><circle cx="8" cy="8" r="2.2" /></I>;
export const IcAt = () => <I><circle cx="8" cy="8" r="2.3" /><path d="M10.3 8v1a1.7 1.7 0 0 0 3.4 0V8A5.7 5.7 0 1 0 11 12.9" /></I>;
export const IcShield = () => <I><path d="M8 1.8 13 3.6v4c0 3.1-2.1 5.4-5 6.6-2.9-1.2-5-3.5-5-6.6v-4Z" /></I>;
export const IcBranch = () => <I><circle cx="4.5" cy="3.8" r="1.5" /><circle cx="4.5" cy="12.2" r="1.5" /><circle cx="11.5" cy="5.5" r="1.5" /><path d="M4.5 5.3v5.4M11.5 7c0 2.5-3 2.6-6.4 4.1" /></I>;
export const IcPlus = () => <I><path d="M8 3v10M3 8h10" /></I>;

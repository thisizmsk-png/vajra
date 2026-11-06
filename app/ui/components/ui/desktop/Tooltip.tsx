import { cloneElement, useEffect, useId, useRef, useState } from "react";
import type { ReactElement } from "react";

/** Shows after 400 ms of hover or focus; Esc hides it. It describes the child (aria-describedby) and is never the only source of the information. */
export function Tooltip({ text, children }: { text: string; children: ReactElement<{ "aria-describedby"?: string }> }) {
  const id = useId();
  const [show, setShow] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const on = () => { clearTimeout(timer.current); timer.current = setTimeout(() => setShow(true), 400); };
  const off = () => { clearTimeout(timer.current); setShow(false); };
  useEffect(() => () => clearTimeout(timer.current), []);
  const own = children.props["aria-describedby"];
  const child = cloneElement(children, { "aria-describedby": [own, show && id].filter(Boolean).join(" ") || undefined }); // keep the child's own
  return (
    <span className="v-tip-wrap" onMouseEnter={on} onMouseLeave={off} onFocus={on} onBlur={off} onKeyDown={(e) => { if (e.key === "Escape" && show) { e.stopPropagation(); off(); } }}>
      {child}
      {show && (
        <span role="tooltip" id={id} className="v-tooltip">
          {text}
        </span>
      )}
    </span>
  );
}

import { CrewG } from "./Crew";
import { Kbd } from "../shared";
import { history, run } from "../data";
import { IcFile, IcHistory, IcRedirect, IcSearch, IcTerminal } from "./icons";
import type { ReactNode } from "react";

/** ⌘K over the Crew view: one field for actions, runs and files (Raycast, AO). The query is "re". */
function Opt({ icon, children, d, keys, sel }: { icon: ReactNode; children: ReactNode; d?: string; keys?: string[]; sel?: boolean }) {
  return (
    <div className="opt" role="option" aria-selected={sel ? "true" : "false"}>
      {icon}
      <span>{children}{d ? <span className="d">{d}</span> : null}</span>
      {keys ? <span className="g-keys">{keys.map((k) => <Kbd key={k}>{k}</Kbd>)}</span> : <span />}
    </div>
  );
}

export function PaletteG() {
  const past = history[0];
  return (
    <>
      <CrewG inert />
      <div className="g g-scrim" aria-hidden="true" />
      <div className="g g-pal" role="dialog" aria-modal="true" aria-label="Commands">
        <div className="in">
          <IcSearch />
          <span className="q" role="combobox" aria-expanded="true" aria-controls="g-pal-list" aria-label="Search or run a command">re</span>
          <span className="scope">Backend</span>
        </div>
        <div id="g-pal-list" role="listbox" aria-label="Results">
          <div className="grp">Actions for Backend</div>
          <Opt icon={<IcRedirect />} keys={["R"]} sel><mark>Re</mark>direct Backend</Opt>
          <Opt icon={<IcFile />} keys={["D"]} d="4 files"><mark>Re</mark>ad Backend's diff</Opt>
          <div className="grp">This run</div>
          <Opt icon={<IcTerminal />} keys={["⌘", "↵"]} d={`${run.title}`}><mark>Re</mark>view plan change</Opt>
          <Opt icon={<IcTerminal />}><mark>Re</mark>ject plan v2</Opt>
          <div className="grp">Files and history</div>
          <Opt icon={<IcFile />} d="Backend, +117 −8">src/middleware/<mark>r</mark>at<mark>e</mark>Limit.ts</Opt>
          <Opt icon={<IcHistory />} d={past.outcome}>Fix login <mark>re</mark>direct loop on expired sessions</Opt>
        </div>
        <div className="ft">
          <span><Kbd>↑</Kbd><Kbd>↓</Kbd> Move</span>
          <span><Kbd>↵</Kbd> Run</span>
          <span><Kbd>⇥</Kbd> Change scope</span>
          <span className="r"><Kbd>esc</Kbd> Close</span>
        </div>
      </div>
    </>
  );
}

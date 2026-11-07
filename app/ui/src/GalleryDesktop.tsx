import { useState } from "react";
import { Section, Button, Text } from "../components/ui/base";
import { ActionCard, CommandPalette, CrewTree, DiffView, EvidenceRow, ResizablePanes, Table, Tabs, TerminalPane, Tooltip } from "../components/ui/desktop";
import type { CrewNode } from "../components/ui/desktop";

const E = "\u001b";
const LINES = [
  `${E}[1mvajra-deliver${E}[0m run 20261106-0709 started`,
  `${E}[32m✓${E}[0m preflight passed`,
  `${E}[33mwarn${E}[0m base is 3 commits behind origin`,
  `${E}[31mfail${E}[0m ac-2: npm test exited 1`,
  `hostile line: ${E}]8;;https://evil.example${E}\\a link${E}]8;;${E}\\ ${E}]52;c;ZXZpbA==\u0007 ${E}[2J<img src=x onerror=alert(1)>`,
  ...Array.from({ length: 2000 }, (_, i) => `step ${i} ${E}[90mlog line${E}[0m`),
];
const DIFF = "diff --git a/scratch.txt b/scratch.txt\n--- a/scratch.txt\n+++ b/scratch.txt\n@@ -1 +1,3 @@\n scratch\n+step1\n+step2\ndiff --git a/README.md b/README.md\n@@ -1 +1 @@\n-old\n+new";
const TREE: CrewNode[] = [{ id: "lead", label: "Lead", status: "in_progress", children: [{ id: "be", label: "Backend", status: "in_progress", children: [{ id: "w1", label: "Backend worker 1", status: "done" }] }, { id: "fe", label: "Frontend", status: "blocked" }] }];

/** The K3 desktop kit in use, for review and the click crawl (open with #kit). Sample data only; not an app screen. */
export function GalleryDesktop() {
  const [tab, setTab] = useState<"task" | "diff" | "log">("log");
  const [sel, setSel] = useState("be");
  const [palette, setPalette] = useState(false);
  const [note, setNote] = useState("");
  return (
    <>
      <Section title="Tabs and terminal">
        <Tabs label="Worker" value={tab} onChange={setTab} tabs={[
          { id: "task", label: "Task", panel: <Text variant="body">Add eight lines to scratch.txt.</Text> },
          { id: "diff", label: "Diff", panel: <DiffView diff={DIFF} height={160} /> },
          { id: "log", label: "Log", panel: <TerminalPane lines={LINES} height={200} /> },
        ]} />
      </Section>
      <Section title="Crew and panes">
        <div className="v-gallery__panel">
          <ResizablePanes initial={220} min={160} max={320} label="Resize crew tree"
            first={<CrewTree label="Crew" nodes={TREE} selected={sel} onSelect={setSel} defaultOpen={["lead", "be"]} />}
            second={<Text variant="body">Selected: {sel}</Text>} />
        </div>
      </Section>
      <Section title="Criteria and actions">
        <Table caption="Criteria" rowKey={(r) => r.id} rows={[{ id: "ac-1", ok: true }, { id: "ac-2", ok: false }]}
          columns={[{ key: "id", header: "Criterion", sort: (a, b) => a.id.localeCompare(b.id), cell: (r) => r.id }, { key: "ok", header: "Result", cell: (r) => (r.ok ? "Pass" : "Fail") }]}
          expand={(r) => `Evidence for ${r.id}`} />
        <table>
          <caption className="v-sr">Evidence</caption>
          <thead><tr><th scope="col">Criterion</th><th scope="col">Result</th><th scope="col">Command</th><th scope="col">Exit</th><th scope="col">Evidence</th><th scope="col">Commit</th></tr></thead>
          <tbody>
          <EvidenceRow id="ac-1" state="pass" command="grep -c step scratch.txt" exitCode={0} evidenceFile="evidence.json" commit="4c70bf04d064" />
          <EvidenceRow id="ac-2" state="fail" command="npm test" exitCode={1} />
        </tbody>
        </table>
        <div className="v-gallery__gap"><ActionCard kind="redirect" worker="Backend" by="Lead" text="Use the kit Button" state="delivered" time="14:22" onOpenWorker={() => setNote("worker")} onOpenAudit={() => setNote("audit")} /></div>
        <div className="v-gallery__flex v-gallery__center v-gallery__gap">
          <Tooltip text="Open the command palette (Cmd K)"><Button onClick={() => setPalette(true)}>Commands</Button></Tooltip>
          <Text variant="body-sm" muted>{note}</Text>
        </div>
      </Section>
      <CommandPalette open={palette} onClose={() => setPalette(false)} commands={[
        { id: "start", label: "Start run", run: () => setNote("Start run") },
        { id: "stop", label: "Stop worker", run: () => setNote("Stop worker") },
        { id: "settings", label: "Open settings", hint: "Cmd ,", run: () => setNote("Open settings") },
      ]} />
    </>
  );
}

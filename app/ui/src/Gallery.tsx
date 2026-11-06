import { useState } from "react";
import {
  AlertDialog, Badge, Button, Card, Input, ListBox, ListRow, SegmentedControl, Section, Separator, Sheet, Skeleton, StatusDot, Switch, Text, ToastRegion,
  useToasts,
} from "../components/ui/base";
import { S } from "../strings";

/** Every base component in its states, for review and the click crawl (open with #kit). Not an app screen. */
export function Gallery() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [rounds, setRounds] = useState<"1" | "2" | "3">("2");
  const [on, setOn] = useState(true);
  const [text, setText] = useState("main");
  const [sheet, setSheet] = useState(false);
  const [stop, setStop] = useState(false);
  const [sel, setSel] = useState(0);
  const { toasts, push, dismiss } = useToasts();
  document.documentElement.dataset.theme = theme;
  return (
    <main style={{ padding: 24, display: "grid", gap: 16, maxWidth: 720 }}>
      <Text as="h1" variant="headline-lg">Kit gallery</Text>
      <SegmentedControl label="Theme" value={theme} onChange={setTheme} options={[{ value: "dark", label: "Dark" }, { value: "light", label: "Light" }]} />
      <Section title="Buttons">
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <Button variant="primary">Start run</Button>
          <Button>Message</Button>
          <Button variant="ghost">Cancel</Button>
          <Button variant="danger" onClick={() => setStop(true)}>Stop worker</Button>
          <Button disabled>Disabled</Button>
          <Button busy>Stopping</Button>
          <Button size="sm">Small</Button>
        </div>
      </Section>
      <Section title="Status">
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          {(Object.keys(S.status) as Array<keyof typeof S.status>).map((k) => <StatusDot key={k} status={k} />)}
        </div>
      </Section>
      <Section title="Badges">
        <div style={{ display: "flex", gap: 8 }}>
          <Badge>Default</Badge><Badge status="success">Pass</Badge><Badge status="warning">Needs approval</Badge><Badge status="danger">Fail</Badge><Badge status="info">Ready</Badge>
        </div>
      </Section>
      <Section title="Inputs">
        <div style={{ display: "grid", gap: 12 }}>
          <Input label="Base branch" hint="A branch or commit" value={text} onChange={setText} />
          <Input label="Reason" error="Required" type="textarea" value="" onChange={() => {}} />
          <Input label="Engine" type="select" value="claude" options={[{ value: "claude", label: "Claude Code" }, { value: "codex", label: "Codex" }]} onChange={() => {}} />
          <SegmentedControl label="Max rounds" value={rounds} onChange={setRounds} options={[{ value: "1", label: "1" }, { value: "2", label: "2" }, { value: "3", label: "3" }]} />
          <div style={{ display: "flex", gap: 8, alignItems: "center" }}><Switch label="Notifications" checked={on} onChange={setOn} /><Text variant="body">Notifications</Text></div>
        </div>
      </Section>
      <Section title="List">
        <ListBox label="Runs">
          {["Add dark mode", "Fix export flag", "Refactor bridge"].map((r, i) => (
            <ListRow key={r} selected={sel === i} onSelect={() => setSel(i)} meta={`${i + 1} of 7`}>{r}</ListRow>
          ))}
        </ListBox>
        <Separator />
        <Card><Text variant="body">Plain card</Text><Text variant="body-sm">Secondary text</Text></Card>
        <div style={{ marginTop: 8 }}><Card interactive onClick={() => push("info", "Opened run")}><Text variant="label">Interactive card</Text></Card></div>
      </Section>
      <Section title="Layers and feedback">
        <div style={{ display: "flex", gap: 8 }}>
          <Button onClick={() => setSheet(true)}>Open sheet</Button>
          <Button onClick={() => push("success", "Stopped Backend at 14:22")}>Success toast</Button>
          <Button onClick={() => push("danger", "Could not write the queue entry")}>Error toast</Button>
        </div>
        <div style={{ marginTop: 8, display: "grid", gap: 8 }}><Skeleton delay={0} /><Skeleton delay={0} variant="block" /></div>
      </Section>
      <Sheet open={sheet} onClose={() => setSheet(false)} title="Message Backend" side="right">
        <Input label="Message" type="textarea" value="" onChange={() => {}} />
        <div className="v-dialog__actions"><Button onClick={() => setSheet(false)}>Close</Button></div>
      </Sheet>
      <AlertDialog open={stop} destructive title="Stop Backend?" confirmLabel="Stop worker" onCancel={() => setStop(false)} onConfirm={() => { setStop(false); push("success", "Stopped Backend at 14:22"); }}>
        <Text variant="body">It stops at its next tool call.</Text>
      </AlertDialog>
      <ToastRegion toasts={toasts} onDismiss={dismiss} />
    </main>
  );
}

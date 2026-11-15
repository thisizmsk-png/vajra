// Settings > General: the disk the worker copies use, and Clean up (ADR 16.6 Retention, "Disk").
// The size is a walk of the copies' folder; Clean up removes only copies the core proves clean (merged, nothing else).
import { useCallback, useEffect, useState } from "react";
import { Button, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText, newActionId } from "../../ipc";
import type { WorkerDisk } from "../../ipc";
import { LoadingRows, Problem, problemDetail } from "../states";
import "./preferences.css";

const C = S.settings.copies;

/** Bytes as a short size: 1 decimal from MB up. */
export function size(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB", "TB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1; }
  return i === 0 ? `${v} ${units[0]}` : `${v.toFixed(i >= 2 ? 1 : 0)} ${units[i]}`;
}

export function WorkerCopies() {
  const [disk, setDisk] = useState<WorkerDisk | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");
  const load = useCallback(() => {
    setErr("");
    api.workerDisk().then(setDisk, (e) => setErr(problemDetail(e)));
  }, []);
  useEffect(load, [load]);
  const clean = () => {
    setBusy(true);
    setNote("");
    api.actWorkerCleanUp({ action_id: newActionId(), project_id: "global", expect_generation: null })
      .then((r) => {
        if (r.error || !r.result) { setNote(`${C.cleanFailed} ${r.error ? errorText(r.error) : ""}`.trim()); return; }
        setNote(C.cleaned(r.result.removed.length));
        setDisk(r.result.disk);
      }, (e) => setNote(`${C.cleanFailed} ${errorText(e)}`))
      .finally(() => setBusy(false));
  };
  return (
    <section aria-labelledby="v-settings-copies" className="v-panel v-settings__checks">
      <div className="v-panel__bar">
        <h2 id="v-settings-copies" className="v-panel__title">{C.title}</h2>
      </div>
      <div className="v-panel__body v-copies">
        <Text variant="body" muted>{C.lede}</Text>
        {err && <Problem title={C.failed} detail={err} retry={load} />}
        {!disk && !err && <LoadingRows rows={1} label={C.checking} />}
        {disk && (
          <>
            <Text variant="body" numeric>{disk.copies === 0 ? C.none : C.use(disk.copies, size(disk.bytes), disk.partial)}</Text>
            {disk.copies > 0 && <Text variant="body" muted>{C.shared}</Text>}
            <div className="v-copies__act">
              <Button size="sm" busy={busy} disabled={disk.copies === 0} aria-describedby="v-copies-hint" onClick={clean}>{C.cleanUp}</Button>
              <span id="v-copies-hint"><Text as="span" variant="body-sm" muted>{C.cleanUpHint}</Text></span>
            </div>
          </>
        )}
        {/* Always mounted, so the result is announced when it arrives. */}
        <p role="status" className="v-copies__note">{note}</p>
      </div>
    </section>
  );
}

import { useEffect, useState } from "react";
import { Button, Sheet, Text } from "../../components/ui/base";
import { api } from "../ipc";
import { S } from "../../strings";

type Found = { root: string; name: string; worktrees: number };
const W = S.frame.welcome;

/**
 * First-launch help: when there are no projects, look (read only) for git repositories already on this Mac and offer to add
 * them. Shows nothing when none are found or the look fails; adding is the person's pick, one `project_add` each.
 */
export function FindRepos({ onAdded }: { onAdded: () => void }) {
  const [found, setFound] = useState<Found[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [open, setOpen] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");

  useEffect(() => {
    let live = true;
    api.projectDiscover().then(
      (r) => {
        if (live && Array.isArray(r?.found)) {
          setFound(r.found);
          setTruncated(r.truncated === true);
        }
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, []);

  if (found.length === 0) return null;
  const show = () => {
    setPicked(new Set(found.map((f) => f.root)));
    setNote("");
    setOpen(true);
  };
  const toggle = (root: string) =>
    setPicked((p) => {
      const n = new Set(p);
      if (n.has(root)) n.delete(root);
      else n.add(root);
      return n;
    });
  const add = async () => {
    setBusy(true);
    let ok = 0;
    const left: Found[] = [];
    let bad = 0;
    for (const f of found) {
      if (!picked.has(f.root)) {
        left.push(f);
        continue;
      }
      try {
        await api.projectAdd(f.root);
        ok++;
      } catch {
        bad++;
        left.push(f);
      }
    }
    setBusy(false);
    setFound(left);
    if (ok > 0) onAdded();
    if (bad > 0) {
      setPicked(new Set(left.map((f) => f.root).filter((r) => picked.has(r))));
      setNote(W.partial(ok, bad));
    } else setOpen(false);
  };

  return (
    <>
      <button type="button" className="v-tb__add" onClick={show}>
        {W.offerShort(found.length)}
      </button>
      <Sheet open={open} onClose={() => !busy && setOpen(false)} title={W.title}>
        <Text>{W.offer(found.length)}</Text>
        <Text variant="body-sm" muted>
          {W.intro}
        </Text>
        <div className="v-find__bar">
          <Button variant="ghost" size="sm" onClick={() => setPicked(new Set(found.map((f) => f.root)))}>
            {W.all}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setPicked(new Set())}>
            {W.none}
          </Button>
        </div>
        <ul className="v-find__list">
          {found.map((f) => (
            <li key={f.root}>
              <label className="v-find__row">
                <input type="checkbox" checked={picked.has(f.root)} onChange={() => toggle(f.root)} />
                <span className="v-find__name">{f.name}</span>
                <span className="v-find__path">{f.root}</span>
                {f.worktrees > 0 && <span className="v-find__wt">{W.worktrees(f.worktrees)}</span>}
              </label>
            </li>
          ))}
        </ul>
        {truncated && (
          <Text variant="body-sm" muted>
            {W.truncated}
          </Text>
        )}
        {note && <div role="status">{note}</div>}
        <div className="v-find__bar">
          <Button variant="ghost" onClick={() => setOpen(false)} disabled={busy}>
            {W.cancel}
          </Button>
          <Button variant="primary" onClick={() => void add()} disabled={busy || picked.size === 0}>
            {W.add(picked.size)}
          </Button>
        </div>
      </Sheet>
    </>
  );
}

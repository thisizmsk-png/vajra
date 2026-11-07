import { createContext, useCallback, useContext, useEffect, useId, useRef, useState } from "react";
import type { MutableRefObject, ReactNode } from "react";
import { CommandPalette } from "../components/ui/desktop";
import type { Command } from "../components/ui/desktop";

type Source = MutableRefObject<readonly Command[]>;
type Register = (key: string, source: Source | null) => void;
const Ctx = createContext<Register>(() => {});

/**
 * Adds a screen's commands to the ⌘K palette while the screen is mounted. Only commands the screen can really run
 * belong here. The palette reads the latest list when it opens, so `run` may close over fresh state.
 */
export function useCommands(commands: readonly Command[]) {
  const register = useContext(Ctx);
  const key = useId();
  const ref = useRef(commands);
  ref.current = commands;
  // Re-register only when what the palette shows changes, not on every render.
  const shape = commands.map((c) => [c.id, c.label, c.hint ?? "", (c.keys ?? []).join(" ")].join("\u0000")).join("\u0001");
  useEffect(() => {
    register(key, ref);
    return () => register(key, null);
  }, [register, key, shape]);
}

/** ⌘K opens the palette on every screen, ⌘K or Esc closes it. `commands` are the shell's own; screens add theirs. */
export function PaletteHost({ commands, children }: { commands: readonly Command[]; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [sources, setSources] = useState<ReadonlyMap<string, Source>>(new Map());
  const register = useCallback<Register>((key, source) => {
    setSources((m) => {
      const next = new Map(m);
      if (source) next.set(key, source);
      else next.delete(key);
      return next;
    });
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.shiftKey || e.altKey || e.ctrlKey || e.isComposing || e.key.toLowerCase() !== "k") return;
      e.preventDefault();
      setOpen((o) => !o);
    };
    // A screen key (⌘1 to ⌘7) moves on while the palette is open; the palette then belongs to the old screen.
    const onRoute = () => setOpen(false);
    window.addEventListener("keydown", onKey);
    window.addEventListener("hashchange", onRoute);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("hashchange", onRoute);
    };
  }, []);
  // Screen commands first (they are about what is on screen), then the shell's. Ids are namespaced per source.
  const all: Command[] = [];
  let i = 0;
  for (const src of sources.values()) {
    const n = i++;
    for (const c of src.current) all.push({ ...c, id: `s${n}-${c.id}` });
  }
  for (const c of commands) all.push({ ...c, id: `shell-${c.id}` });
  return (
    <Ctx.Provider value={register}>
      {children}
      <CommandPalette open={open} onClose={() => setOpen(false)} commands={all} />
    </Ctx.Provider>
  );
}

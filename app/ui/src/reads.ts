// One owner per expensive read. Every caller of a keyed read while one is in flight gets that same promise, and a
// caller within `freshMs` of the last answer gets the last value, so a burst of `changed` events or two screens asking
// for the same thing costs one call to the core. A failed read is never kept: the next caller asks again.

type Entry = { at: number; value?: unknown; flight?: Promise<unknown> };
const entries = new Map<string, Entry>();
const counts = { hits: 0, misses: 0 };

export function ownedRead<T>(key: string, read: () => Promise<T>, freshMs = 0): Promise<T> {
  const e = entries.get(key);
  if (e?.flight) { counts.hits += 1; return e.flight as Promise<T>; }
  if (e && freshMs > 0 && "value" in e && Date.now() - e.at < freshMs) { counts.hits += 1; return Promise.resolve(e.value as T); }
  counts.misses += 1;
  // A flight that was forgotten or replaced while it ran still answers its own callers, but never becomes the kept value.
  const mine: Entry = { at: e?.at ?? 0, value: e?.value };
  const flight: Promise<T> = read().then(
    (value) => { if (entries.get(key) === mine) entries.set(key, { at: Date.now(), value }); return value; },
    (err) => { if (entries.get(key) === mine) entries.delete(key); throw err; },
  );
  mine.flight = flight;
  entries.set(key, mine);
  return flight;
}

/** Drop a read's kept value, so the next caller asks the core (an action that changed it calls this). */
export function forgetRead(key?: string) {
  if (key === undefined) entries.clear();
  else entries.delete(key);
}

/** Hits and misses so far, for a debug panel or a test. */
export const readCounts = () => ({ ...counts, kept: entries.size });

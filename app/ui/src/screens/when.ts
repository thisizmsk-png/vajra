/** "11 hours ago" for a stored timestamp, in the reader's locale; the exact time stays in the title. An unparseable
 * value is shown as it is. Stored values are "YYYY-MM-DD HH:MM" (local) or ISO 8601. */
const STEPS: [Intl.RelativeTimeFormatUnit, number][] = [["second", 60], ["minute", 60], ["hour", 24], ["day", 7], ["week", 4.35], ["month", 12], ["year", Infinity]];

export function parseWhen(v: string): number {
  const t = Date.parse(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(v) ? v.replace(" ", "T") : v);
  return Number.isNaN(t) ? NaN : t;
}

export function ago(v: string | undefined, now = Date.now()): string {
  if (!v) return "";
  const t = parseWhen(v);
  if (Number.isNaN(t)) return v;
  let d = (t - now) / 1000;
  const fmt = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  for (const [unit, size] of STEPS) {
    if (Math.abs(d) < size) return fmt.format(Math.round(d), unit);
    d /= size;
  }
  return v;
}

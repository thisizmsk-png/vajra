/** Nearest-rank percentile of a list of numbers; null for an empty list. */
export function pct(xs, p) {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.min(s.length - 1, Math.ceil((p / 100) * s.length) - 1)] : null;
}

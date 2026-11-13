// What the typing-lag gate checks. The runner's own rendering costs about 69 ms at p95 with nothing streaming (CI, 13 Nov),
// so an absolute 50 ms budget measures the VM, not us. The gate checks what streaming ADDS to the idle control instead.
export const BUDGET_DELTA_P95_MS = 20;
/** The old absolute budget, checked only with --absolute (a fast machine should meet it). */
export const BUDGET_P95_MS = 50;
// Ceilings that are always on. The delta cannot see a cost in the composer's own keystroke path (it lands in the streaming
// pass and the idle control alike), so these catch it. Set from the runner, not a laptop: the runner's idle control was
// 68.8 ms and streaming 189.2 ms on the first run (13 Nov), so ~2x the idle figure and a bound a little over the streaming one.
export const CEILING_IDLE_P95_MS = 150;
export const CEILING_STREAMING_P95_MS = 250;

/**
 * @param {{keystrokes:number, painted:number, typed:number, p95_ms:number}} streaming
 * @param {{p95_ms:number}} idle
 * @param {{absolute?:boolean}} [o]
 * @returns {{ok:boolean, delta_p95_ms:number, reasons:string[]}}
 */
export function verdict(streaming, idle, o = {}) {
  const delta = +(streaming.p95_ms - idle.p95_ms).toFixed(1);
  const reasons = [];
  if (streaming.painted !== streaming.keystrokes || streaming.typed !== streaming.keystrokes) reasons.push(`only ${streaming.painted}/${streaming.keystrokes} keys painted, ${streaming.typed} typed`);
  if (delta > BUDGET_DELTA_P95_MS) reasons.push(`streaming adds ${delta} ms at p95 over the idle control, budget ${BUDGET_DELTA_P95_MS} ms`);
  if (idle.p95_ms > CEILING_IDLE_P95_MS) reasons.push(`idle control p95 ${idle.p95_ms} ms over the ceiling ${CEILING_IDLE_P95_MS} ms: typing is slow with nothing streaming`);
  if (streaming.p95_ms > CEILING_STREAMING_P95_MS) reasons.push(`streaming p95 ${streaming.p95_ms} ms over the ceiling ${CEILING_STREAMING_P95_MS} ms`);
  if (o.absolute && streaming.p95_ms > BUDGET_P95_MS) reasons.push(`p95 ${streaming.p95_ms} ms over the absolute budget ${BUDGET_P95_MS} ms`);
  return { ok: reasons.length === 0, delta_p95_ms: delta, reasons };
}

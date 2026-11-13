// What the typing-lag gate checks. The runner's own rendering costs about 69 ms at p95 with nothing streaming (CI, 13 Nov),
// so an absolute 50 ms budget measures the VM, not us. The gate checks what streaming ADDS to the idle control instead.
export const BUDGET_DELTA_P95_MS = 20;
/** The old absolute budget, still printed and still checked with --absolute (a fast machine should meet it). */
export const BUDGET_P95_MS = 50;

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
  if (o.absolute && streaming.p95_ms > BUDGET_P95_MS) reasons.push(`p95 ${streaming.p95_ms} ms over the absolute budget ${BUDGET_P95_MS} ms`);
  return { ok: reasons.length === 0, delta_p95_ms: delta, reasons };
}

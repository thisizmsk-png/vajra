import type { RunInput } from "../../ipc";
import { S } from "../../../strings";

/** The shortest ask a run starts with: the deliver workflow refuses less, and the core's start check says the same
 *  (`MIN_ASK_CHARS`). A test pins this to the workflow's own number. */
export const MIN_ASK = 10;

export type Draft = { repo: string; ask: string; base: string; rounds: string; maxInFlight: string };
export type Errors = Partial<Record<keyof Draft, string>>;

const int = (s: string) => (/^\d+$/.test(s.trim()) ? Number(s.trim()) : NaN);

/** The same limits the core enforces (1..3 rounds, 1..16 workers, non-empty ask), so a bad value never leaves the form. */
export function validate(d: Draft): Errors {
  const e: Errors = {};
  if (!d.repo.trim()) e.repo = S.home.errRepo;
  if (!d.ask.trim()) e.ask = S.home.errAsk;
  else if (Array.from(d.ask.trim()).length < MIN_ASK) e.ask = S.home.errAskShort(MIN_ASK);
  if (!d.base.trim()) e.base = S.home.errBase;
  const r = int(d.rounds);
  if (!(r >= 1 && r <= 3)) e.rounds = S.home.errRounds;
  const f = int(d.maxInFlight);
  if (!(f >= 1 && f <= 16)) e.maxInFlight = S.home.errInFlight;
  return e;
}

export const toInput = (d: Draft): RunInput => ({
  ask: d.ask,
  maxRounds: int(d.rounds),
  base: d.base.trim(),
  maxInFlight: int(d.maxInFlight),
  campaign: null,
});

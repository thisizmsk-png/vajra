import type { RunInput } from "../../ipc";
import { S } from "../../../strings";

export type Draft = { repo: string; ask: string; base: string; rounds: string; maxInFlight: string };
export type Errors = Partial<Record<keyof Draft, string>>;

const int = (s: string) => (/^\d+$/.test(s.trim()) ? Number(s.trim()) : NaN);

/** The same limits the core enforces (1..3 rounds, 1..16 workers, non-empty ask), so a bad value never leaves the form. */
export function validate(d: Draft): Errors {
  const e: Errors = {};
  if (!d.repo.trim()) e.repo = S.home.errRepo;
  if (!d.ask.trim()) e.ask = S.home.errAsk;
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

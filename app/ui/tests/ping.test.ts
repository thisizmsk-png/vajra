import { describe, expect, it } from "vitest";
import { EMPTY_PING_STATE, HOUR_MS, arrive, flush, groupOf } from "../src/projects/ping";
import type { PingEntry, PingState } from "../src/projects/ping";
import { DEFAULT_PING } from "../src/preferenceContract";
import type { PingRule } from "../src/preferenceContract";

const names: Record<string, string> = { p1: "harbor-api", p2: "harbor-web" };
const name = (id: string) => names[id] ?? "Unsorted";
const MIN = 60_000;

/** Feed arrivals at the given times, firing the timer whenever a ping is due before the next one; returns every ping. */
function play(rule: PingRule, events: [number, string, string, number?][], until: number, notify = true) {
  let s: PingState = EMPTY_PING_STATE;
  const pings: PingEntry[] = [];
  const quiet: PingEntry[] = [];
  const fireUntil = (t: number) => {
    while (s.due !== null && s.due <= t) {
      const r = flush(s, s.due, rule, name);
      s = r.state;
      if (r.ping) pings.push(r.ping);
    }
  };
  for (const [at, project, kind, count] of events) {
    fireUntil(at);
    const r = arrive(s, { project_id: project, kind, count: count ?? 1 }, at, rule, notify, name);
    s = r.state;
    if (r.quiet) quiet.push(r.quiet);
  }
  fireUntil(until);
  return { pings, quiet, state: s };
}

describe("the ping rule", () => {
  it("maps the core's arrival kinds to the rule's three switches, and ignores anything else", () => {
    expect([groupOf("plan"), groupOf("run_proposal"), groupOf("question"), groupOf("blocked"), groupOf("failed"), groupOf("done")])
      .toEqual(["decisions", "decisions", "questions", "blocked", "blocked", null]);
  });

  // Given 5 tasks blocking within 2 minutes, Then the person gets 1 ping, not 5, and it says why.
  it("groups arrivals within the batch window into one ping that says why", () => {
    const { pings } = play(DEFAULT_PING, [[0, "p1", "blocked"], [20_000, "p1", "blocked"], [40_000, "p2", "plan"], [60_000, "p1", "blocked"], [100_000, "p1", "question"]], 10 * MIN);
    expect(pings).toHaveLength(1);
    expect(pings[0].at).toBe(2 * MIN);
    expect(pings[0].text).toBe("5 things need you: 3 blocked tasks and 1 question in harbor-api; 1 decision in harbor-web");
    expect(pings[0].why).toBe("Pinged because your rule is on for decisions, questions and blocked tasks. Grouped: they arrived within 2 min of the first.");
  });

  it("names the one thing when one thing arrives, and pings at once with no grouping", () => {
    const { pings } = play({ ...DEFAULT_PING, batch_minutes: 0 }, [[5, "p2", "question"]], 10);
    expect(pings).toEqual([{ at: 5, pinged: true, text: "A worker has a question in harbor-web", why: "Pinged because your rule is on for questions." }]);
  });

  // Given a budget of 2 an hour, When 4 separate arrivals come, Then 2 ping, and the rest wait and leave as one held ping.
  it("holds pings past the hourly budget and says they were held", () => {
    const rule = { ...DEFAULT_PING, per_hour: 2, batch_minutes: 0 };
    const { pings, state } = play(rule, [[0, "p1", "plan"], [10 * MIN, "p1", "plan"], [20 * MIN, "p1", "question"], [30 * MIN, "p2", "blocked"]], 2 * HOUR_MS);
    expect(pings.map((p) => p.at)).toEqual([0, 10 * MIN, HOUR_MS]);
    expect(pings[2].text).toBe("2 things need you: 1 question in harbor-api; 1 blocked task in harbor-web");
    expect(pings[2].why).toContain("Held until now: you allow 2 pings an hour.");
    expect(state.held).toBe(false);
  });

  it("never pings more than the budget in any hour", () => {
    const rule = { ...DEFAULT_PING, per_hour: 3, batch_minutes: 0 };
    const events: [number, string, string][] = Array.from({ length: 40 }, (_, i) => [i * 3 * MIN, "p1", "plan"]);
    const { pings } = play(rule, events, 3 * HOUR_MS);
    for (const p of pings) expect(pings.filter((q) => q.at > p.at - HOUR_MS && q.at <= p.at).length).toBeLessThanOrEqual(3);
    // Nothing was lost: every arrival was told by some ping.
    const told = pings.reduce((n, p) => n + (Number(/^(\d+) things/.exec(p.text)?.[1]) || 1), 0);
    expect(told).toBe(40);
  });

  it("keeps a kind the rule turns off quiet, and says so", () => {
    const { pings, quiet } = play({ ...DEFAULT_PING, questions: false }, [[0, "p1", "question"]], HOUR_MS);
    expect(pings).toEqual([]);
    expect(quiet).toEqual([{ at: 0, pinged: false, text: "A worker has a question in harbor-api", why: "Not pinged: your rule is off for questions." }]);
  });

  it("pings nothing when notifications are off, and logs why", () => {
    const { pings, quiet } = play(DEFAULT_PING, [[0, "p1", "plan", 2]], HOUR_MS, false);
    expect(pings).toEqual([]);
    expect(quiet[0].why).toBe("Not pinged: notifications are off.");
    expect(quiet[0].text).toBe("2 things need you: 2 decisions in harbor-api");
  });

  it("is told counts and project names only, so an unknown project reads as Unsorted", () => {
    const { pings } = play({ ...DEFAULT_PING, batch_minutes: 0 }, [[0, "p_gone", "plan"]], 1);
    expect(pings[0].text).toBe("A decision needs you in Unsorted");
  });
});

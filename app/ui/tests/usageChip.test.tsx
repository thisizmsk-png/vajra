// The usage chip in the status bar (H5): shown only at 80% or more, with the reset time if the source gave one.
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { StatusBar } from "../src/shell/StatusBar";
import { PreferencesProvider } from "../src/preferences";
import { accountLabel, usageWarning, type UsageList } from "../src/providerContract";

const win = (kind: string, used_pct: number, resets_at: string | null = null) => ({ kind, used_pct, resets_at, warn: used_pct >= 80 });
const list = (...usage: UsageList["usage"]): UsageList => ({ schema: "vajra.provider_usage/1", usage });
const acct = (account: string, windows: ReturnType<typeof win>[]) => ({ account, read_at: "2026-11-09T12:00:00Z", source: "cli", windows, amounts: null });

const bar = (usage?: UsageList | null) => render(
  <PreferencesProvider projectId={null}><StatusBar runs={0} usage={usage} compact={null} onColumn={() => {}} onSettings={() => {}} /></PreferencesProvider>,
);

describe("usage warning chip", () => {
  afterEach(cleanup);

  it("picks the most used window at or over 80% and ignores the rest", () => {
    expect(usageWarning(null)).toBeNull();
    expect(usageWarning(list(acct("claude", [win("weekly", 79)])))).toBeNull();
    const w = usageWarning(list(acct("claude", [win("5h", 82)]), acct("openrouter", [win("credit", 91)])));
    expect(w?.account).toBe("openrouter");
    expect(w?.window.used_pct).toBe(91);
  });

  it("shows nothing when nothing is reported or nothing is near its limit", () => {
    bar(list(acct("claude", []), acct("codex", [win("daily", 10)])));
    expect(screen.queryByRole("status")).toBeNull();
    cleanup();
    bar(undefined);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("puts the account's mark before the warning", () => {
    bar(list(acct("codex", [win("daily", 90)])));
    expect(screen.getByRole("status").querySelector(".v-pt[data-family='codex'] img")).not.toBeNull();
  });

  it("names the account by its label, not its id; an account with no known label keeps its id", () => {
    for (const [id, label] of [["codex", "Codex CLI"], ["openrouter", "OpenRouter"], ["acme-corp", "acme-corp"]]) {
      cleanup();
      bar(list(acct(id, [win("daily", 91)])));
      expect(screen.getByRole("status").textContent?.replace(/\s+/g, " ").trim()).toBe(`${label} 91% used today.`); // the tile adds no words
    }
    expect(accountLabel("claude")).toBe("Claude Code");
  });

  it("says which account, how much, and when it resets", () => {
    bar(list(acct("claude", [win("weekly", 84, "2026-11-12T09:00:00Z")])));
    const chip = screen.getByRole("status");
    expect(chip.textContent).toMatch(/^Claude Code 84% used this week\. Resets /);
  });

  it("leaves the reset out when the source gave none", () => {
    bar(list(acct("openrouter", [win("credit", 88)])));
    expect(screen.getByRole("status").textContent).toBe("OpenRouter 88% used of its credit limit.");
  });
});

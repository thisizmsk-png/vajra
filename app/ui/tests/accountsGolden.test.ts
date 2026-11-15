import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { parseAccounts } from "../src/accountsContract";
import { S } from "../strings";

// The bytes the core serialises for `provider_accounts` (checked byte for byte by app/core/src/providers/detect_tests.rs):
// the UI parser must read them as they are. It once read `override_warning` as a string, so the core's
// `{name, found_in}` object made the warning vanish.
const golden = JSON.parse(readFileSync(resolve(__dirname, "../../core/tests/fixtures/ipc/accounts.json"), "utf8"));

describe("the core's provider_accounts answer", () => {
  const r = parseAccounts(golden);
  const list = r.ok ? r.value.accounts : [];

  it("parses whole", () => {
    expect(r.ok).toBe(true);
    expect(list.map((a) => a.id)).toEqual(["claude", "codex", "openrouter"]);
  });

  it("keeps the override warning's name and where it was found, so the warning shows", () => {
    const codex = list.find((a) => a.id === "codex")!;
    expect(codex.billing.override_warning).toEqual({ name: "OPENAI_API_KEY", found_in: "cli_config" });
    expect(S.accounts.overrideWarning(codex.billing.override_warning!)).toBe("OPENAI_API_KEY in the CLI's config file: it bills instead of your plan");
    expect(list.find((a) => a.id === "claude")!.billing.override_warning).toBeNull();
  });

  it("reads the stored-key account as a keyed account with its saved state", () => {
    const or = list.find((a) => a.id === "openrouter")!;
    expect([or.credential.source, or.credential.saved, or.state, or.billing.kind, or.harness]).toEqual(["api_key", true, "key_saved", "gateway", "codex"]);
  });

  it("refuses a warning that is not the core's object, rather than showing garbage", () => {
    const bad = JSON.parse(JSON.stringify(golden));
    bad.accounts[1].billing.override_warning = "OPENAI_API_KEY in your Codex config";
    expect(parseAccounts(bad).ok).toBe(false);
    bad.accounts[1].billing.override_warning = { name: "has spaces and a \u001b", found_in: "cli_config" };
    expect(parseAccounts(bad).ok).toBe(false);
  });

  it("reads an unknown place as the CLI's config, and keeps the name", () => {
    const odd = JSON.parse(JSON.stringify(golden));
    odd.accounts[1].billing.override_warning = { name: "OPENAI_API_KEY", found_in: "somewhere_new" };
    const p = parseAccounts(odd);
    expect(p.ok && p.value.accounts[1].billing.override_warning).toEqual({ name: "OPENAI_API_KEY", found_in: "cli_config" });
  });
});

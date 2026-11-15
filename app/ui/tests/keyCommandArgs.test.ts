import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { api, setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";

// The core's stored-key commands and the argument names they take: the same file app/core/tests/ipc_args.rs checks against
// the command signatures in main.rs. If the UI sends a name the core does not take, the call fails at run time (it once
// sent `accountId` to a command that took `account`), so the payload each `api.providerKey*` builds is pinned here.
const commands: Record<string, string[]> = JSON.parse(readFileSync(resolve(__dirname, "../../core/tests/fixtures/ipc/key-commands.json"), "utf8"));
const camel = (s: string) => s.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
const FAKE_KEY = "fake-test-key-0000-not-a-secret";

afterEach(() => setTransport());

describe("the stored-key command payloads", () => {
  for (const [cmd, names] of Object.entries(commands)) {
    it(`${cmd} sends exactly ${names.join(", ")}`, async () => {
      const f = fakeIpc({ [cmd]: null });
      setTransport(f.transport);
      const method = (api as unknown as Record<string, (...a: string[]) => Promise<unknown>>)[camel(cmd)];
      expect(method, `api.${camel(cmd)} exists`).toBeTypeOf("function");
      await method("openrouter", FAKE_KEY);
      expect(f.calls).toHaveLength(1);
      expect(f.calls[0].cmd).toBe(cmd);
      expect(Object.keys(f.calls[0].args ?? {}).sort()).toEqual([...names].sort());
    });
  }
});

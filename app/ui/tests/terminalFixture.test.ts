import { describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { api } from "../src/ipc";
import { terminalFixture } from "../src/terminal/fixtureShell";
import { fakeIpc } from "./fakeIpc";

describe("the dev fixture shell", () => {
  it("opens, echoes, runs a line, exits, and refuses a folder that is not allowed", async () => {
    setTransport(terminalFixture(fakeIpc().transport));
    const got: string[] = [];
    let exit: number | null | undefined;
    const info = await api.termOpen("/work/app", 24, 80, (m) => ("data" in m ? got.push(new TextDecoder().decode(m.data)) : (exit = m.exit)));
    expect(info).toMatchObject({ cwd: "/work/app", alive: true });
    await new Promise((r) => setTimeout(r, 5));
    await api.termWrite(info.id, "hi\r");
    expect(got.join("")).toContain("no such command: hi");
    await api.termWrite(info.id, "exit\r");
    expect(exit).toBe(0);
    await api.termClose(info.id);
    await expect(api.termWrite(info.id, "x")).rejects.toMatchObject({ error: "not_found" });
    await expect(api.termOpen("/work/denied", 24, 80, () => {})).rejects.toMatchObject({ error: "cwd_not_allowed" });
    setTransport();
  });
});

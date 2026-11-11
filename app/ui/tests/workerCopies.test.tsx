// Settings > General, worker copies (ADR 16.6 "Disk"): the size line, and Clean up as one guarded, app-wide action that
// reports what it removed. Also: a launch refused while agents are off shows the core's line as it is.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { errorText, setTransport } from "../src/ipc";
import { WorkerCopies, size } from "../src/screens/settings/WorkerCopies";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const C = S.settings.copies;
afterEach(() => { cleanup(); setTransport(); });

describe("worker copies in Settings", () => {
  it("shows the copies and their size, and says shared space may be less", async () => {
    setTransport(fakeIpc({ worker_disk: { copies: 3, bytes: 412 * 1024 * 1024 } }).transport);
    render(<WorkerCopies />);
    expect(await screen.findByText(C.use(3, "412.0 MB"))).toBeTruthy();
    expect(screen.getByText(C.shared)).toBeTruthy();
  });

  it("Clean up sends one guarded app-wide action and shows what it removed and the new size", async () => {
    const f = fakeIpc({
      worker_disk: { copies: 2, bytes: 2048 },
      act_worker_clean_up: { receipt: { action_id: "a1", state: "done" }, result: { removed: ["t1"], disk: { copies: 1, bytes: 1024 } } },
    });
    setTransport(f.transport);
    render(<WorkerCopies />);
    fireEvent.click(await screen.findByRole("button", { name: C.cleanUp }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(C.cleaned(1)));
    expect(screen.getByText(C.use(1, "1 KB"))).toBeTruthy();
    const acts = f.calls.filter((c) => c.cmd === "act_worker_clean_up");
    expect(acts).toHaveLength(1);
    const env = acts[0].args?.envelope as { project_id: string; action_id: string };
    expect(env.project_id).toBe("global");
    expect(env.action_id).toBeTruthy();
  });

  it("says plainly when nothing could be removed, and Clean up is off with no copies", async () => {
    const f = fakeIpc({ worker_disk: { copies: 1, bytes: 10 }, act_worker_clean_up: { receipt: { action_id: "a2", state: "done" }, result: { removed: [], disk: { copies: 1, bytes: 10 } } } });
    setTransport(f.transport);
    render(<WorkerCopies />);
    fireEvent.click(await screen.findByRole("button", { name: C.cleanUp }));
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe(C.cleaned(0)));
    cleanup();
    const empty = fakeIpc({ worker_disk: { copies: 0, bytes: 0 } });
    setTransport(empty.transport);
    render(<WorkerCopies />);
    expect(await screen.findByText(C.none)).toBeTruthy();
    const b = screen.getByRole("button", { name: C.cleanUp });
    expect(b.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(b);
    expect(empty.calls.some((c) => c.cmd === "act_worker_clean_up")).toBe(false);
  });

  it("a size reads in the largest whole unit", () => {
    expect([size(0), size(900), size(2048), size(5 * 1024 ** 3)]).toEqual(["0 bytes", "900 bytes", "2 KB", "5.0 GB"]);
  });

  it("a launch refused while agents are off shows the core's line without the code", () => {
    expect(errorText({ error: "agents_off", detail: "Agents are off: x." })).toBe("Agents are off: x.");
  });
});

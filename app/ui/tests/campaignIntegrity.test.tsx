import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { History } from "../src/screens/history/History";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
  window.location.hash = "";
});

const checking = (...slugs: string[]) => ({ campaigns: slugs.map((slug) => ({ slug, goal: slug, status: "done", passing: 1, total: 1, integrity: "checking" })) });
const rowOf = (slug: string) => within(screen.getByRole("table")).getAllByRole("row").find((r) => within(r).queryByRole("button", { name: slug }))!;

describe("History badges fill in as each campaign_integrity returns (#28)", () => {
  it("paints the list at once as Checking, then fills each badge from its own call", async () => {
    let releaseB!: (v: unknown) => void;
    const slowB = new Promise((r) => { releaseB = r; });
    const f = fakeIpc({
      list_campaigns: checking("a-camp", "b-camp"),
      campaign_integrity: (a?: Record<string, unknown>) => a?.slug === "b-camp" ? slowB : { slug: "a-camp", integrity: "ok" },
    });
    setTransport(f.transport);
    render(<History />);
    await screen.findByRole("table");
    await waitFor(() => expect(within(rowOf("a-camp")).getByText(S.history.integrityOk)).toBeTruthy());
    expect(within(rowOf("b-camp")).getByText(S.history.integrityChecking)).toBeTruthy();
    releaseB({ slug: "b-camp", integrity: "failed" });
    await waitFor(() => expect(within(rowOf("b-camp")).getByText(S.history.integrityFailed)).toBeTruthy());
    expect(f.calls.filter((c) => c.cmd === "campaign_integrity").map((c) => c.args?.slug).sort()).toEqual(["a-camp", "b-camp"]);
  });

  it("asks only for the campaigns that read Checking, never for one already settled", async () => {
    const f = fakeIpc({
      list_campaigns: { campaigns: [
        { slug: "a-camp", goal: "a", status: "done", integrity: "ok" },
        { slug: "b-camp", goal: "b", status: "done", integrity: "checking" },
        { slug: "bad", error: "unreadable" },
      ] },
      campaign_integrity: { slug: "b-camp", integrity: "ok" },
    });
    setTransport(f.transport);
    render(<History />);
    await screen.findByRole("table");
    await waitFor(() => expect(within(rowOf("b-camp")).getByText(S.history.integrityOk)).toBeTruthy());
    expect(f.calls.filter((c) => c.cmd === "campaign_integrity").map((c) => c.args?.slug)).toEqual(["b-camp"]);
  });

  it("shows Not checked when the call fails or answers unknown, never a green badge", async () => {
    const f = fakeIpc({
      list_campaigns: checking("a-camp", "b-camp"),
      campaign_integrity: (a?: Record<string, unknown>) => { if (a?.slug === "a-camp") throw { error: "io_error" }; return { slug: "b-camp", integrity: "unknown" }; },
    });
    setTransport(f.transport);
    render(<History />);
    await screen.findByRole("table");
    await waitFor(() => expect(within(rowOf("a-camp")).getByText(S.history.integrityUnknown)).toBeTruthy());
    await waitFor(() => expect(within(rowOf("b-camp")).getByText(S.history.integrityUnknown)).toBeTruthy());
    expect(screen.queryByText(S.history.integrityOk)).toBeNull();
  });

  it("a reload that settles a verdict wins over a late answer for the earlier list", async () => {
    let releaseOld!: (v: unknown) => void;
    const slowOld = new Promise((r) => { releaseOld = r; });
    let n = 0;
    const f = fakeIpc({
      list_campaigns: () => (++n === 1 ? checking("a-camp") : { campaigns: [{ slug: "a-camp", goal: "a", status: "done", integrity: "failed" }] }),
      campaign_integrity: () => slowOld,
    });
    setTransport(f.transport);
    render(<History />);
    await screen.findByRole("table");
    f.emit("changed", ["a-camp"]);
    await waitFor(() => expect(within(rowOf("a-camp")).getByText(S.history.integrityFailed)).toBeTruthy());
    releaseOld({ slug: "a-camp", integrity: "ok" });
    await new Promise((r) => setTimeout(r, 20));
    expect(within(rowOf("a-camp")).getByText(S.history.integrityFailed)).toBeTruthy();
    expect(screen.queryByText(S.history.integrityOk)).toBeNull();
  });
});

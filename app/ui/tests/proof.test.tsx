import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { setTransport } from "../src/ipc";
import type { ProofLabel, RunProof } from "../src/ipc";
import { Proof, parsePr, proofTone } from "../src/screens/run/Proof";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const L = S.run.proof;
const LABELS: ProofLabel[] = ["verified", "verified_approved", "evidence_unguarded", "not_verified", "rejected", "abandoned", "in_progress"];

/** The core's answer for each label (a fixture of `run_proof`). */
const proof = (label: ProofLabel, extra: Partial<RunProof> = {}): RunProof => ({
  slug: "deliver-20261112-1900",
  goal: "Add rate limits",
  label,
  reason: `reason for ${label}`,
  why: label === "evidence_unguarded" ? "A helper outside the Vajra guard wrote the final check." : null,
  hint: label === "evidence_unguarded" ? "To get plain Verified, pick a Claude helper as the checker." : null,
  integrity: "ok",
  record_sha256: "ab".repeat(32),
  record_note: "Checked by Vajra on this Mac. The signing key is guarded, not yet sandboxed.",
  requires: "The evidence check needs macOS 15 or later.",
  export: true,
  markdown: `## Vajra proof\n\n**${L.label[label]}**.`,
  ...extra,
});

beforeAll(() => {
  // jsdom has no modal dialog: model showModal/close the way browsers do (as base.test.tsx does).
  HTMLDialogElement.prototype.showModal = function (this: HTMLDialogElement) { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function (this: HTMLDialogElement) { this.removeAttribute("open"); this.dispatchEvent(new Event("close")); };
});
afterEach(() => { cleanup(); setTransport(); });

describe("Proof", () => {
  it.each(LABELS)("shows the %s label exactly as the core answered it", async (label) => {
    setTransport(fakeIpc({ run_proof: proof(label) }).transport);
    render(<Proof slug="deliver-20261112-1900" />);
    expect(await screen.findByText(L.label[label])).toBeTruthy();
    expect(screen.getByText(`reason for ${label}`)).toBeTruthy();
    expect(screen.getByText("The evidence check needs macOS 15 or later.")).toBeTruthy();
    // The record note is the core's one string, shown as given.
    expect(screen.getByText("Checked by Vajra on this Mac. The signing key is guarded, not yet sandboxed.")).toBeTruthy();
  });

  it("never prints a bare Verified for a run whose evidence ran outside the guard", async () => {
    setTransport(fakeIpc({ run_proof: proof("evidence_unguarded") }).transport);
    const { container } = render(<Proof slug="deliver-20261112-1900-og" />);
    await screen.findByText(L.label.evidence_unguarded);
    const words = Array.from(container.querySelectorAll("*")).filter((e) => e.children.length === 0).map((e) => e.textContent?.trim());
    expect(words).not.toContain("Verified");
    expect(proofTone("evidence_unguarded")).not.toBe(proofTone("verified"));
    // "Why?" opens the risk text and the way to plain Verified.
    fireEvent.click(screen.getByRole("button", { name: L.why }));
    expect(screen.getByText(/outside the Vajra guard/)).toBeTruthy();
    expect(screen.getByText(/pick a Claude helper/)).toBeTruthy();
  });

  it("no string says the record can be verified by someone else", () => {
    const all = JSON.stringify(L).toLowerCase();
    expect(all).not.toContain("verifiable");
    expect(all).not.toContain("cryptographically");
  });

  it("shows the record check and its hash, and a failed check says so", async () => {
    setTransport(fakeIpc({ run_proof: proof("not_verified", { integrity: "integrity_failed", record_sha256: null }) }).transport);
    render(<Proof slug="s1-camp" />);
    expect(await screen.findByText(L.recordFailed)).toBeTruthy();
    expect(screen.queryByText(L.hash)).toBeNull();
    cleanup();
    setTransport(fakeIpc({ run_proof: proof("verified") }).transport);
    render(<Proof slug="s1-camp" />);
    expect(await screen.findByText("ab".repeat(32))).toBeTruthy();
    expect(screen.getByText(L.recordOk)).toBeTruthy();
  });

  it("a run still going, or a build without export, says why its export buttons are off", async () => {
    setTransport(fakeIpc({ run_proof: proof("in_progress") }).transport);
    render(<Proof slug="s1-camp" />);
    expect(await screen.findByText(L.notFinished)).toBeTruthy();
    expect(screen.getByRole("button", { name: L.exportPage }).getAttribute("aria-disabled")).toBe("true");
    expect(screen.queryByLabelText(L.prLink)).toBeNull();
    cleanup();
    setTransport(fakeIpc({ run_proof: proof("verified", { export: false }) }).transport);
    render(<Proof slug="s1-camp" />);
    expect(await screen.findByText(L.exportOff)).toBeTruthy();
  });

  it("exports the page and reports where it went", async () => {
    const ipc = fakeIpc({ run_proof: proof("verified"), export_proof_page: { slug: "s1-camp", path: "/data/exports/s1-camp-proof.html", label: "verified" } });
    setTransport(ipc.transport);
    render(<Proof slug="s1-camp" />);
    fireEvent.click(await screen.findByRole("button", { name: L.exportPage }));
    expect(await screen.findByText(L.exported("/data/exports/s1-camp-proof.html"))).toBeTruthy();
  });

  it("posting asks first, naming the repo and PR, and a bad link is refused before anything is sent", async () => {
    const ipc = fakeIpc({
      run_proof: proof("evidence_unguarded"),
      act_post_proof_comment: { receipt: { state: "done" }, result: { repo: "acme/web", pr: 42 } },
    });
    setTransport(ipc.transport);
    render(<Proof slug="s1-camp" />);
    const field = await screen.findByLabelText(L.prLink);
    fireEvent.change(field, { target: { value: "https://github.com/acme/web/issues/42" } });
    fireEvent.click(screen.getByRole("button", { name: L.post }));
    expect(screen.getByText(L.prInvalid)).toBeTruthy();
    fireEvent.change(field, { target: { value: "https://github.com/acme/web/pull/42" } });
    fireEvent.click(screen.getByRole("button", { name: L.post }));
    expect(screen.getByText(L.confirmBody("acme/web", 42))).toBeTruthy();
    expect(ipc.calls.some((c) => c.cmd === "act_post_proof_comment")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: L.confirmPost }));
    await waitFor(() => expect(screen.getByText(L.posted("acme/web", 42))).toBeTruthy());
    const call = ipc.calls.find((c) => c.cmd === "act_post_proof_comment")!;
    expect(call.args?.prUrl).toBe("https://github.com/acme/web/pull/42");
    expect(call.args).not.toHaveProperty("markdown");
  });

  it("cancelling the confirm sends nothing; a refused post says why", async () => {
    const ipc = fakeIpc({
      run_proof: proof("verified"),
      act_post_proof_comment: { receipt: { state: "rejected", detail: "GitHub CLI (gh) was not found." }, error: { error: "gh_missing", detail: "GitHub CLI (gh) was not found." } },
    });
    setTransport(ipc.transport);
    render(<Proof slug="s1-camp" />);
    fireEvent.change(await screen.findByLabelText(L.prLink), { target: { value: "https://github.com/acme/web/pull/7" } });
    fireEvent.click(screen.getByRole("button", { name: L.post }));
    fireEvent.click(screen.getByRole("button", { name: S.cancel }));
    expect(ipc.calls.some((c) => c.cmd === "act_post_proof_comment")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: L.post }));
    fireEvent.click(screen.getByRole("button", { name: L.confirmPost }));
    expect(await screen.findByText(L.postFailed("GitHub CLI (gh) was not found."))).toBeTruthy();
  });

  it("a burst of changed events runs at most one proof check at a time, then one rerun", async () => {
    const pending: (() => void)[] = [];
    const ipc = fakeIpc({ run_proof: () => new Promise((res) => pending.push(() => res(proof("verified")))) });
    setTransport(ipc.transport);
    render(<Proof slug="s1-camp" />);
    const runs = () => ipc.calls.filter((c) => c.cmd === "run_proof").length;
    await waitFor(() => expect(runs()).toBe(1));
    act(() => { for (let i = 0; i < 20; i++) ipc.emit("changed", ["s1-camp"]); });
    expect(runs()).toBe(1);
    await act(async () => { pending.shift()!(); });
    await waitFor(() => expect(runs()).toBe(2));
    // The queued rerun is the only one; nothing more is started until another event.
    await act(async () => { pending.shift()!(); });
    expect(runs()).toBe(2);
    expect(await screen.findByText(L.label.verified)).toBeTruthy();
  });

  it("a proof with checks still running asks again until they finish, with no changed event", async () => {
    vi.useFakeTimers();
    try {
      let n = 0;
      const ipc = fakeIpc({
        run_proof: () => (++n < 3 ? proof("not_verified", { pending: true, reason: "not checked yet" }) : proof("verified")),
      });
      setTransport(ipc.transport);
      render(<Proof slug="s1-camp" />);
      const runs = () => ipc.calls.filter((c) => c.cmd === "run_proof").length;
      await act(async () => { await vi.advanceTimersByTimeAsync(0); });
      expect(screen.getByText("not checked yet")).toBeTruthy();
      await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
      await act(async () => { await vi.advanceTimersByTimeAsync(1500); });
      expect(screen.getByText(L.label.verified)).toBeTruthy();
      // Settled: no more asks.
      await act(async () => { await vi.advanceTimersByTimeAsync(10_000); });
      expect(runs()).toBe(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("a check that never finishes stops asking after the cap", async () => {
    vi.useFakeTimers();
    try {
      const ipc = fakeIpc({ run_proof: proof("not_verified", { pending: true }) });
      setTransport(ipc.transport);
      render(<Proof slug="s1-camp" />);
      await act(async () => { await vi.advanceTimersByTimeAsync(5 * 60_000); });
      const runs = ipc.calls.filter((c) => c.cmd === "run_proof").length;
      expect(runs).toBe(41);
    } finally {
      vi.useRealTimers();
    }
  });

  it("an unreadable proof is an error, not an empty state", async () => {
    setTransport(fakeIpc({ run_proof: () => new Error("boom") }).transport);
    render(<Proof slug="s1-camp" />);
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("parses only plain GitHub pull request links", () => {
    expect(parsePr("https://github.com/acme/web/pull/42")).toEqual({ repo: "acme/web", pr: 42 });
    for (const bad of ["https://github.com/acme/web/pull/042", "https://github.com/../x/pull/1", "https://github.com/a/b/pull/1?x", "acme/web#1"]) expect(parsePr(bad)).toBeNull();
  });
});

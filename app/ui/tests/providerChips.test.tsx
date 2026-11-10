// Multi-provider on the Director-General's proposal card and on every surface a worker appears (sidebar, Board, Crew, Run, status
// bar). Fixture data only; no real key anywhere. Every model reads "Model · provider", a pick outside the project's allowed list is
// never offered, Start carries only the picks the user changed, and a hash mismatch is shown.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ACCOUNTS, AGENTS, USAGE } from "../src/accountsFixture";
import { parseProposalWorkers } from "../src/accountsContract";
import { setTransport } from "../src/ipc";
import type { InboxItem, RunOverview } from "../src/ipc";
import { boardCards } from "../src/projects/adapter";
import { CurrentProject } from "../src/projects/current";
import type { Current } from "../src/projects/current";
import { workersOf } from "../src/projects/store";
import { CrewTree } from "../components/ui/desktop";
import { DirectorGeneral } from "../src/screens/orch/DirectorGeneral";
import { ProviderLine, iconOf } from "../src/screens/ProviderLine";
import { errorText, parseWorkerProvider, ranOn, workerLine } from "../src/workerProvider";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

const A = S.agents;
const P = S.proposalWorkers;
const W = S.workerProvider;
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v));
afterEach(() => { cleanup(); setTransport(); });

const claude = { account: "claude", label: "Claude Code", family: "claude", model: null, model_name: null, billing: "subscription", requested: null, error: null };
const gateway = { account: "openrouter", label: "OpenRouter", family: "deepseek", model: "deepseek/deepseek-v3.2", model_name: "DeepSeek V3.2", billing: "gateway", requested: null, error: null };

describe("a worker's provider (effective)", () => {
  it("reads only the fields it names; absent or unreadable is no provider, never a guess", () => {
    expect(parseWorkerProvider(undefined)).toBeNull();
    expect(parseWorkerProvider({ account: "x" })).toBeNull();
    const withSecret = { ...gateway, api_key: "fake-test-key", credential: { key: "fake-test-key" } };
    expect(JSON.stringify(parseWorkerProvider(withSecret))).not.toContain("fake-test-key");
    expect(parseWorkerProvider({ ...gateway, billing: "wibble" })?.billing).toBe("unknown");
  });
  it("every line reads Model · provider: the default model, a named model, an id, and Not reported", () => {
    expect(workerLine(parseWorkerProvider(claude)!)).toBe(`${S.accounts.defaultModel} · Claude Code`);
    expect(workerLine(parseWorkerProvider(gateway)!)).toBe("DeepSeek V3.2 · OpenRouter");
    expect(workerLine(parseWorkerProvider({ ...gateway, model_name: null })!)).toBe("deepseek/deepseek-v3.2 · OpenRouter");
    expect(workerLine(parseWorkerProvider({ ...claude, model: "unknown" })!)).toBe(`${W.notReported} · Claude Code`);
  });
  it("Ran on X (asked for Y) only when what ran differs from what was asked", () => {
    const asked = { model: "x-ai/grok-4", model_name: "Grok 4", label: "OpenRouter" };
    expect(ranOn(parseWorkerProvider({ ...gateway, requested: asked })!)).toBe(W.ranOn("DeepSeek V3.2 · OpenRouter", "Grok 4 · OpenRouter"));
    expect(ranOn(parseWorkerProvider({ ...gateway, requested: { model: gateway.model, model_name: gateway.model_name, label: "OpenRouter" } })!)).toBeNull();
    // The provider can differ too: the harness ran its own default where the gateway model was asked for.
    expect(ranOn(parseWorkerProvider({ ...claude, requested: { model: gateway.model, model_name: "DeepSeek V3.2", label: "OpenRouter" } })!)).toBe(W.ranOn(`${S.accounts.defaultModel} · Claude Code`, "DeepSeek V3.2 · OpenRouter"));
    expect(ranOn(parseWorkerProvider(gateway)!)).toBeNull();
  });
  it("provider errors read in words: retrying with the time, or Needs you with the reason", () => {
    expect(errorText({ kind: "timeout", action: "retrying", attempt: 1, retry_at: "14:05" })).toBe("Upstream timeout. Retrying, attempt 1 of 2 at 14:05.");
    expect(errorText({ kind: "rate_limit", action: "asking_user", attempt: null, retry_at: "Thu 09:00" })).toBe("Needs you: rate limited, resets Thu 09:00.");
    expect(errorText({ kind: "auth", action: "asking_user", attempt: null, retry_at: null })).toBe(W.error.needsYou.auth(null));
    expect(errorText({ kind: "unexplained_exit", action: "asking_user", attempt: null, retry_at: null })).toBe(W.error.needsYou.unexplained_exit(null));
  });
  it("ProviderLine shows the icon, the line, the billing chip, the ran-on note and the error state; compact drops the chip", () => {
    const p = parseWorkerProvider({ ...gateway, requested: { model: "x-ai/grok-4", model_name: "Grok 4" }, error: { kind: "timeout", action: "retrying", attempt: 1, retry_at: "14:05" } })!;
    const { container, rerender } = render(<ProviderLine p={p} live />);
    expect(container.querySelector("[data-model-line]")?.textContent).toBe("DeepSeek V3.2 · OpenRouter");
    expect(screen.getByText(W.billing.gateway)).toBeTruthy();
    expect(screen.getByText(W.ranOn("DeepSeek V3.2 · OpenRouter", "Grok 4 · OpenRouter"))).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Upstream timeout. Retrying, attempt 1 of 2 at 14:05.");
    expect(container.querySelector("[aria-hidden='true']")?.querySelectorAll(":scope > img")).toHaveLength(2); // the tile is decorative; the name is written beside it
    rerender(<ProviderLine p={p} compact />);
    expect(screen.queryByText(W.billing.gateway)).toBeNull();
  });
  it("a family without a mark gets the fallback icon", () => {
    const { container } = render(iconOf(parseWorkerProvider({ ...claude, family: "brand-new-family" })!));
    expect(container.querySelector("[data-family]")?.getAttribute("data-family")).toBe("unknown");
  });
});

describe("workers on every surface", () => {
  it("project_state workers and the crew tree carry the provider; an unreadable one is absent", () => {
    const node = (id: string, provider: unknown) => ({ id: `agent:${id}`, kind: "worker", title: id, engine_id: null, role: id, status: "in_progress", overlays: [], children: [], provider });
    const root = { id: "lead", kind: "lead", title: "Lead", engine_id: null, status: "in_progress", overlays: [], children: [node("backend", claude), node("docs", { nope: 1 })] };
    const rows = workersOf(root);
    expect(rows[0].provider?.label).toBe("Claude Code");
    expect(rows[1].provider).toBeNull();
  });
  it("Board cards take the provider from the task", () => {
    const overview = { slug: "s", run_id: "r", stale: false, timeline: { phases: [], current: "", status: "active" }, graph: { waves: 1, tasks: [{ id: "r.t1", local: "t1", title: "T", owner: "o", status: "in_progress", deps: [], wave: 1, provider: gateway }] } } as unknown as RunOverview;
    expect(boardCards(overview, [], [])[0].provider?.model_name).toBe("DeepSeek V3.2");
  });
  it("the crew tree draws the icon before the name, decoratively", () => {
    const { container } = render(<CrewTree label="Crew" nodes={[{ id: "a", label: "Backend", status: "done", icon: iconOf(parseWorkerProvider(claude)!) }]} selected="a" onSelect={() => {}} />);
    expect(screen.getByRole("treeitem", { name: /Backend/ })).toBeTruthy();
    expect(container.querySelector(".v-pt")?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelectorAll(".v-pt img")).toHaveLength(2);
  });
});

describe("the proposal card's workers", () => {
  const WORKERS = [
    { slot: "w1", role: "backend", task: "Token bucket middleware", account: "claude", model: null, effort: "high", source: "role_default", why: "Touches auth; keeps the guard" },
    { slot: "w3", role: "docs", task: "Document the limits", account: "openrouter", model: "deepseek/deepseek-v3.2", effort: null, source: "role_default", why: "Plain writing, low risk" },
    { slot: "w4", role: "tests", task: "Fixtures for the limits", account: "codex", model: null, effort: "medium", source: "dg", why: "Generated test data" },
  ];
  const proposal = (o: Partial<InboxItem> & { workers?: unknown } = {}): InboxItem => ({
    id: "proposal:a1", kind: "run_proposal", origin: "app", created: "", age_s: 5, title: "Start a run?", body: "", actions: ["start", "dismiss"],
    ask: "Add rate limits", base: "main", rounds: 2, project_id: "p_X", project: "/src/api", generation: 3, proposal_sha256: "ab".repeat(32), workers: WORKERS, ...o,
  } as InboxItem);
  const project = { schema: "vajra.project/1", id: "p_1", name: "api", root: "/src/api", created: null, archived: null, rev: 0 };
  const cur = (items: InboxItem[]): Current => ({ project, campaigns: [], focus: "c1", runId: null, dgRun: "20261106-2140", proposals: items });
  const STREAM = "orchestrator-p_X-g3";
  function mount(item: InboxItem, replies: Record<string, unknown> = {}) {
    const f = fakeIpc({
      lead_status: { available: true, busy: false, name: "Director-General", state: "idle", project_id: "p_X", generation: 3, stream: STREAM },
      stream_window: { lines: [], head_seq: 0, first_seq: 1, at_start: true, at_end: true, gap: false },
      provider_accounts: ACCOUNTS, provider_usage: USAGE, provider_agents: clone(AGENTS), start_proposal: { action_id: "a9", state: "started", run: {} }, ...replies,
    });
    setTransport(f.transport);
    render(<CurrentProject.Provider value={cur([item])}><DirectorGeneral /></CurrentProject.Provider>);
    return f;
  }
  const card = () => screen.getByRole("article", { name: S.dg.proposal });
  const workers = async () => within(await screen.findByRole("region", { name: P.title }));
  const pick = async (role: string, task: string) => (await (await workers()).findByLabelText(A.pickLabel(`${role}: ${task}`))) as HTMLSelectElement;

  it("a chip per worker: Model · provider, effort, billing, the reason, the limits of a harness outside the guard, and the 80% warning", async () => {
    mount(proposal());
    const w = await workers();
    expect(((await pick("Backend", "Token bucket middleware")).selectedOptions[0]).textContent).toBe(`${S.accounts.defaultModel} · Claude Code`);
    expect(((await w.findByLabelText(A.effortLabel("Backend: Token bucket middleware"))) as HTMLSelectElement).value).toBe("high");
    expect(((await pick("Docs", "Document the limits")).selectedOptions[0]).textContent).toBe("DeepSeek V3.2 · OpenRouter");
    expect(w.getByText("Touches auth; keeps the guard")).toBeTruthy();
    expect(w.getByText("Max plan, s…@gmail.com")).toBeTruthy();
    expect(w.getByText("OpenRouter key")).toBeTruthy();
    expect(w.getByText(`${P.billingUnknown} · OPENAI_API_KEY in your Codex config`)).toBeTruthy();
    expect(w.getAllByText(P.limits)).toHaveLength(2); // the Codex worker and the gateway worker both run on harness B
    expect(w.getAllByText(S.accounts.warn80("Claude Code", S.accounts.window.weekly, 84, "Thu 09:00"))).toHaveLength(1);
    expect(w.getByText(/Billed to your Claude Code plan \(1\), your OpenRouter key \(1\) and your Codex CLI login \(1\)\./)).toBeTruthy();
    expect(w.getByText(/2 workers run outside the Vajra guard\./)).toBeTruthy();
  });

  it("every option in every picker reads Model · provider, and nothing outside the allowed list is offered", async () => {
    mount(proposal());
    await workers();
    const select = await pick("Docs", "Document the limits");
    const texts = [...select.options].map((o) => o.textContent ?? "");
    expect(texts).toEqual([`${S.accounts.defaultModel} · Claude Code`, `${S.accounts.defaultModel} · Codex CLI`, "DeepSeek V3.2 · OpenRouter"]);
    for (const t of texts) expect(t).toMatch(/^.+ · .+$/);
    expect(screen.queryByRole("option", { name: /Grok 4/ })).toBeNull();
    expect(screen.getByText(P.notAllowed)).toBeTruthy();
  });

  it("a changed pick says Changed by you with what was proposed; Start carries only the changed picks; changing it back clears it", async () => {
    const f = mount(proposal());
    const docs = await pick("Docs", "Document the limits");
    fireEvent.change(docs, { target: { value: "codex|" } });
    const w = await workers();
    expect(w.getByText(P.changed("DeepSeek V3.2 · OpenRouter"))).toBeTruthy();
    expect(w.getAllByText(/Changed by you/)).toHaveLength(1);
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "start_proposal")).toBe(true));
    expect(f.calls.find((c) => c.cmd === "start_proposal")?.args).toEqual({
      id: "proposal:a1", proposalSha256: "ab".repeat(32), overrides: [{ seat: "w3", account: "codex", model: null, effort: null }],
    });
  });

  it("nothing changed means Start carries no overrides; reverting a change removes it", async () => {
    const f = mount(proposal());
    const docs = await pick("Docs", "Document the limits");
    fireEvent.change(docs, { target: { value: "codex|" } });
    fireEvent.change(docs, { target: { value: "openrouter|deepseek/deepseek-v3.2" } });
    expect((await workers()).queryByText(/Changed by you/)).toBeNull();
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "start_proposal")).toBe(true));
    expect(f.calls.find((c) => c.cmd === "start_proposal")?.args).toEqual({ id: "proposal:a1", proposalSha256: "ab".repeat(32) });
  });

  it("changing the effort alone is a change too", async () => {
    const f = mount(proposal());
    await workers();
    fireEvent.change(await (await workers()).findByLabelText(A.effortLabel("Backend: Token bucket middleware")), { target: { value: "low" } });
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(f.calls.some((c) => c.cmd === "start_proposal")).toBe(true));
    expect((f.calls.find((c) => c.cmd === "start_proposal")?.args as { overrides: unknown[] }).overrides).toEqual([{ seat: "w1", account: "claude", model: null, effort: "low" }]);
  });

  it("a hash mismatch (the workers changed after they were shown) is refused with the reason, with the picks still on screen", async () => {
    mount(proposal(), { start_proposal: () => { throw { error: "proposal_changed" }; } });
    await workers();
    fireEvent.click(within(card()).getByRole("button", { name: S.dg.start }));
    await waitFor(() => expect(within(card()).getByText(S.dg.receipt.changed)).toBeTruthy());
    expect(((await pick("Docs", "Document the limits")).selectedOptions[0]).textContent).toBe("DeepSeek V3.2 · OpenRouter");
  });

  it("when the project's allowed list cannot be read, the picks are shown but not editable, with the reason", async () => {
    mount(proposal(), { provider_agents: new Error("x") });
    const w = await workers();
    await waitFor(() => expect(w.getByText(P.readFailed)).toBeTruthy());
    expect(w.queryAllByRole("combobox")).toHaveLength(0);
    expect(document.body.querySelector("[data-model-line]")?.textContent).toMatch(/^.+ · .+$/);
  });

  it("a proposal without workers never loads the accounts", async () => {
    const f = mount(proposal({ workers: undefined }));
    await within(card()).findByRole("button", { name: S.dg.start });
    expect(f.calls.some((c) => c.cmd === "provider_accounts")).toBe(false);
  });

  it("reads workers leniently: a bad row is skipped, the rest kept", () => {
    expect(parseProposalWorkers([...WORKERS, { slot: "bad" }, "x"]).map((x) => x.slot)).toEqual(["w1", "w3", "w4"]);
    expect(parseProposalWorkers(undefined)).toEqual([]);
  });
});

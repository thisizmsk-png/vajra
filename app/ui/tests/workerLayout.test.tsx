// A worker's tab lays its stream and Details side by side with a two-column grid on `.v-worker`. jsdom has no layout,
// so the rule is held at its two sources: the DOM must keep the pane and the aside as direct children of `.v-worker`
// (WorkerPane returns a bare fragment), and the CSS must keep the grid and the narrow Details rule.
import { cleanup, render, waitFor } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import type { CrewNodeDto } from "../src/ipc";
import { WorkerView } from "../src/screens/worker/WorkerView";
import { fakeIpc } from "./fakeIpc";

afterEach(() => { cleanup(); setTransport(); window.location.hash = ""; });

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "../src/shell.css"), "utf8");
const narrow = css.slice(css.indexOf("@media (max-width: 1199px)"));

const tree = {
  id: "run:r", kind: "run", title: "run", status: "in_progress", children: [
    { id: "task:t1", kind: "task", title: "Token bucket", status: "in_progress", children: [
      { id: "agent:backend", kind: "agent", title: "Backend", role: "Backend", status: "in_progress", children: [] },
    ] },
  ],
} as unknown as CrewNodeDto;

describe("worker tab layout", () => {
  it("keeps the stream pane and the Details aside as direct children of .v-worker, so the grid can place them", async () => {
    window.location.hash = "#/worker?slug=c1&id=r1&worker=backend";
    setTransport(fakeIpc({ crew: () => tree }).transport);
    const { container } = render(<WorkerView />);
    await waitFor(() => expect(container.querySelector(".v-worker > .v-crew__aside")).not.toBeNull());
    const worker = container.querySelector(".v-worker") as HTMLElement;
    const kids = Array.from(worker.children);
    expect(kids.some((c) => c.classList.contains("v-crew__pane"))).toBe(true);
    expect(kids.some((c) => c.classList.contains("v-crew__aside"))).toBe(true);
    // Nothing wraps them: a wrapper div would put Details back under the stream.
    expect(kids.every((c) => c.classList.contains("v-crew__pane") || c.classList.contains("v-crew__aside"))).toBe(true);
  });

  it("keeps the two-column grid on .v-worker, and the narrow one under 1200 px", () => {
    expect(css).toMatch(/\.v-worker\s*\{[^}]*display:\s*grid;[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+280px/);
    expect(narrow).toMatch(/\.v-worker\s*\{\s*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+220px/);
  });

  it("stacks each label over its value in the narrow Details column", () => {
    expect(narrow).toMatch(/\.v-crew__aside \.v-crew__facts\s*\{\s*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
  });
});

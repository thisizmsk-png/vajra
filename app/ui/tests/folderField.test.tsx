import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { FolderField } from "../src/screens/FolderField";
import { S } from "../strings";
import { fakeIpc } from "./fakeIpc";

afterEach(() => {
  cleanup();
  setTransport();
});

function Harness({ start = "" }: { start?: string }) {
  const [v, setV] = useState(start);
  return <FolderField label="Repository folder" value={v} onChange={setV} />;
}

const field = () => screen.getByLabelText("Repository folder") as HTMLInputElement;
const choose = () => fireEvent.click(screen.getByRole("button", { name: S.folder.chooseLabel }));

describe("FolderField", () => {
  it("asks the system picker for one folder and fills the field with the chosen path", async () => {
    const f = fakeIpc({ "plugin:dialog|open": "/Users/me/code/app" });
    setTransport(f.transport);
    render(<Harness />);
    choose();
    await waitFor(() => expect(field().value).toBe("/Users/me/code/app"));
    expect(f.calls).toEqual([{ cmd: "plugin:dialog|open", args: { options: { directory: true, multiple: false, title: S.folder.title } } }]);
  });

  it("opens at the folder already in the field", async () => {
    const f = fakeIpc({ "plugin:dialog|open": null });
    setTransport(f.transport);
    render(<Harness start=" /Users/me/code " />);
    choose();
    await waitFor(() => expect(f.calls).toHaveLength(1));
    expect(f.calls[0].args).toEqual({ options: { directory: true, multiple: false, title: S.folder.title, defaultPath: "/Users/me/code" } });
  });

  it("leaves the field as it was when the person cancels", async () => {
    const f = fakeIpc({ "plugin:dialog|open": null });
    setTransport(f.transport);
    render(<Harness start="/r" />);
    choose();
    await waitFor(() => expect(screen.getByRole("button", { name: S.folder.chooseLabel }).getAttribute("aria-busy")).toBeNull());
    expect(field().value).toBe("/r");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says to paste the path when the picker cannot open, and clears that once the person types", async () => {
    setTransport(fakeIpc({ "plugin:dialog|open": new Error("denied") }).transport);
    render(<Harness />);
    choose();
    expect((await screen.findByRole("alert")).textContent).toBe(S.folder.failed);
    expect(field().getAttribute("aria-invalid")).toBe("true");
    fireEvent.change(field(), { target: { value: "/typed" } });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(field().value).toBe("/typed");
  });

  it("keeps the text editable for a pasted path", () => {
    setTransport(fakeIpc().transport);
    render(<Harness />);
    fireEvent.change(field(), { target: { value: "/pasted" } });
    expect(field().value).toBe("/pasted");
  });
});

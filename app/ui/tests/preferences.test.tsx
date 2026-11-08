import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";
import { PreferenceForm } from "../src/screens/settings/PreferenceForm";
import { PreferencesProvider, capturePreferences } from "../src/preferences";
import { Home } from "../src/screens/home/Home";
import { DEFAULT_PREFERENCES, matchesShortcut } from "../src/preferenceContract";
const P="p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const view=(project:string|null=null, rev=0, theme="system")=>({schema:"vajra.preferences/1",rev,project_id:project,global:{...DEFAULT_PREFERENCES},overrides:{},effective:{...DEFAULT_PREFERENCES,theme},sources:Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map(k=>[k,"global"]))});
afterEach(()=>{cleanup();setTransport();});
describe("FG-08 preferences",()=>{
 it("saves the selected scope explicitly with its revision, then reset stays scoped",async()=>{
  let revision=0;const f=fakeIpc({preferences_read:(a?:Record<string,unknown>)=>view(a?.projectId as string|null,revision),preferences_update:(a?:Record<string,unknown>)=>{revision++;return view(a?.projectId as string|null,revision,(a?.patch as {theme?:string}).theme??"system");}});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);
  const theme=await screen.findByLabelText("Theme");fireEvent.change(theme,{target:{value:"dark"}});expect(f.calls.filter(c=>c.cmd==="preferences_update")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");expect(f.calls.find(c=>c.cmd==="preferences_update")?.args).toMatchObject({projectId:P,expectRev:0,patch:{theme:"dark"},reset:false});
  fireEvent.click(screen.getByRole("button",{name:"Reset this scope"}));await waitFor(()=>expect(f.calls.filter(c=>c.cmd==="preferences_update")).toHaveLength(2));expect(f.calls.filter(c=>c.cmd==="preferences_update")[1].args).toMatchObject({projectId:P,reset:true});
 });
 it("shows inherited values and lets one field inherit without resetting the rest",async()=>{
  const f=fakeIpc({preferences_read:()=>({...view(P),sources:{...view(P).sources,theme:"project"},overrides:{theme:"dark"}}),preferences_update:()=>view(P,1)});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);fireEvent.click(await screen.findByRole("button",{name:"Use global Theme"}));fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");expect(f.calls.find(c=>c.cmd==="preferences_update")?.args?.patch).toEqual({inherit:["theme"]});
 });
 it("keeps a conflict visible instead of claiming save success or silently retrying",async()=>{
  const f=fakeIpc({preferences_read:view(),preferences_update:()=>Promise.reject({error:"revision_conflict"})});setTransport(f.transport);render(<PreferenceForm projectId={null} section="appearance"/>);fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"light"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));expect(await screen.findByRole("alert")).toBeTruthy();expect(screen.queryByText("Saved.")).toBeNull();expect(f.calls.filter(c=>c.cmd==="preferences_update")).toHaveLength(1);
 });
 it("drops an old scope reply, and changing scope never carries a draft into another project",async()=>{
  let resolve!:(v:unknown)=>void;const slow=new Promise(r=>{resolve=r;});setTransport(fakeIpc({preferences_read:(a?:Record<string,unknown>)=>a?.projectId===P?slow:view(null,2,"light")}).transport);
  const r=render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);r.rerender(<PreferenceForm projectId={null} section="appearance"/>);await waitFor(()=>expect((screen.getByLabelText("Theme") as HTMLSelectElement).value).toBe("light"));await act(async()=>resolve(view(P,1,"dark")));expect((screen.getByLabelText("Theme") as HTMLSelectElement).value).toBe("light");
 });
 it("applies saved appearance with no polling and preserves a captured launch configuration",async()=>{
  const f=fakeIpc({preferences_read:view(null,0,"dark")});setTransport(f.transport);render(<PreferencesProvider projectId={null}><span>App</span></PreferencesProvider>);await waitFor(()=>expect(document.documentElement.dataset.theme).toBe("dark"));const launch=capturePreferences(view() as never);await act(async()=>window.dispatchEvent(new CustomEvent("vajra-preferences-saved")));expect(launch.settings.theme).toBe("system");expect(f.calls.filter(c=>c.cmd==="preferences_read")).toHaveLength(2);
 });
 it("initializes new task capacity from saved defaults without editing an active run",async()=>{
  setTransport(fakeIpc({preferences_read:{...view(),effective:{...DEFAULT_PREFERENCES,workers:7,rounds:1}},preflight:{rows:[],start_enabled:true},list_campaigns:{campaigns:[]},resume_targets:{targets:{}}}).transport);
  render(<PreferencesProvider projectId={null}><Home/></PreferencesProvider>);
  await waitFor(()=>expect((screen.getByLabelText("Workers at once (1 to 16)") as HTMLInputElement).value).toBe("7"));
 });
 it("handles an unsupported core reply without crashing the settings screen",async()=>{
  setTransport(fakeIpc({preferences_read:{}}).transport);
  render(<PreferenceForm projectId={null} section="appearance"/>);
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.getByRole("button",{name:"Reload preferences"})).toBeTruthy();
 });
 it("supports the saved primary key but does not hijack composition or terminal keystrokes",()=>{
  expect(matchesShortcut(new KeyboardEvent("keydown",{key:"p",metaKey:true,shiftKey:true}),"Primary+Shift+P")).toBe(true);expect(matchesShortcut(new KeyboardEvent("keydown",{key:"p",metaKey:true,shiftKey:true,isComposing:true}),"Primary+Shift+P")).toBe(false);
  const terminal=document.createElement("textarea");terminal.className="xterm-helper-textarea";const e=new KeyboardEvent("keydown",{key:"k",metaKey:true,bubbles:true});let matched=true;terminal.addEventListener("keydown",x=>{matched=matchesShortcut(x,"Primary+K");});terminal.dispatchEvent(e);expect(matched).toBe(false);
 });
});

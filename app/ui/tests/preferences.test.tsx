import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTransport } from "../src/ipc";
import { fakeIpc } from "./fakeIpc";
import { PreferenceForm } from "../src/screens/settings/PreferenceForm";
import { PreferencesProvider, capturePreferences } from "../src/preferences";
import { Home } from "../src/screens/home/Home";
import { Settings } from "../src/screens/settings/Settings";
import { CurrentProject } from "../src/projects/current";
import { DEFAULT_PREFERENCES, matchesShortcut } from "../src/preferenceContract";
const done=(result:unknown)=>({receipt:{state:"done",action_id:"prefs-save-test",project_id:"global"},result});
const P="p_01JC5X8Z3A9K2M4N6P7Q8R9S0T";
const view=(project:string|null=null, rev=0, theme="system")=>({schema:"vajra.preferences/1",rev,project_id:project,caps:{active_runs:1,workers_per_run:16,editable:false},global:{...DEFAULT_PREFERENCES},overrides:{},effective:{...DEFAULT_PREFERENCES,theme},sources:Object.fromEntries(Object.keys(DEFAULT_PREFERENCES).map(k=>[k,"global"]))});
afterEach(()=>{cleanup();setTransport();});
describe("FG-08 preferences",()=>{
 it("saves the selected scope explicitly with its revision, then reset stays scoped",async()=>{
  let revision=0;const f=fakeIpc({preferences_read:(a?:Record<string,unknown>)=>view(a?.projectId as string|null,revision),act_preferences_update:(a?:Record<string,unknown>)=>{revision++;return done(view((a?.envelope as {project_id:string}).project_id,revision,(a?.patch as {theme?:string}).theme??"system"));}});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);
  const theme=await screen.findByLabelText("Theme");fireEvent.change(theme,{target:{value:"dark"}});expect(f.calls.filter(c=>c.cmd==="act_preferences_update")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");expect(f.calls.find(c=>c.cmd==="act_preferences_update")?.args).toMatchObject({envelope:{project_id:P,expect_generation:0},patch:{theme:"dark"},reset:false});
  fireEvent.click(screen.getByRole("button",{name:"Reset this scope"}));await waitFor(()=>expect(f.calls.filter(c=>c.cmd==="act_preferences_update")).toHaveLength(2));expect(f.calls.filter(c=>c.cmd==="act_preferences_update")[1].args).toMatchObject({envelope:{project_id:P},reset:true});
 });
 it("shows inherited values and lets one field inherit without resetting the rest",async()=>{
  const f=fakeIpc({preferences_read:()=>({...view(P),sources:{...view(P).sources,theme:"project"},overrides:{theme:"dark"}}),act_preferences_update:()=>done(view(P,1))});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);fireEvent.click(await screen.findByRole("button",{name:"Use global Theme"}));fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");expect(f.calls.find(c=>c.cmd==="act_preferences_update")?.args?.patch).toEqual({inherit:["theme"]});
 });
 it("keeps a conflict visible instead of claiming save success or silently retrying",async()=>{
  const f=fakeIpc({preferences_read:view(),act_preferences_update:()=>({receipt:{state:"rejected",reason:"revision_conflict",action_id:"prefs-stale"},error:{error:"revision_conflict"}})});setTransport(f.transport);render(<PreferenceForm projectId={null} section="appearance"/>);fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"light"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));expect(await screen.findByRole("alert")).toBeTruthy();expect(screen.queryByText("Saved.")).toBeNull();expect(f.calls.filter(c=>c.cmd==="act_preferences_update")).toHaveLength(1);
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
 it("takes the next project's capacity defaults when the project changes, and keeps an edit only within its own project",async()=>{
  const Q="p_01JC5X8Z3A9K2M4N6P7Q8R9S0U";const workers:Record<string,number>={[P]:7,[Q]:3};
  const f=fakeIpc({preferences_read:(a?:Record<string,unknown>)=>({...view(a?.projectId as string),effective:{...DEFAULT_PREFERENCES,workers:workers[a?.projectId as string]}}),preflight:{rows:[],start_enabled:true},list_campaigns:{campaigns:[]},resume_targets:{targets:{}}});setTransport(f.transport);
  const proj=(id:string)=>({schema:"vajra.project/1",id,name:id,root:"/src/"+id,created:null,archived:null,rev:0});
  const tree=(id:string)=><CurrentProject.Provider value={{project:proj(id),campaigns:[],focus:null,runId:null}}><PreferencesProvider projectId={id}><Home/></PreferencesProvider></CurrentProject.Provider>;
  const field=()=>screen.getByLabelText("Workers at once (1 to 16)") as HTMLInputElement;
  const r=render(tree(P));await waitFor(()=>expect(field().value).toBe("7"));
  r.rerender(tree(Q));await waitFor(()=>expect(field().value).toBe("3"));
  fireEvent.change(field(),{target:{value:"9"}});await act(async()=>window.dispatchEvent(new CustomEvent("vajra-preferences-saved")));
  await waitFor(()=>expect(f.calls.filter(c=>c.cmd==="preferences_read"&&c.args?.projectId===Q).length).toBeGreaterThan(1));expect(field().value).toBe("9");
 });
 it("shows no editor, shell or notification controls until something reads them, and a save elsewhere keeps their stored values",async()=>{
  const stored={...view(),overrides:{editor:"code",shell:"zsh",notifications_text:true},effective:{...DEFAULT_PREFERENCES,editor:"code",shell:"zsh",notifications_text:true}};
  const f=fakeIpc({preferences_read:stored,provider_list:{schema:"vajra.providers/1",providers:[]},preflight:{rows:[],start_enabled:true},act_preferences_update:(a?:Record<string,unknown>)=>done({...stored,rev:1,effective:{...stored.effective,...(a?.patch as object)}})});setTransport(f.transport);
  window.location.hash="#/settings?section=general";
  const r=render(<PreferencesProvider projectId={null}><Settings/></PreferencesProvider>);
  expect(await screen.findByText("Not built yet")).toBeTruthy();
  expect(screen.queryByLabelText("Default editor")).toBeNull();expect(screen.queryByLabelText("Terminal shell")).toBeNull();expect(screen.queryByRole("button",{name:"Save preferences"})).toBeNull();
  r.unmount();window.location.hash="#/settings?section=notifications";
  const n=render(<PreferencesProvider projectId={null}><Settings/></PreferencesProvider>);
  expect(await screen.findByText("Not built yet")).toBeTruthy();
  expect(screen.queryByRole("checkbox")).toBeNull();expect(screen.queryByRole("button",{name:"Save preferences"})).toBeNull();
  n.unmount();render(<PreferenceForm projectId={null} section="appearance"/>);
  fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"dark"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");
  const write=f.calls.find(c=>c.cmd==="act_preferences_update");expect(write?.args?.patch).toEqual({theme:"dark"});
  window.location.hash="";
 });
 it("handles an unsupported core reply without crashing the settings screen",async()=>{
  setTransport(fakeIpc({preferences_read:{}}).transport);
  render(<PreferenceForm projectId={null} section="appearance"/>);
  expect(await screen.findByRole("alert")).toBeTruthy();
  expect(screen.getByRole("button",{name:"Reload preferences"})).toBeTruthy();
 });
 it("reuses the action id after a lost reply and displays the receipt",async()=>{
  let calls=0;const f=fakeIpc({preferences_read:view(),act_preferences_update:(a?:Record<string,unknown>)=>{if(++calls===1)return Promise.reject(new Error("lost"));return {receipt:{state:"done",action_id:(a?.envelope as {action_id:string}).action_id},result:view(null,1,"dark")};}});setTransport(f.transport);
  render(<PreferenceForm projectId={null} section="appearance"/>);
  fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"dark"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));
  fireEvent.click(await screen.findByRole("button",{name:"Retry same action"}));await screen.findByText("Saved.");
  const writes=f.calls.filter(c=>c.cmd==="act_preferences_update");expect(writes[0].args?.envelope).toEqual(writes[1].args?.envelope);expect(screen.getByText(/Action done/)).toBeTruthy();
 });
 it("keeps the agreed notification defaults (on, item text off) while the section has no controls",()=>{
  expect([DEFAULT_PREFERENCES.notifications,DEFAULT_PREFERENCES.notifications_text]).toEqual([true,false]);
 });
 it("requires reload after an unknown outcome rather than creating a fresh action",async()=>{
  const f=fakeIpc({preferences_read:view(),act_preferences_update:()=>({receipt:{state:"unknown",action_id:"prefs-unknown"}})});setTransport(f.transport);
  render(<PreferenceForm projectId={null} section="appearance"/>);fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"dark"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));
  await screen.findByText(/Action unknown ·/);expect(screen.getByRole("button",{name:"Save preferences"}).getAttribute("aria-disabled")).toBe("true");fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));expect(f.calls.filter(c=>c.cmd==="act_preferences_update")).toHaveLength(1);expect(screen.queryByRole("button",{name:"Retry same action"})).toBeNull();
 });
 it("locks every draft control after a lost reply, so a retry of the first intent cannot clear a newer edit",async()=>{
  let calls=0;const f=fakeIpc({preferences_read:view(P),act_preferences_update:(a?:Record<string,unknown>)=>{if(++calls===1)return Promise.reject(new Error("lost"));return {receipt:{state:"done",action_id:(a?.envelope as {action_id:string}).action_id},result:view(P,1,"dark")};}});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="appearance"/>);
  fireEvent.change(await screen.findByLabelText("Theme"),{target:{value:"dark"}});fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));
  await screen.findByRole("button",{name:"Retry same action"});
  expect((screen.getByLabelText("Theme") as HTMLSelectElement).disabled).toBe(true);
  expect(screen.getByRole("button",{name:"Use global Theme"}).getAttribute("aria-disabled")).toBe("true");
  // Discard would clear the draft while Retry still holds it: it stays off until the outcome is known.
  expect(screen.getByRole("button",{name:"Discard changes"}).getAttribute("aria-disabled")).toBe("true");
  fireEvent.click(screen.getByRole("button",{name:"Discard changes"}));expect((screen.getByLabelText("Theme") as HTMLSelectElement).value).toBe("dark");
  fireEvent.click(screen.getByRole("button",{name:"Retry same action"}));await screen.findByText("Saved.");
  const writes=f.calls.filter(c=>c.cmd==="act_preferences_update");expect(writes).toHaveLength(2);expect(writes[1].args?.patch).toEqual({theme:"dark"});
  expect((screen.getByLabelText("Theme") as HTMLSelectElement).disabled).toBe(false);
 });
 it("offers one group-wide shortcut inheritance that says it resets both bindings",async()=>{
  const both={palette:"Primary+Shift+P",settings:"Primary+Shift+S"};
  const f=fakeIpc({preferences_read:{...view(P),overrides:{shortcuts:both},effective:{...DEFAULT_PREFERENCES,shortcuts:both},sources:{...view(P).sources,shortcuts:"project"}},act_preferences_update:()=>done(view(P,1))});setTransport(f.transport);
  render(<PreferenceForm projectId={P} projectName="Project A" section="keyboard"/>);
  await screen.findByLabelText("Search shortcut");
  expect(screen.queryByRole("button",{name:/^Use global Search shortcut$|^Use global Settings shortcut$/})).toBeNull();
  fireEvent.click(screen.getByRole("button",{name:"Use global shortcuts (both)"}));
  fireEvent.click(screen.getByRole("button",{name:"Save preferences"}));await screen.findByText("Saved.");
  expect(f.calls.find(c=>c.cmd==="act_preferences_update")?.args?.patch).toEqual({inherit:["shortcuts"]});
 });
 it("takes Primary as the platform modifier only: Command on a Mac (never Control), Control elsewhere (never Command)",()=>{
  const k=(o:KeyboardEventInit)=>new KeyboardEvent("keydown",{key:"k",...o});
  expect(matchesShortcut(k({metaKey:true}),"Primary+K",true)).toBe(true);
  expect(matchesShortcut(k({ctrlKey:true}),"Primary+K",true)).toBe(false);
  expect(matchesShortcut(new KeyboardEvent("keydown",{key:",",ctrlKey:true}),"Primary+,",true)).toBe(false);
  expect(matchesShortcut(k({metaKey:true,ctrlKey:true}),"Primary+K",true)).toBe(false);
  expect(matchesShortcut(k({ctrlKey:true}),"Primary+K",false)).toBe(true);
  expect(matchesShortcut(k({metaKey:true}),"Primary+K",false)).toBe(false);
 });
 it("supports the saved primary key but does not hijack composition or terminal keystrokes",()=>{
  expect(matchesShortcut(new KeyboardEvent("keydown",{key:"p",metaKey:true,shiftKey:true}),"Primary+Shift+P")).toBe(true);expect(matchesShortcut(new KeyboardEvent("keydown",{key:"p",metaKey:true,shiftKey:true,isComposing:true}),"Primary+Shift+P")).toBe(false);
  const terminal=document.createElement("textarea");terminal.className="xterm-helper-textarea";const e=new KeyboardEvent("keydown",{key:"k",metaKey:true,bubbles:true});let matched=true;terminal.addEventListener("keydown",x=>{matched=matchesShortcut(x,"Primary+K");});terminal.dispatchEvent(e);expect(matched).toBe(false);
 });
});

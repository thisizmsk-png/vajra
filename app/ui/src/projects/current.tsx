// The project the shell has placed you in, for the views inside it (Home's launcher, the board, the inspector).
import { createContext, useContext } from "react";
import type { Campaign, InboxItem } from "../ipc";
import type { Project } from "./contract";

export type Current = {
  project: Project | null;
  /** This project's campaigns, newest first. */
  campaigns: Campaign[];
  /** The campaign the board and Review show: the live run's if it is this project's, else the newest. */
  focus: string | null;
  /** The live run, when it belongs to this project. */
  runId: string | null;
  /** The run the Director-General's commands are addressed by (FG-02 keys them by a run of the project): the live run,
   *  else the newest one this app started here. Null when the project has none yet. */
  dgRun?: string | null;
  /** Run proposals for this project (inbox `run_proposal` items whose folder is this project's). */
  proposals?: InboxItem[];
};

export const CurrentProject = createContext<Current>({ project: null, campaigns: [], focus: null, runId: null });
export const useCurrentProject = () => useContext(CurrentProject);

// The project the shell has placed you in, for the views inside it (Home's launcher, the board, the inspector).
import { createContext, useContext } from "react";
import type { Campaign } from "../ipc";
import type { Project } from "./contract";

export type Current = {
  project: Project | null;
  /** This project's campaigns, newest first. */
  campaigns: Campaign[];
  /** The campaign the board and Review show: the live run's if it is this project's, else the newest. */
  focus: string | null;
  /** The live run, when it belongs to this project. */
  runId: string | null;
};

export const CurrentProject = createContext<Current>({ project: null, campaigns: [], focus: null, runId: null });
export const useCurrentProject = () => useContext(CurrentProject);

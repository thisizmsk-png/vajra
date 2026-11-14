// A screen polls its run only while the core says the run is live (nothing runs at rest). Tests of that polling mount
// the screen inside the real projects provider, with the core answering that `camp`/`r1` is running.
import type { ReactNode } from "react";
import { ProjectsProvider } from "../src/projects/store";

export const liveRun = (campaign = "camp", runId = "r1") => ({
  active_runs: { count: 1, runs: [{ stream: `run-${runId}`, session_id: "s", run_id: runId, campaign }] },
  list_campaigns: { campaigns: [] },
  resume_targets: { targets: {} },
  inbox: { items: [], count: 0 },
  project_list: { projects: [] },
});

export const Live = ({ children }: { children: ReactNode }) => <ProjectsProvider>{children}</ProjectsProvider>;

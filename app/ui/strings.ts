// User-visible wording of the kit. Screens add their own strings next to these.
// Log every wording change in docs/design/changed-strings.md for the owner's approval.
export const S = {
  dismiss: "Dismiss",
  cancel: "Cancel",
  close: "Close",
  busySuffix: "",
  status: {
    pending: "Pending",
    ready: "Ready",
    claimed: "Claimed",
    in_progress: "In progress",
    done: "Done",
    blocked: "Blocked",
    question: "Question",
    failed: "Failed",
    pass: "Pass",
    fail: "Fail",
    unknown: "Not run",
    needs_approval: "Needs approval",
  },
  nav: { home: "Home", run: "Run", crew: "Crew", chat: "Chat", inbox: "Inbox", history: "History", settings: "Settings" },
  shell: {
    navLabel: "Main",
    notBuilt: "This screen is not built yet.",
    errorTitle: "This screen hit a problem",
    errorBody: "The rest of the app is fine. You can try again or go to another screen.",
    retry: "Try again",
  },
} as const;

export type StatusKey = keyof typeof S.status;

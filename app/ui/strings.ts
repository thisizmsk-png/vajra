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
} as const;

export type StatusKey = keyof typeof S.status;

// Reading the provider accounts and their usage, shared by Settings > Providers (a lazy chunk) and the status bar (the idle
// shell): hooks only, no UI. A late answer from an earlier ask is dropped; usage is best effort.
import { useCallback, useEffect, useRef, useState } from "react";
import { S } from "../strings";
import { api } from "./ipc";
import { parseAccounts, parseUsage } from "./accountsContract";
import type { Account, Usage } from "./accountsContract";
import { problemDetail } from "./screens/states";

const C = S.accounts;

export type Accounts = { list: Account[] | null; usage: Usage[]; error: string; loading: boolean; reload: () => void };

/** The accounts and their usage, validated. A late answer from an earlier ask is dropped. Usage is best effort: if it fails, the meters say "not reported". */
export function useAccounts(on: boolean): Accounts {
  const seq = useRef(0);
  const [s, setS] = useState<Omit<Accounts, "reload">>({ list: null, usage: [], error: "", loading: on });
  const reload = useCallback(() => {
    const mine = ++seq.current;
    setS((x) => ({ ...x, error: "", loading: true }));
    Promise.all([api.providerAccounts(), api.providerUsage().catch(() => null)]).then(([a, u]) => {
      if (mine !== seq.current) return;
      const r = parseAccounts(a);
      const us = u === null ? null : parseUsage(u);
      setS(r.ok ? { list: r.value.accounts, usage: us?.ok ? us.value : [], error: "", loading: false }
        : { list: null, usage: [], error: r.reason === "unsupported_version" ? C.updateNeeded : C.unreadable, loading: false });
    }, (e) => {
      if (mine === seq.current) setS({ list: null, usage: [], error: problemDetail(e), loading: false });
    });
  }, []);
  useEffect(() => {
    if (on) reload();
    return () => { ++seq.current; };
  }, [on, reload]);
  return { ...s, reload };
}

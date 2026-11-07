import { useEffect, useState } from "react";
import { api } from "../ipc";
import type { Campaign } from "../ipc";

/**
 * #28: `list_campaigns` answers at once, with `integrity: "checking"` for every campaign whose verdict is not fresh.
 * This asks the core for each of those (`campaign_integrity`, one call per campaign, the core caps how many verify at
 * once) and shows the answers as they arrive. A failed call reads "unknown" (Not checked), never ok. Answers belong
 * to the list they were asked for: a newer list drops them, and a campaign the list already settled keeps the list's
 * verdict.
 */
export function useCampaignIntegrity(camps: Campaign[] | null): Campaign[] | null {
  const [got, setGot] = useState<{ list: Campaign[]; by: Record<string, Campaign["integrity"]> } | null>(null);
  useEffect(() => {
    if (!camps) return;
    let live = true;
    setGot({ list: camps, by: {} });
    for (const c of camps) {
      if (c.integrity !== "checking") continue;
      const done = (v: Campaign["integrity"]) => live && setGot((g) => (g && g.list === camps ? { list: camps, by: { ...g.by, [c.slug]: v } } : g));
      api.campaignIntegrity(c.slug).then((r) => done(r.integrity), () => done("unknown"));
    }
    return () => { live = false; };
  }, [camps]);
  if (!camps) return null;
  const by = got && got.list === camps ? got.by : {};
  return camps.map((c) => (c.integrity === "checking" && by[c.slug] ? { ...c, integrity: by[c.slug] } : c));
}

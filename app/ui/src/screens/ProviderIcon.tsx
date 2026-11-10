// A provider's tile: its official mark where one is cleared to ship (assets/providers/marks.ts), a two-letter monogram where not, the
// plain fallback icon for anything unknown. The tile keeps its size either way. With the provider's name written beside it the tile is
// decorative (aria-hidden); with no name beside it, pass `named` and the tile becomes an image named for the provider.
import { useState, type CSSProperties } from "react";
import fallback from "../assets/providers/fallback.svg?raw";
import { MARKS } from "../assets/providers/marks";
import { cn } from "../../components/ui/cn";

const MONOGRAM: Record<string, string> = { claude: "CC", codex: "CX", grok: "GK", deepseek: "DS", openrouter: "OR" };
/** The provider's name, for the accessible label when no text name sits beside the tile. */
export const PROVIDER_NAME: Record<string, string> = { claude: "Claude Code", codex: "Codex CLI", grok: "Grok", deepseek: "DeepSeek", openrouter: "OpenRouter" };

/** The mark, or the monogram when there is none or an image fails to load (blocked, missing): a tile is never left empty. */
function Mark({ id, className }: { id: string; className: string }) {
  const [failed, setFailed] = useState(false);
  const m = MARKS[id];
  if (!m || failed) return <>{MONOGRAM[id]}</>;
  return (
    <>
      <img className={cn(className, "v-pt__mk--light")} style={{ "--fill": m.fill } as CSSProperties} src={m.light} alt="" draggable={false} onError={() => setFailed(true)} />
      <img className={cn(className, "v-pt__mk--dark")} style={{ "--fill": m.fill } as CSSProperties} src={m.dark} alt="" draggable={false} onError={() => setFailed(true)} />
    </>
  );
}

/** `family` is the model family for a gateway model (grok, deepseek), the provider id otherwise. `via` adds the gateway's corner badge. */
export function ProviderIcon({ family, via, large, named }: { family: string; via?: string; large?: boolean; named?: boolean }) {
  const known = !!MONOGRAM[family];
  const badge = via && MONOGRAM[via] ? via : undefined;
  const name = PROVIDER_NAME[family] ?? family;
  const a11y = named ? { role: "img", "aria-label": via && PROVIDER_NAME[via] ? `${name} · ${PROVIDER_NAME[via]}` : name } : { "aria-hidden": true as const };
  return (
    <span className={cn("v-pt", large && "v-pt--lg", !known && "v-pt--fb", MARKS[family] && "v-pt--mark")} {...a11y} data-family={known ? family : "unknown"}>
      {known ? <Mark id={family} className="v-pt__mk" /> : <span className="v-pt__fb" dangerouslySetInnerHTML={{ __html: fallback }} />}
      {badge && <span className="v-pt__via"><Mark id={badge} className="v-pt__mk v-pt__mk--sm" /></span>}
    </span>
  );
}

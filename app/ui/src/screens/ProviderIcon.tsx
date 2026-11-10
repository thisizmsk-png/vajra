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

/** A gateway model id's vendor prefix ("x-ai/grok-4" -> "x-ai") named as the registry knows it. A vendor with a MARKS entry shows its mark;
 *  adding a mark for another vendor is just a new MARKS entry (and an alias here if the prefix differs from the key). */
const VENDOR_KEY: Record<string, string> = { "x-ai": "grok", openai: "codex", anthropic: "claude", "meta-llama": "meta", bytedance: "bytedance-seed" };
/** "~openai/..." is OpenRouter's alias form of "openai/...". */
export const vendorOf = (model?: string | null): string | undefined => {
  const v = model && model.includes("/") ? model.split("/")[0].replace(/^~/, "").toLowerCase() : undefined;
  return v ? (VENDOR_KEY[v] ?? v) : undefined;
};

/** The mark, or the monogram when there is none or an image fails to load (blocked, missing): a tile is never left empty. */
function Mark({ id, className }: { id: string; className: string }) {
  const [failed, setFailed] = useState(false);
  const m = MARKS[id];
  if (!m || failed) return <>{MONOGRAM[id] ?? id.replace(/[^a-z0-9]/gi, "").slice(0, 2).toUpperCase()}</>;
  return (
    <>
      <img className={cn(className, "v-pt__mk--light")} style={{ "--fill": m.fill } as CSSProperties} src={m.light} alt="" draggable={false} onError={() => setFailed(true)} />
      <img className={cn(className, "v-pt__mk--dark")} style={{ "--fill": m.fill } as CSSProperties} src={m.dark} alt="" draggable={false} onError={() => setFailed(true)} />
    </>
  );
}

/** `family` is the model family for a gateway model (grok, deepseek), the provider id otherwise. `model` is the gateway model id: its vendor prefix
 *  picks the mark (initials for a vendor with none, the plain fallback only with no vendor at all). `via` adds the gateway's corner badge. */
export function ProviderIcon({ family: given, model, via, large, named }: { family: string; model?: string | null; via?: string; large?: boolean; named?: boolean }) {
  const vendor = vendorOf(model);
  const family = vendor ?? given;
  const known = !!MONOGRAM[family] || !!vendor;
  const badge = via && MONOGRAM[via] ? via : undefined;
  const name = PROVIDER_NAME[family] ?? (model ? model.split("/")[0] : family);
  const a11y = named ? { role: "img", "aria-label": via && PROVIDER_NAME[via] ? `${name} · ${PROVIDER_NAME[via]}` : name } : { "aria-hidden": true as const };
  return (
    <span className={cn("v-pt", large && "v-pt--lg", !known && "v-pt--fb", MARKS[family] && "v-pt--mark", MARKS[family]?.plate && "v-pt--plate")} {...a11y} data-family={known ? family : "unknown"}>
      {known ? <Mark id={family} className="v-pt__mk" /> : <span className="v-pt__fb" dangerouslySetInnerHTML={{ __html: fallback }} />}
      {badge && <span className="v-pt__via"><Mark id={badge} className="v-pt__mk v-pt__mk--sm" /></span>}
    </span>
  );
}

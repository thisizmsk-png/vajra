// A provider's tile: a two-letter monogram until its official mark is cleared (assets/providers/SOURCES.md), the plain
// fallback icon for anything unknown. Decorative: the provider's name is always written beside it, so the tile is aria-hidden.
import fallback from "../assets/providers/fallback.svg?raw";
import { cn } from "../../components/ui/cn";

const MONOGRAM: Record<string, string> = { claude: "CC", codex: "CX", grok: "GK", deepseek: "DS", openrouter: "OR" };

/** `family` is the model family for a gateway model (grok, deepseek), the provider id otherwise. `via` adds the gateway's corner badge. */
export function ProviderIcon({ family, via, large }: { family: string; via?: string; large?: boolean }) {
  const mono = MONOGRAM[family];
  const badge = via ? MONOGRAM[via] : undefined;
  return (
    <span className={cn("v-pt", large && "v-pt--lg", !mono && "v-pt--fb")} aria-hidden="true" data-family={mono ? family : "unknown"}>
      {mono ?? <span className="v-pt__fb" dangerouslySetInnerHTML={{ __html: fallback }} />}
      {badge && <span className="v-pt__via">{badge}</span>}
    </span>
  );
}

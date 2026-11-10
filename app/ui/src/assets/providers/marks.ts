// The brand marks that are cleared to ship, as the files supplied (SOURCES.md says which and why). `light` is the file made for light
// backgrounds, `dark` the one made for dark backgrounds; the unaltered file is used in both themes and nothing is ever recoloured.
// A provider with no entry here shows its monogram tile. Nothing is fetched at run time: the build ships each file as a same-origin
// asset (assetsInlineLimit 0), because the app's CSP blocks data: images.
import gatewayLight from "./openrouter-ink.svg?url";
import gatewayDark from "./openrouter-cloud.svg?url";
import codexLight from "./OAI_OpenAI-Blossom_Black.svg?url";
import codexDark from "./OAI_OpenAI-Blossom_White.svg?url";
import grokLight from "./Grok_Logomark_Dark.svg?url";
import grokDark from "./Grok_Logomark_Light.svg?url";
import claudeSpark from "./Claude Spark - Clay.svg?url";

/** `fill` is the share of the file's canvas the mark itself covers (the Blossom file keeps a wide margin around the flower), so the tile can scale the
 *  image, never the file, to make every mark about the same size. */
export const MARKS: Record<string, { light: string; dark: string; fill: number }> = {
  openrouter: { light: gatewayLight, dark: gatewayDark, fill: 1 },
  codex: { light: codexLight, dark: codexDark, fill: 0.52 },
  grok: { light: grokLight, dark: grokDark, fill: 1 },
  claude: { light: claudeSpark, dark: claudeSpark, fill: 1 }, // one clay-orange file reads on both themes
};

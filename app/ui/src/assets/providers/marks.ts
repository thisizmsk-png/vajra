// The brand marks that ship, as the files supplied. Each file's row in SOURCES.md says on what ground: "Cleared" by its terms, or a dated
// "Owner exception" (the owner accepted the clearance risk); a test fails for a file with neither. `light` is the file made for light
// backgrounds, `dark` the one made for dark backgrounds; the unaltered file is used in both themes and nothing is ever recoloured.
// A provider with no entry here shows its monogram tile. Nothing is fetched at run time: the build ships each file as a same-origin
// asset (assetsInlineLimit 0), because the app's CSP blocks data: images.
import gatewayLight from "./openrouter-ink.svg?url";
import gatewayDark from "./openrouter-cloud.svg?url";
import codexLight from "./OAI_OpenAI-Blossom_Black.svg?url";
import codexDark from "./OAI_OpenAI-Blossom_White.svg?url";
import grokLight from "./Grok_Logomark_Dark.svg?url";
import grokDark from "./Grok_Logomark_Light.svg?url";
import deepseekIcon from "./deepseek-favicon.ico?url";
import claudeSpark from "./Claude Spark - Clay.svg?url";

/** `fill` is the share of the file's canvas the mark itself covers (the Blossom file keeps a wide margin around the flower), so the tile can scale the
 *  image, never the file, to make every mark about the same size. */
export type Mark = { light: string; dark: string; fill: number; plate?: boolean };
export const MARKS: Record<string, Mark> = {
  openrouter: { light: gatewayLight, dark: gatewayDark, fill: 1 },
  codex: { light: codexLight, dark: codexDark, fill: 0.52 },
  grok: { light: grokLight, dark: grokDark, fill: 1 },
  deepseek: { light: deepseekIcon, dark: deepseekIcon, fill: 1 }, // raster whale icon, one file for both themes; chip and tile sizes only
  claude: { light: claudeSpark, dark: claudeSpark, fill: 1 }, // one clay-orange file reads on both themes
};

// Model vendors (the id prefix on OpenRouter), one folder each under vendors/, the file exactly as obtained (SOURCES.md has each source).
// A vendor with no entry shows its initials. `plate`: the file's glyph is dark on a transparent ground, so the tile gets a light plate behind it
// in the dark theme (the file is never recoloured). Not here: Nvidia (wide lockup only), Qwen and Reka (JPEG only), Inference.net (its SVG embeds a data: raster the app's CSP may block).
const vendorFiles = import.meta.glob("./vendors/*/*", { eager: true, query: "?url", import: "default" }) as Record<string, string>;
const vf = (vendor: string, file: string): string => vendorFiles[`./vendors/${vendor}/${file}`];
const one = (vendor: string, file: string, extra: Partial<Mark> = {}): Mark => ({ light: vf(vendor, file), dark: vf(vendor, file), fill: 1, ...extra });
Object.assign(MARKS, {
  "aion-labs": one("aion-labs", "apple-touch-icon.png"),
  "bytedance-seed": one("bytedance-seed", "favicon.ico"),
  cohere: one("cohere", "apple-touch-icon.png"),
  fireworks: one("fireworks", "icon.svg"),
  google: one("google", "gemini_sparkle_512.png"),
  inclusionai: one("inclusionai", "favicon.png"),
  liquid: one("liquid", "icon.png", { plate: true }),
  meituan: one("meituan", "favicon-mt.ico"),
  meta: one("meta", "meta-symbol.svg"),
  minimax: one("minimax", "favicon.ico"),
  mistralai: { light: vf("mistralai", "Icon-Mistral-Black.svg"), dark: vf("mistralai", "Icon-Mistral-White.svg"), fill: 1 },
  moondream: one("moondream", "apple-touch-icon.png"),
  moonshotai: one("moonshotai", "favicon.ico"),
  nousresearch: one("nousresearch", "favicon.png"),
  perceptron: { light: vf("perceptron", "favicon-dark.svg"), dark: vf("perceptron", "favicon-light.svg"), fill: 1 },
  perplexity: one("perplexity", "favicon.svg", { plate: true }),
  reflection: one("reflection", "icon.svg"),
  relace: one("relace", "FFActUeE4daoS9F9mpqmKv4y3ZI.svg", { plate: true }),
  stepfun: one("stepfun", "step_favicon.svg"),
  tencent: one("tencent", "apple-touch-icon.png"),
  thinkingmachines: one("thinkingmachines", "apple-touch-icon.png"),
  upstage: one("upstage", "symbol-256.png"),
  writer: one("writer", "writer-w-logo-192.png", { plate: true }),
  xiaomi: one("xiaomi", "mimo.ico"),
  "z-ai": one("z-ai", "logo.svg"),
} satisfies Record<string, Mark>);

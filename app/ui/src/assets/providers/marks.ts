// The brand marks that are cleared to ship, as the files supplied (SOURCES.md says which and why). `light` is the file made for light
// backgrounds, `dark` the one made for dark backgrounds; the unaltered file is used in both themes and nothing is ever recoloured.
// A provider with no entry here shows its monogram tile. Nothing is fetched at run time: a small file is inlined by the build.
import gatewayLight from "./openrouter-ink.svg?url";
import gatewayDark from "./openrouter-cloud.svg?url";

export const MARKS: Record<string, { light: string; dark: string }> = {
  openrouter: { light: gatewayLight, dark: gatewayDark },
};

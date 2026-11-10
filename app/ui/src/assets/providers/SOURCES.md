# Provider icons: sources

Rule (docs: provider icons plan): identify, never endorse; a brand mark is used as supplied, never altered. A mark ships only
on one of two documented grounds, stated in its row's Status column: "Cleared" (its published terms permit use as supplied,
quoted in the Terms column) or "Owner exception" (the product owner, who is accountable for the clearance, downloaded it from
the vendor's own source and accepted the clearance risk in writing on the date given; this is an explicit, authorised
exception to "terms cleared", not a clearance). A provider with neither shows a two-letter monogram tile plus its name.
The app never fetches an icon at run time: files are downloaded once, at build time, from the official source below, and
a test (`tests/providerIcons.test.tsx`) fails if a file's sha256 stops matching this table.

| File | Source | Downloaded | sha256 | Terms | Status |
|---|---|---|---|---|---|
| `fallback.svg` | Ours (original, `currentColor`) | n/a | `b54cb3a650d58406dd82d956fefd262fb20d1eb4725a329536d2e6b081aa7545` | None | Ours: in use for any provider or model family without a cleared mark, and any unknown id |
| `openrouter-ink.svg` | https://openrouter.ai/brand/logos/transparent/glyph/svg/glyph-ink.svg (glyph, "Ink", made for light backgrounds) | 2026-11-10 | `91be6b8a91745f089bfe42b89a5cbcd4b666fca6473610f9a6c11a31bd7523bc` | https://openrouter.ai/brand: "Please don't stretch, recolor, or remix the marks." | Cleared: In use on light themes. The file is byte for byte as downloaded. Used only to identify the gateway provider. |
| `openrouter-cloud.svg` | https://openrouter.ai/brand/logos/transparent/glyph/svg/glyph-cloud.svg (glyph, "Cloud", made for dark backgrounds) | 2026-11-10 | `385bbf00cd5718ad652fdfc05b4b4e7d0ea3a834cfd807e1133d3bee2b9a7c81` | https://openrouter.ai/brand: "Please don't stretch, recolor, or remix the marks." | Cleared: In use on dark themes. Byte for byte as downloaded. |
| `OAI_OpenAI-Blossom_Black.svg` | https://openai.com/brand/ ("Download logos", `SVGs/`), the Blossom mark in black, for light backgrounds | 2026-11-10 | `75c1e9fffa5e8c437bec1d67197a73992bca45d166c6ff23215185dea8fae92a` | https://openai.com/brand/ ("By using our logos, you agree to our Marks usage terms."; page rule: "DON'T stretch or alter wordmark in any way") | Owner exception: Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Codex. |
| `OAI_OpenAI-Blossom_White.svg` | same kit, the Blossom mark in white, for dark backgrounds | 2026-11-10 | `01d158767c4eec0e47bd617e67759c33da0accd1438be1a8d29dfdb99ce87285` | same page, same terms | Owner exception: Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Codex. |
| `Grok_Logomark_Dark.svg` | https://x.ai/legal/brand-guidelines ("Download Logos", SpaceXAI_Grok_Assets), the Grok logomark in dark (#0A0A0A), for light backgrounds | 2026-11-10 | `a127a7cd42b0450f7d3827a331b0730aab49fd99c3fe920d172475b9ffc83992` | https://x.ai/legal/brand-guidelines ("only use our logos exactly as provided at the download link below, without any alteration or adjustment") | Owner exception: Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Grok. |
| `Grok_Logomark_Light.svg` | same kit, the Grok logomark in white, for dark backgrounds | 2026-11-10 | `b20648e2f111d7fbc91f58b22d1e76e9885b68a163cb5a1010f7f11bf5840491` | same page, same terms | Owner exception: Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Grok. |
| `Claude Spark - Clay.svg` | https://www.anthropic.com/press-kit (redirects to the anthropic.com CDN zip "Anthropic media resources.zip", sha256 `c68ac92df86c825f95177e24016fcc9a8863a3fd4ca344fe6f0700b2c1e07151`), the Spark mark alone in Clay orange, one file for both themes | 2026-11-10 | `6d53db4be375e899c937c26cf16684a80d6e869b1928d72b37748bef2560e219` | Anthropic press kit | Owner exception: Owner downloaded from Anthropic's press kit and accepted the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Claude Code. |
| `deepseek-favicon.ico` | https://www.deepseek.com/favicon.ico (the whale icon, 225x225, 32-bit ICO; the only square official mark; raster, so used only at chip and tile sizes) | 2026-11-10 | `30a4420e6e4dcb17fd7de560c5004346c2bc8cd971d2d5f5ee15326603e51321` | Served by deepseek.com; no brand kit published | Owner exception: Owner accepted the clearance risk 2026-11-10; used only to identify the provider; byte for byte as served, owner-supplied; one file for both themes. DeepSeek. |

The fallback is the plan's `icons/fallback.svg` without its fixed `width`, `height` and `role="img"`: the icon is
decorative (`aria-hidden`), sized by the tile, and the provider's name is the accessible name.

Every provider now has a mark. A monogram (two letters) shows only if a mark's file fails to load. The wide DeepSeek lockup (github deepseek-ai/DeepSeek-LLM, images/logo.svg) is not used: it is not square and is not to be cropped.

To add a cleared mark: put the downloaded file here untouched, add its row above, and add its `light` and `dark` files to
`marks.ts`. Nothing else changes.

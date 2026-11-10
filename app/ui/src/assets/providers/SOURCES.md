# Provider icons: sources

Rule (docs: provider icons plan): identify, never endorse; a brand mark is used as supplied, never altered, and only once
its terms are cleared. Until a row below says "cleared", the UI shows a two-letter monogram tile plus the provider's name.
The app never fetches an icon at run time: files are downloaded once, at build time, from the official source below, and
a test (`tests/providerIcons.test.tsx`) fails if a file's sha256 stops matching this table.

| File | Source | Downloaded | sha256 | Terms | Status |
|---|---|---|---|---|---|
| `fallback.svg` | Ours (original, `currentColor`) | n/a | `b54cb3a650d58406dd82d956fefd262fb20d1eb4725a329536d2e6b081aa7545` | None | In use for any provider or model family without a cleared mark, and any unknown id |
| `openrouter-ink.svg` | https://openrouter.ai/brand/logos/transparent/glyph/svg/glyph-ink.svg (glyph, "Ink", made for light backgrounds) | 2026-11-10 | `91be6b8a91745f089bfe42b89a5cbcd4b666fca6473610f9a6c11a31bd7523bc` | https://openrouter.ai/brand: "Please don't stretch, recolor, or remix the marks." | In use on light themes. The file is byte for byte as downloaded. Used only to identify the gateway provider. |
| `openrouter-cloud.svg` | https://openrouter.ai/brand/logos/transparent/glyph/svg/glyph-cloud.svg (glyph, "Cloud", made for dark backgrounds) | 2026-11-10 | `385bbf00cd5718ad652fdfc05b4b4e7d0ea3a834cfd807e1133d3bee2b9a7c81` | https://openrouter.ai/brand: "Please don't stretch, recolor, or remix the marks." | In use on dark themes. Byte for byte as downloaded. |
| `OAI_OpenAI-Blossom_Black.svg` | https://openai.com/brand/ ("Download logos", `SVGs/`), the Blossom mark in black, for light backgrounds | 2026-11-10 | `75c1e9fffa5e8c437bec1d67197a73992bca45d166c6ff23215185dea8fae92a` | https://openai.com/brand/ ("By using our logos, you agree to our Marks usage terms."; page rule: "DON'T stretch or alter wordmark in any way") | Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Codex. |
| `OAI_OpenAI-Blossom_White.svg` | same kit, the Blossom mark in white, for dark backgrounds | 2026-11-10 | `01d158767c4eec0e47bd617e67759c33da0accd1438be1a8d29dfdb99ce87285` | same page, same terms | Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Codex. |
| `Grok_Logomark_Dark.svg` | https://x.ai/legal/brand-guidelines ("Download Logos", SpaceXAI_Grok_Assets), the Grok logomark in dark (#0A0A0A), for light backgrounds | 2026-11-10 | `a127a7cd42b0450f7d3827a331b0730aab49fd99c3fe920d172475b9ffc83992` | https://x.ai/legal/brand-guidelines ("only use our logos exactly as provided at the download link below, without any alteration or adjustment") | Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Grok. |
| `Grok_Logomark_Light.svg` | same kit, the Grok logomark in white, for dark backgrounds | 2026-11-10 | `b20648e2f111d7fbc91f58b22d1e76e9885b68a163cb5a1010f7f11bf5840491` | same page, same terms | Owner downloaded the kit and accepted the usage terms and the clearance risk 2026-11-10; used only to identify the provider; byte for byte as downloaded, owner-supplied. Grok. |

The fallback is the plan's `icons/fallback.svg` without its fixed `width`, `height` and `role="img"`: the icon is
decorative (`aria-hidden`), sized by the tile, and the provider's name is the accessible name.

Monogram only (no mark shipped), and why:

- Claude Code: no public file; the logo is supplied on request. Stays "CC".
- DeepSeek: its only official file is a 195x41 horizontal lockup, unreadable in the square tile and not to be cropped. Stays "DS" unless an official square mark exists.

To add a cleared mark: put the downloaded file here untouched, add its row above, and add its `light` and `dark` files to
`marks.ts`. Nothing else changes.

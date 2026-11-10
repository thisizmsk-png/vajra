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

The fallback is the plan's `icons/fallback.svg` without its fixed `width`, `height` and `role="img"`: the icon is
decorative (`aria-hidden`), sized by the tile, and the provider's name is the accessible name.

Monogram only (no mark shipped), and why:

- Claude Code, Codex CLI, DeepSeek: the plan's rule is "ask first". Permission has not been given. Do not add a file until a row
  here says "cleared by <who> on <date>".
- Grok: the official source (https://x.ai/legal/brand-guidelines) answered HTTP 403 to an automated download on 2026-11-10, so
  no file was fetched. Drop the official one-colour file here, add its row (source URL, date, sha256, the clause that permits
  identification), and map it in `marks.ts`.

To add a cleared mark: put the downloaded file here untouched, add its row above, and add its `light` and `dark` files to
`marks.ts`. Nothing else changes.

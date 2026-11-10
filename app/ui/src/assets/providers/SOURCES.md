# Provider icons: sources

Rule (docs: provider icons plan): identify, never endorse; a brand mark is used as supplied, never altered, and only once
its terms are cleared. Until a row below says "cleared", the UI shows a two-letter monogram tile plus the provider's name.

| File | Source | Downloaded | sha256 | Terms | Status |
|---|---|---|---|---|---|
| `fallback.svg` | Ours (original, `currentColor`) | n/a | `b54cb3a650d58406dd82d956fefd262fb20d1eb4725a329536d2e6b081aa7545` | None | In use for any provider or model family without a cleared mark, and any unknown id |

The fallback is the plan's `icons/fallback.svg` without its fixed `width`, `height` and `role="img"`: the icon is
decorative (`aria-hidden`), sized by the tile, and the provider's name is the accessible name.

No brand mark is shipped yet: Claude Code, Codex CLI, Grok, DeepSeek and OpenRouter are all "monogram only" (CC, CX, GK,
DS, OR). Drop the cleared file here, add its row (source URL, date, sha256, terms link, "cleared by <who> on <date>"),
and map it in `ProviderIcon.tsx`.

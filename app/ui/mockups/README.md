# Art direction mockups (owner pick)

Three directions for Vajra Desktop, each as a hi-fi mockup of Home and Crew in dark and light.
Mockups only: nothing here is imported by the app, and `src/screens` and `components/ui` are untouched.

| Dir | Name | Feels like | Signature |
|---|---|---|---|
| `a/` | Graphite | Linear, Raycast | Action bar footer with the selection and its keys |
| `b/` | Console | Cursor, Warp | Live agent lanes with command blocks and a Lead prompt |
| `c/` | Score | Bold surprise | The run as lanes over time, with the approval as a gate |
| `a2/` | Graphite v2 | The owner's pick (A), refined | A plus runs in the sidebar, one ⌘K field, Inbox, a review pane |

All three share one realistic run (`data.ts`), use the kit's token role names (`--c-*`) and reuse the
kit's `StatusDot`, so the chosen direction can move into `theme/tokens.css` and the kit.

Run from `app/ui`:

```sh
npx vite --config mockups/vite.config.ts      # http://127.0.0.1:1430
npx tsc -p mockups/tsconfig.json --noEmit
```

Open `?d=a|b|c&s=home|crew&t=dark|light`, or the bare URL for an index. `a2` also has
`s=home-empty`, `s=inbox` and `s=palette`, and reuses Graphite's tokens (`theme/a.css`).

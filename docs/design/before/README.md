# Before: UI SOP Phase 0 baseline

Captured on `ao/ui-redesign/root` from `origin/main` (7ce6593) with the web build and mocked IPC
(`app/ui/shots-harness`, not committed). 48 screenshots: 8 screens (Home, Crew, History, Settings,
Run, Chat, Inbox, kit Gallery) x dark and light x 1280x800, 1440x900 and 900x700. Run and Chat
and Inbox are the "not built" placeholders on main.

Light theme: the app has no theme switch and no `prefers-color-scheme` handling, so these light
shots set `data-theme="light"` on the root from the harness. The light theme is reachable in the
shipped app only through the kit Gallery.

## Violation count

| Measure | Count | Where |
|---|---|---|
| design-guard violations (`scanTree`) | 0 | whole UI tree (baseline test passes) |
| Raw hex, `rgb()`/`hsl()`, named colours | 0 | screens and kit, outside tokens |
| `letter-spacing` / `tracking-*` / ALL-CAPS / gradients / banned fonts | 0 | screens and kit |
| Inline `style={{` blocks | 16 | 12 static spacing blocks in the kit Gallery pages (`Gallery.tsx` 9, `GalleryDesktop.tsx` 3); 4 dynamic (`VirtualList` 3 positional, `CrewTree` 1 indent) |
| Hard-coded `px` literals in kit and shell CSS | 115 | `base.css`, `desktop.css`, `shell.css` (not tokens) |
| Hard-coded `font-size` | 7 | same CSS files |
| Light theme reachable in the app | no | no switch, no `prefers-color-scheme` |
| Favicon request | 404 | Home loads `/favicon.ico` (console error on every screen) |
| Overlay focus trap, Escape, focus return | present | `Sheet` and `AlertDialog` use the native `<dialog>` |
| Toast live region | present | `role="status"` `aria-live="polite"`; danger uses `role="alert"` |

Counts are from `grep` and the guard scanner, run on this branch. The design-guard target is 0 and
is already met; the open violations are the inline spacing, the hard-coded px, and the light theme
with no way to reach it.

# Changed strings

Log of every visible wording change, for owner approval (UI SOP).

This branch changes **no visible wording**. Changes in this PR:

| Where | Before | After | Visible? |
|---|---|---|---|
| `app/ui/index.html` | No favicon link (404 on every load) | `<link rel="icon" href="data:,">` | No (removes a console error) |
| `app/ui/src/Gallery.tsx`, `GalleryDesktop.tsx` | Inline style objects | Classes in `src/gallery.css` | No (kit dev page; all 48 screenshots byte-identical) |

Pending for Phase 3 (screen copy), waiting on the owner's go-ahead: none proposed yet.

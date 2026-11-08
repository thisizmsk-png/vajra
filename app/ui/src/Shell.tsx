import { useEffect, useRef } from "react";
import { Icon, Kbd, ariaShortcut } from "../components/ui/base";
import type { IconName } from "../components/ui/base";
import { ErrorBoundary } from "./ErrorBoundary";
import { PaletteButton, PaletteHost } from "./palette";
import { SCREENS, toHash, useRoute } from "./router";
import type { Screen } from "./router";
import { REGISTRY } from "./screens/registry";
import { S } from "../strings";
import "./shell.css";

/** ⌘1 to ⌘7 open the screens in nav order; each link shows its chord. */
const chord = (i: number) => ["⌘", String(i + 1)];
const ICON: Record<Screen, IconName> = { home: "home", run: "run", crew: "crew", chat: "chat", inbox: "inbox", history: "history", settings: "settings" };

/** Graphite shell: a 240 px sidebar with the screens, the current screen beside it, and the ⌘K palette over both. Every screen sits inside an error boundary. */
export function Shell() {
  const [route, go] = useRoute();
  const main = useRef<HTMLElement>(null);
  // The window title names the screen, so a screen reader and the window menu say where you are (WCAG 2.4.2).
  useEffect(() => { document.title = S.shell.title(S.nav[route.screen]); }, [route.screen]);
  // The hash is the router, so the skip link moves focus itself: into the screen's first control, else the screen.
  const skip = () => {
    const m = main.current;
    if (!m) return;
    const first = m.querySelector<HTMLElement>('input, textarea, select, button, a[href], [tabindex="0"]');
    (first ?? m).focus();
  };
  const Screen = REGISTRY[route.screen];
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.shiftKey || e.altKey || e.ctrlKey || e.defaultPrevented) return;
      const i = Number(e.key) - 1;
      if (Number.isInteger(i) && i >= 0 && i < SCREENS.length) {
        e.preventDefault();
        go(SCREENS[i]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go]);
  // The screen you are on is not a place to go to.
  const commands = SCREENS.flatMap((s, i) => (s === route.screen ? [] : [{ id: s, label: S.palette.goScreen(S.nav[s]), keys: chord(i), run: () => go(s) }]));
  return (
    <PaletteHost commands={commands}>
      <div className="v-shell">
        <a className="v-skip" href="#v-main" onClick={(e) => { e.preventDefault(); skip(); }}>{S.shell.skip}</a>
        <nav className="v-shell__nav" aria-label={S.shell.navLabel}>
          <div className="v-shell__brand">{S.shell.brand}</div>
          <PaletteButton label={S.shell.search} />
          {SCREENS.map((s, i) => (
            <a key={s} href={toHash(s)} className="v-shell__link" title={S.nav[s]} aria-current={route.screen === s ? "page" : undefined} aria-keyshortcuts={ariaShortcut(chord(i))}>
              <Icon name={ICON[s]} />
              <span className="v-shell__label">{S.nav[s]}</span>
              <Kbd keys={chord(i)} decorative />
            </a>
          ))}
        </nav>
        <main className="v-shell__main" id="v-main" tabIndex={-1} ref={main}>
          <ErrorBoundary resetKey={toHash(route.screen, route.params)}>
            <Screen />
          </ErrorBoundary>
        </main>
      </div>
    </PaletteHost>
  );
}

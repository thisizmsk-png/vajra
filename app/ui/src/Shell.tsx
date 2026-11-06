import { ErrorBoundary } from "./ErrorBoundary";
import { SCREENS, toHash, useRoute } from "./router";
import { REGISTRY } from "./screens/registry";
import { S } from "../strings";
import "./shell.css";

/** Navigation on the left, the current screen on the right. Every screen sits inside an error boundary. */
export function Shell() {
  const [route] = useRoute();
  const Screen = REGISTRY[route.screen];
  return (
    <div className="v-shell">
      <nav className="v-shell__nav" aria-label={S.shell.navLabel}>
        {SCREENS.map((s) => (
          <a key={s} href={toHash(s)} className="v-shell__link" aria-current={route.screen === s ? "page" : undefined}>
            {S.nav[s]}
          </a>
        ))}
      </nav>
      <main className="v-shell__main">
        <ErrorBoundary resetKey={route.screen}>
          <Screen />
        </ErrorBoundary>
      </main>
    </div>
  );
}

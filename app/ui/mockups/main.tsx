import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import type { ReactNode } from "react";
import { HomeA } from "./a/Home";
import { CrewA } from "./a/Crew";
import { HomeB } from "./b/Home";
import { CrewB } from "./b/Crew";
import { HomeC } from "./c/Home";
import { CrewC } from "./c/Crew";
import { HomeG } from "./a2/Home";
import { CrewG } from "./a2/Crew";
import { InboxG } from "./a2/Inbox";
import { PaletteG } from "./a2/Palette";
import "./shared.css";
import "./theme/a.css";
import "./theme/b.css";
import "./theme/c.css";
import "./a/a.css";
import "./b/b.css";
import "./c/c.css";
import "./a2/g2.css";

// ?d=a|b|c|a2  &s=home|crew (a2 also home-empty, inbox, palette)  &t=dark|light. No params: an index of all twelve frames.
const q = new URLSearchParams(location.search);
const d = q.get("d");
const s = q.get("s") ?? "home";
const t = q.get("t") === "light" ? "light" : "dark";

const screens: Record<string, Record<string, () => ReactNode>> = {
  a: { home: () => <HomeA />, crew: () => <CrewA /> },
  b: { home: () => <HomeB />, crew: () => <CrewB /> },
  c: { home: () => <HomeC />, crew: () => <CrewC /> },
  a2: { home: () => <HomeG />, "home-empty": () => <HomeG empty />, crew: () => <CrewG />, inbox: () => <InboxG />, palette: () => <PaletteG /> },
};

function Index() {
  const names: Record<string, string> = { a: "A. Graphite (Linear, Raycast)", b: "B. Console (Cursor, Warp)", c: "C. Score (bold)", a2: "A2. Graphite v2 (owner pick, refined)" };
  return (
    <main style={{ font: "14px -apple-system, system-ui", padding: 32 }}>
      <h1 style={{ fontSize: 20 }}>Vajra Desktop art directions</h1>
      {Object.keys(screens).map((k) => (
        <section key={k}>
          <h2 style={{ fontSize: 16 }}>{names[k]}</h2>
          <ul>
            {Object.keys(screens[k]).flatMap((sc) =>
              ["dark", "light"].map((th) => (
                <li key={sc + th}>
                  <a href={`?d=${k}&s=${sc}&t=${th}`}>{`${sc}, ${th}`}</a>
                </li>
              )),
            )}
          </ul>
        </section>
      ))}
    </main>
  );
}

const root = document.documentElement;
if (d && screens[d]) {
  root.dataset.theme = t;
  root.dataset.dir = d === "a2" ? "a" : d;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>{d && screens[d] ? (screens[d][s] ?? screens[d].home)() : <Index />}</StrictMode>,
);

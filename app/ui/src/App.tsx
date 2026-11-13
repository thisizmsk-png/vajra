import { lazy, Suspense } from "react";
import "../components/ui/base";
import { Shell } from "./Shell";

// The component gallery is a developer page (`#kit`). It is its own chunk, so the app never loads or parses it at start.
const Gallery = lazy(() => import("./Gallery").then((m) => ({ default: m.Gallery })));

export function App() {
  if (location.hash === "#kit") return <Suspense fallback={null}><Gallery /></Suspense>;
  return <Shell />;
}

import "../components/ui/base";
import { Gallery } from "./Gallery";
import { Shell } from "./Shell";

export function App() {
  if (location.hash === "#kit") return <Gallery />;
  return <Shell />;
}

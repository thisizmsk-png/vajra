import "../components/ui/base";
import { Gallery } from "./Gallery";

export function App() {
  if (location.hash === "#kit") return <Gallery />;
  return (
    <main>
      <h1>Vajra Desktop</h1>
    </main>
  );
}

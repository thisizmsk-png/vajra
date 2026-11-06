import { cn } from "../cn";

export function Separator({ orientation = "horizontal" }: { orientation?: "horizontal" | "vertical" }) {
  return <div role="separator" aria-orientation={orientation} className={cn("v-sep", `v-sep--${orientation}`)} />;
}

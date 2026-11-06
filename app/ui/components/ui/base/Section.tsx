import { useId } from "react";
import type { ReactNode } from "react";

export function Section({ title, action, children }: { title: string; action?: ReactNode; children?: ReactNode }) {
  const id = useId();
  return (
    <section className="v-section" aria-labelledby={id}>
      <div className="v-section__head">
        <h2 id={id} className="v-text v-text--headline-md">
          {title}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

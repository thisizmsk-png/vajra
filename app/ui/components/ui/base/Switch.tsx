/** `role="switch"` on a button: announced once, as "on" or "off". */
export function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return <button type="button" role="switch" aria-checked={checked} aria-label={label} className="v-switch" onClick={() => onChange(!checked)} />;
}

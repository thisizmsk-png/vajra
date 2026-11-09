// The 24 px status bar: what is running on the left; the phone and Settings on the right. Usage appears here once the
// core reports it (FG-19); until then nothing stands in for it.
import { useEffect, useState } from "react";
import { Icon, ariaShortcut } from "../../components/ui/base";
import { usePreferences } from "../preferences";
import { DEFAULT_PREFERENCES, shortcutKeys } from "../preferenceContract";
import { S } from "../../strings";

export function StatusBar({ runs, compact, onColumn, onPhone, onSettings }: {
  runs: number; compact: boolean | null; onColumn: () => void; onPhone: () => void; onSettings: () => void;
}) {
  const prefs = usePreferences();
  // The usage warning needs the accounts code, so it is loaded (and read, once) only while a run is active: the idle shell never has it.
  const [warn, setWarn] = useState<string | null>(null);
  useEffect(() => {
    if (runs <= 0) { setWarn(null); return; }
    let live = true;
    import("../providerData").then((m) => m.readUsageWarning()).then((t) => { if (live) setWarn(t); }, () => { if (live) setWarn(null); });
    return () => { live = false; };
  }, [runs > 0]);
  const keys = shortcutKeys(prefs.view?.effective.shortcuts.settings ?? DEFAULT_PREFERENCES.shortcuts.settings);
  return (
    <footer className="v-sbar" aria-label={S.frame.status}>
      {compact !== null && (
        <button type="button" className="v-sb-btn" aria-pressed={!compact} aria-label={S.frame.column}
          title={`${compact ? S.frame.showColumn : S.frame.hideColumn} (⌘B)`} aria-keyshortcuts={ariaShortcut(["⌘", "B"])} onClick={onColumn}>
          <Icon name="panelLeft" size={12} />
        </button>
      )}
      <span className={runs > 0 ? "v-sbar__runs v-sbar__runs--live" : "v-sbar__runs"}><i aria-hidden="true" />{S.frame.runs(runs)}</span>
      {warn && <span className="v-sbar__warn" role="status" aria-label={S.workerProvider.usageWarnLabel(warn)}>{warn}</span>}
      <span className="v-sbar__end">
        <button type="button" className="v-sb-btn" onClick={onPhone}><Icon name="phone" size={12} />{S.frame.phone}</button>
        <button type="button" className="v-sb-btn" onClick={onSettings} aria-keyshortcuts={ariaShortcut(keys)} title={`${S.views.settings} (${keys.join("")})`}>
          <Icon name="settings" size={12} />{S.views.settings}
        </button>
      </span>
    </footer>
  );
}

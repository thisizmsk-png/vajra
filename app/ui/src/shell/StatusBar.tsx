// The 24 px status bar: what is running on the left; the phone and Settings on the right. Usage appears here once the
// core reports it (FG-19); until then nothing stands in for it.
import { Icon, ariaShortcut } from "../../components/ui/base";
import { usePreferences } from "../preferences";
import { DEFAULT_PREFERENCES, shortcutKeys } from "../preferenceContract";
import { S } from "../../strings";
import { accountLabel, usageWarning, type UsageList } from "../providerContract";
import { ProviderIcon } from "../screens/ProviderIcon";

export function StatusBar({ runs, usage, compact, onColumn, onSettings }: {
  runs: number; usage?: UsageList | null; compact: boolean | null; onColumn: () => void; onSettings: () => void;
}) {
  const prefs = usePreferences();
  const keys = shortcutKeys(prefs.view?.effective.shortcuts.settings ?? DEFAULT_PREFERENCES.shortcuts.settings);
  const warn = usageWarning(usage);
  return (
    <footer className="v-sbar" aria-label={S.frame.status}>
      {compact !== null && (
        <button type="button" className="v-sb-btn" aria-pressed={!compact} aria-label={S.frame.column}
          title={`${compact ? S.frame.showColumn : S.frame.hideColumn} (⌘B)`} aria-keyshortcuts={ariaShortcut(["⌘", "B"])} onClick={onColumn}>
          <Icon name="panelLeft" size={12} />
        </button>
      )}
      <span className={runs > 0 ? "v-sbar__runs v-sbar__runs--live" : "v-sbar__runs"}><i aria-hidden="true" />{S.frame.runs(runs)}</span>
      {warn && (
        <span className="v-sbar__runs v-sbar__usage" role="status"><i aria-hidden="true" /><ProviderIcon family={warn.account} />{S.frame.usageWarn(accountLabel(warn.account), warn.window.kind, warn.window.used_pct, warn.window.resets_at)}</span>
      )}
      <span className="v-sbar__end">
        <button type="button" className="v-sb-btn" onClick={onSettings} aria-keyshortcuts={ariaShortcut(keys)} title={`${S.views.settings} (${keys.join("")})`}>
          <Icon name="settings" size={12} />{S.views.settings}
        </button>
      </span>
    </footer>
  );
}

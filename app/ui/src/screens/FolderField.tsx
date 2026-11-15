import { useState } from "react";
import { Button, Input } from "../../components/ui/base";
import { S } from "../../strings";
import { api } from "../ipc";

type Props = { label: string; hint?: string; value: string; onChange: (v: string) => void; error?: string };

/** A repository field with the system folder picker beside it (D5), so nobody has to type an absolute path. The text
 *  stays editable for a pasted path; cancelling the picker leaves the field as it was. */
export function FolderField({ label, hint, value, onChange, error }: Props) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const choose = async () => {
    setBusy(true);
    setFailed(false);
    try {
      const picked = await api.chooseFolder(S.folder.title, value.trim() || undefined);
      if (typeof picked === "string" && picked) onChange(picked);
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };
  const set = (v: string) => { setFailed(false); onChange(v); };
  return (
    <Input label={label} hint={hint} value={value} onChange={set} error={failed ? S.folder.failed : error}
      trailing={<Button aria-label={S.folder.chooseLabel} busy={busy} onClick={() => void choose()}>{S.folder.choose}</Button>} />
  );
}

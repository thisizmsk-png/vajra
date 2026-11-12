import { useEffect, useRef, useState } from "react";
import { AlertDialog, Badge, Button, Input, Section, Text } from "../../../components/ui/base";
import { S } from "../../../strings";
import { api, errorText, newActionId, onChanged } from "../../ipc";
import type { ProofLabel, RunProof } from "../../ipc";
import { useCurrentProject } from "../../projects/current";

const L = S.run.proof;

/** One tone per label, matched exhaustively: a new label must choose one here, it can't fall through to "success". */
export function proofTone(l: ProofLabel): "success" | "warning" | "danger" | "info" | undefined {
  switch (l) {
    case "verified":
    case "verified_approved":
      return "success";
    case "evidence_unguarded":
      return "warning";
    case "not_verified":
    case "rejected":
      return "danger";
    case "abandoned":
      return undefined;
    case "in_progress":
      return "info";
  }
}

const PR_RE = /^https:\/\/github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+)\/pull\/([1-9][0-9]{0,8})\/?$/;
export const parsePr = (url: string) => {
  const m = PR_RE.exec(url.trim());
  return m && !m[1].startsWith(".") && !m[2].startsWith(".") ? { repo: `${m[1]}/${m[2]}`, pr: Number(m[3]) } : null;
};

const recordText = (i: RunProof["integrity"]) =>
  i === "ok" || i === "no_report" || i === "not_text" ? L.recordOk : i === "integrity_failed" ? L.recordFailed : L.recordUnknown;

/** The run's label, its signed record and the export (6i). The label and every word of the export come from the core. */
export function Proof({ slug }: { slug: string }) {
  const { project } = useCurrentProject();
  const [proof, setProof] = useState<RunProof | null>(null);
  const [err, setErr] = useState("");
  const [why, setWhy] = useState(false);
  const [note, setNote] = useState("");
  const [pr, setPr] = useState("");
  const [prErr, setPrErr] = useState("");
  const [confirm, setConfirm] = useState<{ repo: string; pr: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const seq = useRef(0);

  useEffect(() => {
    setProof(null); setErr(""); setNote(""); setWhy(false);
    const load = () => {
      const mine = ++seq.current;
      api.runProof(slug).then(
        (p) => { if (mine === seq.current) { setProof(p); setErr(""); } },
        (e) => { if (mine === seq.current) setErr(errorText(e)); },
      );
    };
    load();
    const off = onChanged(load);
    return () => { seq.current++; off(); };
  }, [slug]);

  const finished = proof !== null && proof.label !== "in_progress";
  const exportReason = !proof ? "" : !proof.export ? L.exportOff : !finished ? L.notFinished : "";

  const exportPage = () => {
    setBusy(true);
    api.exportProofPage(slug).then((r) => setNote(L.exported(r.path)), (e) => setNote(L.postFailed(errorText(e)))).finally(() => setBusy(false));
  };
  const copyComment = () => {
    if (!proof) return;
    void navigator.clipboard?.writeText(proof.markdown).then(() => setNote(L.copied), () => undefined);
  };
  const askPost = () => {
    const p = parsePr(pr);
    if (!p) { setPrErr(L.prInvalid); return; }
    setPrErr("");
    setConfirm(p);
  };
  const post = () => {
    const target = confirm;
    setConfirm(null);
    if (!target) return;
    setBusy(true);
    const env = { action_id: newActionId(), project_id: project?.id ?? "global", expect_generation: null };
    api.actPostProofComment(env, slug, pr.trim()).then(
      (r) => setNote(r.result ? L.posted(target.repo, target.pr) : L.postFailed(r.error?.detail ?? r.receipt?.detail ?? S.run.failed)),
      (e) => setNote(L.postFailed(errorText(e))),
    ).finally(() => setBusy(false));
  };

  return (
    <Section title={L.title}>
      {err && <div role="alert"><Text variant="body-sm">{L.failed} {err}</Text></div>}
      {!proof && !err && <Text variant="body-sm" muted>{L.loading}</Text>}
      {proof && (
        <div className="v-proof">
          <div className="v-proof__head">
            <Badge status={proofTone(proof.label)}>{L.label[proof.label]}</Badge>
            <Text as="span" variant="body-sm">{proof.reason}</Text>
            {proof.why && <Button size="sm" variant="ghost" aria-expanded={why} onClick={() => setWhy(!why)}>{L.why}</Button>}
          </div>
          {proof.why && why && (
            <div className="v-proof__why">
              <Text variant="body-sm">{proof.why}</Text>
              {proof.hint && <Text variant="body-sm" muted>{proof.hint}</Text>}
            </div>
          )}
          <dl className="v-proof__record">
            <dt>{L.record}</dt>
            <dd>{recordText(proof.integrity)}</dd>
            {proof.record_sha256 && (
              <>
                <dt>{L.hash}</dt>
                <dd><Text as="code" variant="mono" className="v-proof__hash">{proof.record_sha256}</Text></dd>
              </>
            )}
          </dl>
          <Text variant="body-sm" muted>{proof.record_note}</Text>
          <div className="v-proof__acts">
            <Button size="sm" disabled={!!exportReason} busy={busy} onClick={exportPage} aria-describedby={exportReason ? "v-proof-off" : undefined}>{L.exportPage}</Button>
            <Button size="sm" disabled={!!exportReason} onClick={copyComment} aria-describedby={exportReason ? "v-proof-off" : undefined}>{L.copyComment}</Button>
          </div>
          {exportReason && <div id="v-proof-off"><Text variant="body-sm" muted>{exportReason}</Text></div>}
          {!exportReason && (
            <div className="v-proof__post">
              <Input label={L.prLink} hint={L.prHint} value={pr} onChange={(v) => { setPr(v); setPrErr(""); }} error={prErr || undefined} />
              <Button size="sm" busy={busy} onClick={askPost}>{L.post}</Button>
            </div>
          )}
          {note && <div role="status"><Text variant="body-sm">{note}</Text></div>}
          <AlertDialog open={!!confirm} title={L.confirmTitle} confirmLabel={L.confirmPost} onConfirm={post} onCancel={() => setConfirm(null)}>
            {confirm && <Text variant="body-sm">{L.confirmBody(confirm.repo, confirm.pr)}</Text>}
          </AlertDialog>
        </div>
      )}
    </Section>
  );
}

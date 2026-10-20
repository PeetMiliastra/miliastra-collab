import type { InstallPlan, Run, Snapshot } from "./types";
import { useState, useEffect } from "react";
import { call } from "./bridge";
import { Modal, Progress } from "./components";
import { installationBlocker, sliceStatus } from "./domain";
export function InstallFlow({
  state,
  busy,
  run: runOperation,
  error,
  plan,
  onClose,
  onReview,
  onConflict,
  onDone,
  onInstalled,
  onHistory,
  completion,
}: {
  error?: string;
  state: Snapshot;
  busy: string;
  run: Run;
  plan: InstallPlan | null;
  onClose: () => void;
  onInstalled?: () => void;
  onHistory?: () => void;
  completion?: boolean;
  onReview?: () => void;
  onConflict?: (id: string) => void;
  onDone?: () => void;
}) {
  const [localError, setLocalError] = useState("");
  const run: Run = (label, action) => {
    setLocalError("");
    return runOperation(label, action, {
      surface: "modal",
      onError: setLocalError,
    });
  };
  const [ready, setReady] = useState(false);
  const record = state.installation;
  useEffect(() => {
    setReady(false);
  }, [record?.phase, plan?.id]);
  const pending = record && !["verified", "undone"].includes(record.phase);
  const canVerify =
    record && ["awaitingReload", "undoReload"].includes(record.phase);
  const updates = state.sections.filter((s) => !s.installed);
  const blocker = installationBlocker(state);
  const complete =
    record && !pending && !plan && (!updates.length || completion);
  return (
    <Modal
      title={record?.localRevert ? "Revert slice" : "Apply updates"}
      onClose={onClose}
      closeDisabled={!!busy}
    >
      <div className="modal-progress-slot">
        {busy && <Progress label={`${busy}…`} />}
      </div>
      {(localError || error) && (
        <p className="inline-error" role="alert">
          {localError || error}
        </p>
      )}
      <ol className="task-steps" aria-label="Progress">
        <li aria-current={!pending && !complete ? "step" : undefined}>
          Review
        </li>
        <li aria-current={plan || pending ? "step" : undefined}>Apply</li>
        <li aria-current={complete ? "step" : undefined}>Done</li>
      </ol>
      {plan ? (
        <>
          <h2>Apply downloaded slices</h2>
          <div className="share-sections">
            {plan.revisions.map((r) => (
              <p key={r.section}>{r.name}</p>
            ))}
          </div>
          <label className="install-confirm">
            <input
              type="checkbox"
              disabled={!!busy}
              checked={ready}
              onChange={(e) => setReady(e.target.checked)}
            />
            I saved my work and unloaded this stage in Miliastra.
          </label>
          <p>Afterwards, open the stage in Miliastra to load the updates.</p>
          <div className="modal-actions">
            <button className="button" disabled={!!busy} onClick={onClose}>
              Continue later
            </button>
            <button
              className="button primary"
              disabled={!!busy || !ready}
              onClick={() =>
                run("Backing up and applying slices", async () => {
                  await call("install_updates", {
                    planId: plan.id,
                    editorReady: true,
                  });
                  onInstalled?.();
                })
              }
            >
              Back up and apply
            </button>
          </div>
        </>
      ) : pending ? (
        <>
          <h2>{busy ? "Checking the update" : "Applying was interrupted"}</h2>
          <p>
            {canVerify
              ? "Check the copied files to finish the interrupted operation."
              : "Open History to recover the interrupted update."}
          </p>
          {record.error && (
            <p className="inline-error" role="alert">
              {record.error}
            </p>
          )}
          <div className="modal-actions">
            <button className="button" disabled={!!busy} onClick={onClose}>
              Continue later
            </button>
            {canVerify && (
              <button
                className="button primary"
                disabled={!!busy}
                onClick={() =>
                  run("Checking copied files", () => call("verify_install"))
                }
              >
                Check files and finish
              </button>
            )}
            {onHistory && (!canVerify || record.error) && (
              <button className="button" disabled={!!busy} onClick={onHistory}>
                Open History
              </button>
            )}
          </div>
        </>
      ) : updates.length && !complete ? (
        <>
          <h2>Review downloaded updates</h2>
          <div className="share-sections">
            {updates.map((s) => (
              <div className="review-slice" key={s.id}>
                <div>
                  <strong>{s.name}</strong>
                  {s.incoming && (
                    <p className="review-changelog">
                      {s.incoming.summary} <small>— {s.incoming.author}</small>
                    </p>
                  )}
                </div>
                <span>{sliceStatus(s).label}</span>
                {s.conflict && (
                  <button
                    className="button small"
                    disabled={!!busy}
                    onClick={() => onConflict?.(s.id)}
                  >
                    Review {s.name}
                  </button>
                )}
              </div>
            ))}
          </div>
          {blocker && <p role="status">{blocker}</p>}
          <div className="modal-actions">
            <button className="button" disabled={!!busy} onClick={onClose}>
              Continue later
            </button>
            <button
              className="button primary"
              disabled={!!busy || !!blocker}
              onClick={onReview}
            >
              Continue
            </button>
          </div>
        </>
      ) : (
        <>
          <h2>
            {record?.phase === "undone"
              ? "Undo complete"
              : record?.localRevert
                ? "Slice reverted"
                : record
                  ? "Updates applied"
                  : "You are up to date"}
          </h2>
          {record && (
            <p>
              {record.phase === "undone"
                ? "Open the stage in Miliastra to load the changes."
                : record.localRevert
                  ? "Open the stage in Miliastra to load the reverted slice."
                  : "Open the stage in Miliastra to load them."}
            </p>
          )}
          <div className="modal-actions">
            <button className="button primary" onClick={onDone ?? onClose}>
              Done
            </button>
          </div>
        </>
      )}
    </Modal>
  );
}

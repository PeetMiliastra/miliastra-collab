import { useRef, useState } from "react";
import { call } from "./bridge";
import { Modal, Progress } from "./components";
import type { Installation, Run } from "./types";

/** Local installation recovery is available only from History. */
export function InstallationUndo({
  record,
  busy,
  run,
}: {
  record: Installation;
  busy: string;
  run: Run;
}) {
  const [open, setOpen] = useState(false),
    [ready, setReady] = useState(false),
    [error, setError] = useState(""),
    [pending, setPending] = useState(false),
    [completed, setCompleted] = useState(false);
  const submitting = useRef(false);
  const working = !!busy || pending;
  const complete = completed || record.phase === "undone";
  const verifying = record.phase === "undoReload";
  const close = () => {
    setOpen(false);
    setReady(false);
    setError("");
  };
  const restore = async () => {
    if (working || submitting.current || (!verifying && !ready)) return;
    submitting.current = true;
    setPending(true);
    setError("");
    try {
      await run(
        verifying ? "Checking restored files" : "Restoring previous slices",
        async () => {
          if (verifying) await call("verify_install");
          else await call("undo_install", { editorReady: true });
          setCompleted(true);
          setReady(false);
        },
        { surface: "modal", onError: setError },
      );
    } finally {
      submitting.current = false;
      setPending(false);
    }
  };
  return (
    <>
      {!complete && (
        <button
          className="button"
          disabled={working}
          onClick={() => setOpen(true)}
        >
          {verifying ? "Finish undoing" : "Undo last update"}
        </button>
      )}
      {open && (
        <Modal title="Undo last update" onClose={close} closeDisabled={working}>
          <div className="modal-progress-slot">
            {working && (
              <Progress label={`${busy || "Restoring previous slices"}…`} />
            )}
          </div>
          {error && (
            <p className="inline-error" role="alert">
              {error}
            </p>
          )}
          {complete ? (
            <>
              <h2>Undo complete</h2>
              <p>
                Previous slices restored. Open the stage in Miliastra to load
                them.
              </p>
              <div className="modal-actions">
                <button className="button primary" onClick={close}>
                  Done
                </button>
              </div>
            </>
          ) : (
            <>
              <p>
                {verifying
                  ? "Check the restored files to finish the interrupted operation."
                  : "Put the affected slices back the way they were before the last update. This doesn't work if you've edited them since."}
              </p>
              {!verifying && (
                <label className="install-confirm">
                  <input
                    type="checkbox"
                    checked={ready}
                    disabled={working}
                    onChange={(e) => setReady(e.target.checked)}
                  />
                  I saved my work and unloaded this stage in Miliastra.
                </label>
              )}
              <div className="modal-actions">
                <button className="button" disabled={working} onClick={close}>
                  Cancel
                </button>
                <button
                  className="button primary"
                  disabled={working || (!verifying && !ready)}
                  onClick={restore}
                >
                  {verifying
                    ? "Check files and finish"
                    : "Restore previous slices"}
                </button>
              </div>
            </>
          )}
        </Modal>
      )}
    </>
  );
}

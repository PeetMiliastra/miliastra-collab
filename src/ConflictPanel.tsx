import { useSerialWrites } from "./serialWrites";
import { MAX_SUMMARY_UNITS, MAX_DETAILS_UNITS } from "./commands.generated";
import { useState } from "react";
import { ConflictAssetReview } from "./AssetReview";
import { call } from "./bridge";
import { Modal, Progress } from "./components";
import type { ConflictPreview, Draft, Run } from "./types";
export function ConflictPanel({
  preview,
  online,
  busy,
  run,
  onClose,
}: {
  preview: ConflictPreview;
  online: boolean;
  busy: string;
  run: Run;
  onClose: () => void;
}) {
  const [choice, setChoice] = useState("");
  const [draft, setDraft] = useState<Draft>(preview.draft);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const saves = useSerialWrites();
  function update(next: Draft) {
    setDraft(next);
    const saved = saves.enqueue(() =>
      call("save_conflict_draft", { previewId: preview.id, draft: next }),
    );
    saved.catch((e) => setError(String(e)));
  }
  return (
    <Modal
      title={`Resolve ${preview.name}`}
      subtitle="Both versions are saved on this PC."
      onClose={onClose}
      closeDisabled={!!busy}
    >
      <div className="modal-progress-slot">
        {busy && <Progress label={`${busy}…`} />}
      </div>
      <ConflictAssetReview previewId={preview.id} />
      <div className="conflict-options">
        <label>
          <input
            type="radio"
            name="conflict-choice"
            value="mine"
            checked={choice === "mine"}
            disabled={!!busy || !preview.canKeepMine}
            onChange={() => setChoice("mine")}
          />
          <div>
            <strong>Keep my version</strong>
            <p>
              Share your saved slice as a new update. It replaces the current
              project version after checking your reservation.
            </p>
          </div>
        </label>
        <label>
          <input
            type="radio"
            name="conflict-choice"
            value="theirs"
            checked={choice === "theirs"}
            disabled={!!busy || !preview.canUseTheirs}
            onChange={() => setChoice("theirs")}
          />
          <div>
            <strong>Use teammate’s version</strong>
            <p>Back up the stage and apply their downloaded slice.</p>
            <small>
              {preview.latestUpdate?.author ?? "Teammate"}
              {preview.latestUpdate ? `: ${preview.latestUpdate.summary}` : ""}
            </small>
          </div>
        </label>
      </div>
      {(!preview.canKeepMine || !preview.canUseTheirs) && (
        <p className="muted">{preview.reason}</p>
      )}
      {choice === "mine" && (
        <>
          <label className="field">
            What changed? <span>Required</span>
            <input
              aria-label="Conflict changelog"
              disabled={!!busy}
              maxLength={MAX_SUMMARY_UNITS}
              value={draft.summary}
              onChange={(e) => update({ ...draft, summary: e.target.value })}
              placeholder="Kept the new quest route after reviewing both versions"
            />
          </label>
          <label className="field">
            Details <span>Optional</span>
            <textarea
              aria-label="Conflict details"
              disabled={!!busy}
              rows={2}
              maxLength={MAX_DETAILS_UNITS}
              value={draft.details}
              onChange={(e) => update({ ...draft, details: e.target.value })}
            />
          </label>
        </>
      )}

      {choice === "mine" && !online && (
        <p className="muted">
          You’re offline. This needs the project connection.
        </p>
      )}
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      <div className="modal-actions conflict-actions">
        <label className="install-confirm">
          <input
            type="checkbox"
            disabled={!!busy}
            checked={ready}
            onChange={(e) => setReady(e.target.checked)}
          />
          I saved my work and unloaded this stage in Miliastra.
        </label>
        <button
          className="button"
          disabled={!!busy}
          onClick={() =>
            run(
              "Opening preserved versions",
              () => call("open_preserved_conflict", { previewId: preview.id }),
              { surface: "modal", onError: setError },
            )
          }
        >
          Open preserved copies
        </button>
        <button className="button" disabled={!!busy} onClick={onClose}>
          Keep both for now
        </button>
        <button
          className="button primary"
          disabled={
            !!busy ||
            !ready ||
            !choice ||
            (choice === "mine" && (!online || !draft.summary.trim()))
          }
          onClick={() =>
            run(
              "Resolving slice conflict",
              async () => {
                setError("");
                await saves.flush();
                await call("resolve_conflict", {
                  previewId: preview.id,
                  choice,
                  draft,
                  editorReady: true,
                });
                onClose();
              },
              { surface: "modal", onError: setError },
            )
          }
        >
          {choice === "mine"
            ? "Share my version"
            : choice === "theirs"
              ? "Back up and use teammate’s version"
              : "Choose a version"}
        </button>
      </div>
    </Modal>
  );
}

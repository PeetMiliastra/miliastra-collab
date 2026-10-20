import { useReview } from "./useReview";
import { useCallback, useEffect, useRef, useState } from "react";
import { AssetSummary } from "./AssetReview";
import { AssetBrowser } from "./AssetBrowser";
import { call } from "./bridge";
import { Modal } from "./components";
import type { MoveId, Run, Section, SliceInspection, Snapshot } from "./types";

export function SliceChanges({
  section,
  onClose,
}: {
  section: Section;
  onClose: () => void;
}) {
  const load = useCallback(
    () => call("inspect_slice", { section: section.id }),
    [section.id],
  );
  const { data, error } = useReview(load);
  return (
    <Modal title={`Changes in ${section.name}`} onClose={onClose}>
      <p>Saved changes since the last version applied on this device.</p>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {!data && !error && <p role="status">Reading saved assets…</p>}
      {data && <AssetSummary data={data} />}
      <div className="modal-actions">
        <button className="button" onClick={onClose}>
          Close
        </button>
      </div>
    </Modal>
  );
}
export function RevertSlice({
  section,
  busy,
  run,
  onClose,
  onReverted,
}: {
  section: Section;
  busy: boolean;
  run: Run;
  onClose: () => void;
  onReverted: () => void;
}) {
  const [ready, setReady] = useState(false),
    [error, setError] = useState(""),
    [reverting, setReverting] = useState(false);
  const submitting = useRef(false);
  const working = busy || reverting;
  const revert = async () => {
    if (!ready || busy || submitting.current) return;
    submitting.current = true;
    setReverting(true);
    setError("");
    try {
      await run(
        "Backing up and reverting slice",
        async () => {
          const preview = await call("preview_revert", { section: section.id });
          await call("revert_slice", {
            previewId: preview.id,
            editorReady: true,
          });
          onReverted();
        },
        { surface: "modal", onError: setError },
      );
    } finally {
      submitting.current = false;
      setReverting(false);
    }
  };
  return (
    <Modal
      title={`Revert ${section.name}?`}
      onClose={onClose}
      closeDisabled={working}
    >
      <p>
        All edits in this slice will be replaced with the last-shared version.
      </p>
      <p>
        This only changes your local stage. Nothing is shared with teammates.
      </p>
      {section.id === "shared" && (
        <p>Other slices may refer to shared settings removed by this revert.</p>
      )}
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
      <label className="install-confirm">
        <input
          type="checkbox"
          checked={ready}
          disabled={working}
          onChange={(e) => setReady(e.target.checked)}
        />
        I saved my work and unloaded this stage in Miliastra.
      </label>
      <div className="modal-actions">
        <button className="button" disabled={working} onClick={onClose}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={working || !ready}
          onClick={revert}
        >
          {working ? "Reverting…" : "Back up and revert"}
        </button>
      </div>
    </Modal>
  );
}
export function MoveAsset({
  state,
  busy,
  run,
  onClose,
}: {
  state: Snapshot;
  busy: boolean;
  run: Run;
  onClose: () => void;
}) {
  const slices = state.sections.filter((s) => s.id !== "shared");
  const [source, setSource] = useState(slices[0]?.id ?? ""),
    [target, setTarget] = useState("");
  const [data, setData] = useState<SliceInspection | null>(null),
    [assetKey, setAssetKey] = useState("");
  const [generated, setGenerated] = useState<MoveId | null>(null),
    [copied, setCopied] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setData(null);
    setAssetKey("");
    setGenerated(null);
    setCopied(false);
    setError("");
    if (source)
      call("inspect_slice", { section: source })
        .then((value) => {
          if (active) {
            setData(value);
          }
        })
        .catch((e) => {
          if (active) setError(String(e));
        });
    return () => {
      active = false;
    };
  }, [source]);
  const reset = () => {
    setGenerated(null);
    setCopied(false);
    setError("");
  };
  return (
    <Modal
      title="Move asset between slices"
      className={`guid-helper${generated ? " has-generated-id" : ""}`}
      onClose={onClose}
      closeDisabled={busy}
    >
      <p>This generates a new GUID to move your asset to another slice.</p>
      <p className="asset-warning">
        <strong>Changing the GUID breaks any references to this asset.</strong>
      </p>
      <AssetBrowser
        key={source}
        slices={slices}
        source={source}
        assets={data?.section === source ? data.assets : null}
        selected={assetKey}
        busy={busy}
        onSourceChange={(value) => {
          setSource(value);
          setData(null);
          setAssetKey("");
          reset();
          if (value === target) setTarget("");
        }}
        onSelect={(key) => {
          setAssetKey(key);
          reset();
        }}
      />
      <div className="move-destination">
        <div className="move-selection">
          <span>Selected asset</span>
          <strong>
            {data?.assets.find((asset) => asset.key === assetKey)?.name ??
              "Choose an asset above"}
          </strong>
        </div>
        <label className="field">
          To slice
          <select
            aria-label="To slice"
            value={target}
            disabled={busy}
            onChange={(e) => {
              setTarget(e.target.value);
              reset();
            }}
          >
            <option value="">Choose a destination</option>
            {slices
              .filter((s) => s.id !== source)
              .map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} (slice {Number(s.id) + 1})
                </option>
              ))}
          </select>
        </label>
      </div>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {generated && (
        <div className="generated-id">
          <label className="field">
            New GUID
            <input readOnly value={generated.newId} />
          </label>
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              run(
                "Copying GUID",
                async () => {
                  setError("");
                  const id = await call("validate_move_id", {
                    token: generated.token,
                  });
                  await navigator.clipboard.writeText(id);
                  setCopied(true);
                },
                { surface: "modal", onError: setError },
              )
            }
          >
            {copied ? "Copied" : "Copy GUID"}
          </button>
          <p>
            In Miliastra, change the GUID for {generated.name} to this and save
            the stage.
          </p>
        </div>
      )}
      <div className="modal-actions">
        <button className="button" disabled={busy} onClick={onClose}>
          Close
        </button>
        <button
          className="button primary"
          disabled={
            busy ||
            !assetKey ||
            !target ||
            !data?.assets.some((a) => a.key === assetKey && a.canGenerateId)
          }
          onClick={() =>
            run(
              "Generating GUID",
              async () => {
                reset();
                setGenerated(
                  await call("generate_move_id", {
                    source,
                    assetKey,
                    target,
                  }),
                );
              },
              { surface: "modal", onError: setError },
            )
          }
        >
          Generate GUID
        </button>
      </div>
    </Modal>
  );
}

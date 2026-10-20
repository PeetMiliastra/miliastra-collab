import { useCallback, useState } from "react";
import { call } from "./bridge";
import { useReview } from "./useReview";
import type { SliceInspection } from "./types";

export function AssetSummary({ data }: { data: SliceInspection }) {
  const counts = ["Added", "Modified", "Deleted"]
    .map((status) => ({
      status,
      count: data.changes.filter((c) => c.status === status).length,
    }))
    .filter((c) => c.count);
  return (
    <div className="asset-review-content">
      {!data.baselineAvailable && (
        <p role="status">
          The previous version is not available on this device. Changes cannot
          be identified yet.
        </p>
      )}
      {!!counts.length && (
        <p className="asset-counts">
          {counts.map((c) => `${c.count} ${c.status.toLowerCase()}`).join(", ")}
        </p>
      )}
      <ul className="asset-changes">
        {data.changes.map((change) => (
          <li key={change.asset.key}>
            <span className="pill blue-pill">{change.status}</span>
            <div className="asset-change-details">
              <strong>{change.asset.name}</strong>
              <small>{change.asset.kind}</small>
              <small className="asset-id">ID: {change.asset.id}</small>
              {change.beforeName && (
                <small>Previously {change.beforeName}</small>
              )}
            </div>
          </li>
        ))}
      </ul>
      {data.baselineAvailable && data.otherChanges && (
        <p>
          {data.changes.length
            ? "Other unidentified changes"
            : "Cannot identify specific changes"}
        </p>
      )}
      {data.baselineAvailable && !data.otherChanges && !data.changes.length && (
        <p>No saved asset changes found.</p>
      )}
    </div>
  );
}
/** Optional asset inspection, local only. It never writes the changelog for you. */
export function SavedAssetReview({
  section,
  disabled,
}: {
  section: string;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false),
    [attempt, setAttempt] = useState(0);
  const load = useCallback(
    () => call("inspect_slice", { section }),
    [section, attempt],
  );
  const { data, error } = useReview(load, open);
  return (
    <details
      className="asset-review"
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary>Changes</summary>
      {open && (
        <>
          {!data && !error && <p role="status">Reading saved assets…</p>}
          {error && (
            <p role="alert" className="inline-error">
              Asset review is unavailable. {error} You can still write your
              changelog and share.
            </p>
          )}
          {data && <AssetSummary data={data} />}
          <button
            className="text-button"
            disabled={disabled || (!data && !error)}
            onClick={() => setAttempt((a) => a + 1)}
          >
            Refresh asset review
          </button>
        </>
      )}
    </details>
  );
}
export function ConflictAssetReview({ previewId }: { previewId: string }) {
  const [open, setOpen] = useState(false),
    [attempt, setAttempt] = useState(0);
  const load = useCallback(
    () => call("inspect_conflict", { previewId }),
    [previewId, attempt],
  );
  const { data, error } = useReview(load, open);
  return (
    <details
      className="asset-review"
      onToggle={(e) => setOpen(e.currentTarget.open)}
    >
      <summary>Compare saved asset changes</summary>
      {open && (
        <>
          <p>
            Each preserved version is compared with the same version last
            applied on this PC. Choosing a version keeps the whole slice; assets
            are not merged.
          </p>
          {!data && !error && (
            <p role="status">Reading both preserved versions…</p>
          )}
          {data && (
            <div className="asset-comparison">
              <section aria-label="My asset changes">
                <h3>My version</h3>
                <AssetSummary data={data.mine} />
              </section>
              <section aria-label="Teammate’s asset changes">
                <h3>Teammate’s version</h3>
                <AssetSummary data={data.theirs} />
              </section>
            </div>
          )}
          {error && (
            <>
              <p className="inline-error" role="alert">
                Asset comparison is unavailable. {error} Open the preserved
                copies to inspect them.
              </p>
              <button
                className="text-button"
                onClick={() => setAttempt((a) => a + 1)}
              >
                Retry asset comparison
              </button>
            </>
          )}
        </>
      )}
    </details>
  );
}

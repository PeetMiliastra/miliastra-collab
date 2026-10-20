import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Download, LoaderCircle } from "lucide-react";
import { call, desktop } from "./bridge";
import type { Run } from "./types";
export type AppUpdate = { version: string; notes: string; canReplace: boolean };
type Progress = { message: string; percent: number | null };

export const FIRST_UPDATE_CHECK_MS = 10_000;
export const UPDATE_CHECK_EVERY_MS = 6 * 60 * 60 * 1000;

/**
 * Looks for a newer app version shortly after start, every few hours, and straight away
 * when the service says this version is no longer compatible. Failures stay quiet:
 * builds without an update feed, or being offline, just mean no banner.
 */
export function useAppUpdateCheck(protocolMismatch: boolean) {
  const [update, setUpdate] = useState<AppUpdate | null>(null);
  const check = useCallback(() => {
    call("check_application_update")
      .then(setUpdate)
      .catch(() => {});
  }, []);
  useEffect(() => {
    if (!desktop) return;
    const first = setTimeout(check, FIRST_UPDATE_CHECK_MS);
    const every = setInterval(check, UPDATE_CHECK_EVERY_MS);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, [check]);
  useEffect(() => {
    if (desktop && protocolMismatch) check();
  }, [protocolMismatch, check]);
  return update;
}

/** One-click update. It can't be put off when this version no longer works with the service. */
export function UpdateBanner({
  update,
  required,
  busy,
  run,
  onLater,
}: {
  update: AppUpdate;
  required: boolean;
  busy: string;
  run: Run;
  onLater: () => void;
}) {
  const [saved, setSaved] = useState(false);
  return (
    <div
      className={`banner${required ? " error" : ""}`}
      role="status"
      aria-label="App update"
    >
      <Download size={18} />
      <div>
        <strong>{required ? "Update required" : "Update available"}</strong>
        <p>
          {required
            ? `Your project service now needs version ${update.version}. `
            : `Version ${update.version} is ready. `}
          {update.canReplace
            ? "The app restarts in a few seconds and keeps your work."
            : "This app’s folder is read-only, so save the new version somewhere else and open it from there."}
        </p>
        {saved && <p>Downloaded. Close this app and open the new copy.</p>}
      </div>
      <div className="recovery-actions">
        <button
          className="button small primary"
          disabled={!!busy}
          onClick={() =>
            update.canReplace
              ? run("Downloading the new app", () =>
                  call("install_application_update"),
                )
              : run("Saving the updated app", async () => {
                  setSaved(!!(await call("download_application_update")));
                })
          }
        >
          {update.canReplace ? "Restart and update" : "Download updated app"}
        </button>
        {!required && (
          <button className="text-button" disabled={!!busy} onClick={onLater}>
            Later
          </button>
        )}
      </div>
    </div>
  );
}
export function ApplicationUpdates({ busy, run }: { busy: string; run: Run }) {
  const [update, setUpdate] = useState<AppUpdate | null>(null);
  const [checked, setChecked] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [error, setError] = useState("");
  const [downloaded, setDownloaded] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    const pending = listen<Progress>("application-update-progress", (e) =>
      setProgress(e.payload),
    );
    return () => {
      void pending.then((stop) => stop());
    };
  }, []);
  const perform = (label: string, action: () => Promise<unknown>) => {
    setError("");
    setDownloaded(false);
    setProgress({ message: label, percent: null });
    void run(
      label,
      async () => {
        try {
          await action();
        } finally {
          setProgress(null);
        }
      },
      { onError: setError },
    );
  };
  return (
    <div className="settings-card">
      <h2>Application updates</h2>
      <button
        className="button small"
        disabled={!!busy || !desktop}
        onClick={() =>
          perform("Checking for an app update…", async () => {
            setUpdate(await call("check_application_update"));
            setChecked(true);
          })
        }
      >
        Check for an app update
      </button>
      {checked && !update && !error && <p>You are up to date.</p>}
      {update && (
        <>
          <h3>Version {update.version}</h3>
          {update.notes && <p>{update.notes}</p>}
          {!update.canReplace && (
            <p>
              This app folder is read-only. Save the updated app elsewhere, then
              open the new copy.
            </p>
          )}
          {update.canReplace && (
            <button
              className="button primary"
              disabled={!!busy}
              onClick={() =>
                perform("Downloading the new app…", () =>
                  call("install_application_update"),
                )
              }
            >
              Restart and update
            </button>
          )}
          {(!update.canReplace || error) && (
            <button
              className="button"
              disabled={!!busy}
              onClick={() =>
                perform("Saving the updated app…", async () => {
                  const path = await call("download_application_update");
                  setDownloaded(!!path);
                })
              }
            >
              Download updated app
            </button>
          )}
        </>
      )}
      <div className="app-update-progress" role="status" aria-live="polite">
        {progress && (
          <>
            <LoaderCircle className="spin" size={16} />
            <span>
              {progress.message}
              {progress.percent !== null ? " " + progress.percent + "%" : ""}
            </span>
          </>
        )}
        {downloaded && (
          <span>Downloaded. Close this app and open the new copy.</span>
        )}
      </div>
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

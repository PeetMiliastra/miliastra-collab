import { InstallationUndo } from "./InstallationUndo";
import { MAX_SUMMARY_UNITS } from "./commands.generated";
import { Fragment, useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { desktop } from "./bridge";
import { ChevronRight, History } from "lucide-react";
import { call } from "./bridge";
import { initials, updateStatus } from "./domain";
import { Modal } from "./components";
import type { Revision, Run, Snapshot, Update } from "./types";
export function HistoryPage({
  state,
  busy,
  run,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
}) {
  const [entries, setEntries] = useState<Update[]>([]),
    [next, setNext] = useState<number | null>(null),
    [detail, setDetail] = useState<Update | null>(null),
    [section, setSection] = useState(""),
    [error, setError] = useState(""),
    [restoring, setRestoring] = useState<{
      update: Update;
      revision: Revision;
    } | null>(null);
  const requestRef = useRef(0);
  const [loading, setLoading] = useState(false);
  const fetchPage = useCallback(
    async (before?: number) => {
      if (!state.project) return;
      const request = ++requestRef.current;
      setLoading(true);
      try {
        const p = await call("history_page", {
          filter: {
            section,
            before: before ?? null,
          },
        });
        if (request !== requestRef.current) return;
        setEntries((old) => (before ? [...old, ...p.entries] : p.entries));
        setNext(p.next);
        setError("");
      } catch (e) {
        if (request === requestRef.current) setError(String(e));
      } finally {
        if (request === requestRef.current) setLoading(false);
      }
    },
    [state.project?.id, state.project?.sequence, section],
  );
  useEffect(() => {
    const t = setTimeout(() => fetchPage(), 200);
    return () => {
      clearTimeout(t);
      requestRef.current++;
    };
  }, [fetchPage]);
  useEffect(() => {
    if (!desktop) return;
    const p = listen("history-changed", () => fetchPage());
    return () => {
      p.then((f) => f());
    };
  }, [fetchPage]);
  return (
    <>
      <div className="page-heading">
        <div className="heading-row">
          <div>
            <h1>History</h1>
          </div>
        </div>
      </div>
      <div className="history-toolbar">
        <select
          aria-label="Slice"
          value={section}
          onChange={(e) => setSection(e.target.value)}
        >
          <option value="">All slices</option>
          {state.sections.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
        {state.project && state.installation && (
          <InstallationUndo
            key={`${state.project.id}/${state.installation.id}`}
            record={state.installation}
            busy={busy}
            run={run}
          />
        )}
      </div>
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
      <div className="timeline">
        {!entries.length ? (
          <div className="empty-state">
            <History size={34} />
            <h2>
              {loading
                ? "Loading history…"
                : section
                  ? "No matching updates"
                  : "No updates yet"}
            </h2>
            <p>
              {section
                ? "Choose another slice."
                : "Shared updates and their changelogs appear here."}
            </p>
          </div>
        ) : (
          entries.map((u, index) => (
            <Fragment key={u.id}>
              {(index === 0 ||
                new Date(entries[index - 1].at * 1000).toDateString() !==
                  new Date(u.at * 1000).toDateString()) && (
                <h2 className="history-day">{historyDay(u.at)}</h2>
              )}
              <button
                className="history-entry"
                key={u.id}
                onClick={() => setDetail(u)}
              >
                <span className="avatar timeline-avatar">
                  {initials(u.author)}
                </span>
                <div className="entry-content">
                  <h3>{u.summary}</h3>
                  <div className="entry-meta">
                    <strong>{u.author}</strong>
                    <time
                      dateTime={new Date(u.at * 1000).toISOString()}
                      title={new Date(u.at * 1000).toLocaleString()}
                    >
                      {new Date(u.at * 1000).toLocaleTimeString([], {
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </time>
                  </div>
                  <div className="entry-tags">
                    {!(
                      u.sequence === 1 && u.summary === "Initialized project"
                    ) &&
                      u.revisions.map((r) => (
                        <span className="section-tag" key={r.section}>
                          {r.name}
                        </span>
                      ))}
                    {u.nameChanges?.map((c) => (
                      <span className="section-tag" key={c.section}>
                        {c.before} → {c.after}
                      </span>
                    ))}
                  </div>
                </div>
                <span
                  className={`pill ${updateStatus(u, state) === "Applied" ? "green" : ""}`}
                >
                  {updateStatus(u, state)}
                </span>
                <ChevronRight size={17} />
              </button>
            </Fragment>
          ))
        )}
      </div>
      {next !== null && (
        <button
          className="button load-more"
          disabled={loading}
          onClick={() => fetchPage(next)}
        >
          Load earlier changes
        </button>
      )}
      {detail && (
        <Modal
          title={detail.summary}
          subtitle={`Update #${detail.sequence} by ${detail.author}, ${new Date(detail.at * 1000).toLocaleString()}`}
          onClose={() => setDetail(null)}
        >
          <div className="entry-tags">
            {!(
              detail.sequence === 1 && detail.summary === "Initialized project"
            ) &&
              detail.revisions.map((r) => (
                <span className="section-tag" key={r.section}>
                  {r.name}
                </span>
              ))}
          </div>
          {detail.nameChanges?.map((c) => (
            <p key={c.section}>
              <strong>{c.before}</strong> → <strong>{c.after}</strong>
            </p>
          ))}
          {detail.details && <p className="update-details">{detail.details}</p>}
          {detail.restores && (
            <p className="muted">Restores a previous project version.</p>
          )}
          {!!detail.revisions.length &&
            !!state.project &&
            state.compatibility.restoreVersions && (
              <ul className="restore-list" aria-label="Versions in this update">
                {detail.revisions.map((r) => {
                  const current =
                    state.project?.heads[r.section]?.hash === r.hash;
                  return (
                    <li key={r.section}>
                      <span>{r.name}</span>
                      {current ? (
                        <small className="muted">Current version</small>
                      ) : (
                        <button
                          className="button small"
                          disabled={!!busy || !state.online}
                          onClick={() =>
                            setRestoring({ update: detail, revision: r })
                          }
                        >
                          Restore this version
                        </button>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          {!!detail.revisions.length && !state.online && (
            <p className="muted">
              You’re offline. This needs the project connection.
            </p>
          )}
        </Modal>
      )}
      {restoring && (
        <RestoreDialog
          state={state}
          busy={busy}
          run={run}
          update={restoring.update}
          revision={restoring.revision}
          onClose={() => setRestoring(null)}
          onRestored={() => {
            setRestoring(null);
            setDetail(null);
            void fetchPage();
          }}
        />
      )}
    </>
  );
}
/** Restoring shares the old version again as a new update; History isn't changed. */
function RestoreDialog({
  state,
  busy,
  run,
  update,
  revision,
  onClose,
  onRestored,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
  update: Update;
  revision: Revision;
  onClose: () => void;
  onRestored: () => void;
}) {
  const [reason, setReason] = useState(""),
    [error, setError] = useState("");
  const slice = state.sections.find((s) => s.id === revision.section);
  const holder =
    slice?.lock && slice.lock.member !== state.member ? slice.lock.name : null;
  const blocker = slice?.changed
    ? `You have unshared changes to ${revision.name}. Share them first; restoring would replace them when you apply it.`
    : holder
      ? `${holder} has ${revision.name} reserved. Ask them to finish first, or take it over.`
      : null;
  return (
    <Modal title={`Restore ${revision.name}?`} onClose={onClose}>
      <p>
        This brings back {revision.name} as it was in “{update.summary}” by{" "}
        {update.author} ({new Date(update.at * 1000).toLocaleDateString()}).
        It’s shared as a new update, so History keeps every version. Everyone,
        including you, applies it like any other update, with a backup to undo.
      </p>
      {blocker && (
        <p role="alert" className="inline-error">
          {blocker}
        </p>
      )}
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {!state.online && (
        <p className="muted">
          You’re offline. This needs the project connection.
        </p>
      )}
      <label className="field">
        Why are you restoring it? <span>Required</span>
        <input
          autoFocus
          maxLength={MAX_SUMMARY_UNITS}
          value={reason}
          disabled={!!busy || !!blocker}
          placeholder="The new boat layout broke the docking quest"
          onChange={(e) => setReason(e.target.value)}
        />
      </label>
      <div className="modal-actions">
        <button className="button" disabled={!!busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={!!busy || !!blocker || !reason.trim() || !state.online}
          onClick={() =>
            run(
              "Restoring version",
              async () => {
                await call("restore_version", {
                  updateId: update.id,
                  section: revision.section,
                  reason: reason.trim(),
                });
                onRestored();
              },
              { surface: "modal", onError: setError },
            )
          }
        >
          Restore this version
        </button>
      </div>
    </Modal>
  );
}
function historyDay(at: number) {
  const date = new Date(at * 1000),
    today = new Date(),
    yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return "Today";
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  return date.toLocaleDateString([], {
    day: "numeric",
    month: "long",
    year: "numeric",
  });
}

import { MAX_SUMMARY_UNITS, MAX_DETAILS_UNITS } from "./commands.generated";
import { SavedAssetReview } from "./AssetReview";
import { SliceChanges, RevertSlice, MoveAsset } from "./SliceTools";
import { SliceName } from "./SectionNames";
import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowUpRight,
  Check,
  Folder,
  LoaderCircle,
  Users,
  ChevronRight,
  AlertTriangle,
  MonitorX,
} from "lucide-react";
import { call, desktop } from "./bridge";
import {
  canShare,
  initials,
  installationBlocker,
  nextStep,
  people,
  sliceStatus,
} from "./domain";
import type { Draft, Run, Section, Snapshot } from "./types";
import { Modal } from "./components";
export function ProjectPage({
  state,
  busy,
  run,
  onSetup,
  onDemo,
  onShare,
  onHistory,
  onApply,
  onConflict,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
  onSetup: () => void;
  onDemo?: () => void;
  onShare: () => void;
  onHistory: () => void;
  onApply?: () => void;
  onConflict?: (section: string) => void;
}) {
  const [recent, setRecent] = useState<{ id: string; name: string }[]>([]);
  useEffect(() => {
    if (desktop)
      call("connections")
        .then(setRecent)
        .catch(() => {});
  }, [state.project?.id]);
  const changed = state.sections.filter((s) => s.changed),
    updates = state.sections.filter(
      (s) => !s.installed && !s.comparisonPending,
    );
  const checking = state.sections.filter((s) => s.comparisonPending);
  const shareable = changed.filter((s) => !s.conflict);
  const conflicts = state.sections.filter((s) => s.conflict);
  const blocker = installationBlocker(state);
  const step = nextStep(state);
  const incomingFrom = people(
    updates.flatMap((s) => (s.incoming ? [s.incoming.author] : [])),
  );
  const shown = state.sections;
  const review = (section: string) => onConflict?.(section);
  const [changesSlice, setChangesSlice] = useState<Section | null>(null);
  const [revertSlice, setRevertSlice] = useState<Section | null>(null);
  const [guidHelper, setGuidHelper] = useState(false);
  const [takeover, setTakeover] = useState<Section | null>(null);
  return (
    <>
      <div className="page-heading">
        <div className="heading-row">
          <div>
            <h1>{state.project?.name ?? "Your projects"}</h1>
          </div>
          {state.project && (
            <button
              className={`button${step === "share" || step === null ? " primary" : ""}`}
              aria-label={`Share update (${shareable.length} slices)`}
              disabled={!!busy || changed.length === 0}
              onClick={onShare}
            >
              <ArrowUpRight size={17} />
              Share update
              {shareable.length > 0 && (
                <span className="button-count">{shareable.length}</span>
              )}
            </button>
          )}
        </div>
      </div>
      {!state.project ? (
        <>
          {recent.length > 0 && (
            <div className="recent-projects">
              {recent.map((p) => (
                <button
                  className="button"
                  key={p.id}
                  disabled={!!busy}
                  onClick={() =>
                    run("Opening your workspace", () =>
                      call("reopen_project", { projectId: p.id }),
                    )
                  }
                >
                  <Folder size={15} />
                  {p.name}
                  <ChevronRight size={14} />
                </button>
              ))}
            </div>
          )}
          {!recent.length && <p className="muted">No projects yet.</p>}
          {state.testProfile && <PasteInvitation busy={busy} run={run} />}
          <div className="project-actions">
            <button
              className="button primary"
              disabled={!desktop || !!busy}
              onClick={() => onSetup()}
            >
              Create project
            </button>
            {onDemo && (
              <button className="button" disabled={!!busy} onClick={onDemo}>
                Explore a demo project
              </button>
            )}
          </div>
        </>
      ) : (
        <>
          {(checking.length > 0 ||
            conflicts.length > 0 ||
            updates.length > 0) && (
            <div className="attention-list">
              {checking.length > 0 && (
                <div className="attention-row">
                  <LoaderCircle size={20} className="spin" aria-hidden="true" />
                  <div>
                    <strong>Checking saved slices</strong>
                    <p>
                      Comparing your saved stage with the project.{" "}
                      {checking.length} remaining.
                    </p>
                  </div>
                </div>
              )}
              {conflicts.length > 0 && (
                <div className="attention-row conflict-attention">
                  <AlertTriangle size={20} aria-hidden="true" />
                  <div>
                    <strong>
                      {conflicts.length === 1
                        ? `You and a teammate both changed ${conflicts[0].name}`
                        : `${conflicts.length} slices were changed by you and a teammate`}
                    </strong>
                    <p>
                      Look at both versions and choose which to keep before
                      sharing or applying.
                    </p>
                  </div>
                  <button
                    className={`button${step === "conflict" ? " primary" : ""}`}
                    disabled={!!busy}
                    onClick={() => review(conflicts[0].id)}
                  >
                    Review conflict
                  </button>
                </div>
              )}
              {updates.length > 0 && (
                <div className="attention-row update-banner">
                  <div>
                    <strong>
                      {step === "install"
                        ? "Finish applying updates"
                        : updates.every((s) => s.downloaded)
                          ? `${updates.length} update${updates.length === 1 ? "" : "s"} ready from ${incomingFrom}`
                          : `Getting ${updates.length} update${updates.length === 1 ? "" : "s"} from ${incomingFrom}`}
                    </strong>
                    {updates.some((s) => s.incoming) && (
                      <ul className="incoming-list">
                        {updates
                          .filter((s) => s.incoming)
                          .slice(0, 3)
                          .map((s) => (
                            <li key={s.id}>
                              <span>{s.name}</span> {s.incoming!.summary}{" "}
                              <small>— {s.incoming!.author}</small>
                            </li>
                          ))}
                      </ul>
                    )}
                    <p id="install-blocker">
                      {blocker ??
                        "Save your work and unload the stage in Miliastra before applying."}
                    </p>
                  </div>
                  <button className="text-button" onClick={onHistory}>
                    View updates
                  </button>
                  <button
                    className={`button${step === "apply" || step === "install" ? " primary" : ""}`}
                    disabled={!!busy}
                    aria-describedby="install-blocker"
                    onClick={onApply}
                  >
                    Review and apply
                  </button>
                </div>
              )}
            </div>
          )}
          <CollaboratorCards state={state} />
          {state.game.exclusiveFullscreen && (
            <div className="banner" role="status">
              <MonitorX size={18} />
              <div>
                <strong>
                  Notifications are hidden in exclusive fullscreen
                </strong>
                <p>
                  Notifications can’t appear over Miliastra in exclusive
                  fullscreen, so they waited until you left the game. Switch the
                  game to borderless or windowed mode to see them as they
                  happen.
                </p>
              </div>
            </div>
          )}
          {state.removed && (
            <div className="banner error" role="alert">
              <Users size={18} />
              <div>
                <strong>You were removed from this project</strong>
                <p>
                  You can no longer reserve or share slices here. Your stage
                  files stay on this PC. Ask the owner to invite you again if
                  this was a mistake.
                </p>
              </div>
              <button
                className="button"
                disabled={!!busy}
                onClick={() =>
                  run("Leaving project", () => call("forget_project"))
                }
              >
                Leave this project
              </button>
            </div>
          )}
          {state.nameSync?.error && (
            <p role="alert" className="inline-error">
              Slice names could not be synced. Retrying automatically.{" "}
              {state.nameSync.error}
            </p>
          )}
          <div className="section-heading">
            <h2>Slices</h2>
            <button
              className="button small"
              disabled={!!busy}
              onClick={() => setGuidHelper(true)}
            >
              Move asset between slices
            </button>
          </div>
          <table className="section-list">
            <thead>
              <tr className="table-head">
                <th scope="col">Slice</th>
                <th scope="col">Reserved by</th>
                <th scope="col">Status</th>
                <th scope="col">Action</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((s) => (
                <SectionRow
                  key={s.id}
                  section={s}
                  state={state}
                  run={run}
                  me={state.member}
                  busy={Boolean(busy)}
                  online={state.online}
                  onChanges={() => setChangesSlice(s)}
                  onRevert={() => setRevertSlice(s)}
                  onResolve={() => review(s.id)}
                  onTakeover={() => setTakeover(s)}
                  onAsk={() =>
                    run("Asking to finish", () =>
                      call("request_finish", { section: s.id }),
                    )
                  }
                  onStart={(takeover) =>
                    run("Reserving slice", () =>
                      call("start_work", { section: s.id, takeover }),
                    )
                  }
                  onRelease={() =>
                    run("Releasing slice", () =>
                      call("finish_work", { section: s.id }),
                    )
                  }
                />
              ))}
            </tbody>
          </table>
          {changesSlice && (
            <SliceChanges
              section={changesSlice}
              onClose={() => setChangesSlice(null)}
            />
          )}
          {revertSlice && (
            <RevertSlice
              section={revertSlice}
              busy={!!busy}
              run={run}
              onClose={() => setRevertSlice(null)}
              onReverted={() => {
                setRevertSlice(null);
                onApply?.();
              }}
            />
          )}
          {guidHelper && (
            <MoveAsset
              state={state}
              busy={!!busy}
              run={run}
              onClose={() => setGuidHelper(false)}
            />
          )}
          {takeover && (
            <TakeoverDialog
              section={takeover}
              busy={!!busy}
              onClose={() => setTakeover(null)}
              onConfirm={() =>
                run("Taking over slice", async () => {
                  await call("start_work", {
                    section: takeover.id,
                    takeover: true,
                  });
                  setTakeover(null);
                })
              }
            />
          )}
        </>
      )}
    </>
  );
}
/** Anyone can take over, but the current holder may still be editing. */
function TakeoverDialog({
  section,
  busy,
  onClose,
  onConfirm,
}: {
  section: Section;
  busy: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal title={`Take over ${section.name}?`} onClose={onClose}>
      <p>
        {section.lock?.name ?? "A teammate"} reserved this slice and may still
        be working on it. If you both change it, one of you will have to choose
        which version to keep.
      </p>
      <p className="muted">
        {section.lock?.name ?? "They"} will be told you took it over. Consider
        using Ask to finish first.
      </p>
      <div className="modal-actions">
        <button className="button" disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button className="button primary" disabled={busy} onClick={onConfirm}>
          Take over
        </button>
      </div>
    </Modal>
  );
}
/** Test profiles can't receive invitation links from the browser. */
function PasteInvitation({ busy, run }: { busy: string; run: Run }) {
  const [link, setLink] = useState("");
  return (
    <div className="paste-invitation">
      <label className="field">
        Invitation link <span>Test profile</span>
        <input
          placeholder="http://127.0.0.1:8787/invite/…#…"
          value={link}
          onChange={(e) => setLink(e.target.value)}
        />
      </label>
      <button
        className="button"
        disabled={!!busy || !link.trim()}
        onClick={() =>
          run("Opening invitation", async () => {
            await call("open_invitation_link", { link: link.trim() });
            setLink("");
          })
        }
      >
        Open invitation
      </button>
    </div>
  );
}
/** "In Miliastra" or "Online", for you or a teammate. */
function PresenceBadge({ state, member }: { state: Snapshot; member: string }) {
  const mine = member === state.member;
  const report = state.project?.presence?.find((p) => p.member === member);
  const online = mine ? state.online : !!report;
  const inGame = mine ? state.game.running : !!report?.status?.game;
  if (!online && !inGame) return null;
  return (
    <span
      className={`presence-badge ${inGame ? "in-game" : "online"}`}
      title={
        inGame
          ? "Has Miliastra open right now"
          : "Has miliastra/collab open right now"
      }
    >
      {inGame ? "In Miliastra" : "Online"}
    </span>
  );
}
function CollaboratorCards({ state }: { state: Snapshot }) {
  const members = [...(state.project?.members ?? [])];
  if (state.member && !members.some((m) => m.id === state.member))
    members.unshift({ id: state.member, name: "You", owner: false });
  const ordered = [...members].sort(
    (a, b) => Number(b.id === state.member) - Number(a.id === state.member),
  );
  return (
    <section className="collaborator-cards" aria-label="Collaborator activity">
      {ordered.map((m) => {
        const mine = m.id === state.member;
        const slices = state.sections.filter(
          (s) => s.lock?.member === m.id || (mine && s.changed),
        );
        return (
          <article
            className={`collaborator-card ${mine ? "self" : ""}`}
            aria-label={mine ? "Your work" : `${m.name}’s work`}
            key={m.id}
          >
            <div className="collaborator-heading">
              <span className="avatar mini">{initials(m.name)}</span>
              <h2
                title={
                  m.joined
                    ? `Joined ${new Date(m.joined * 1000).toLocaleDateString()}`
                    : undefined
                }
              >
                {mine ? (m.name === "You" ? "You" : `${m.name} (you)`) : m.name}
              </h2>
              {m.owner && <small>owner</small>}
              <PresenceBadge state={state} member={m.id} />
            </div>
            {slices.length ? (
              <ul className="slice-badges">
                {slices.map((s) => (
                  <li
                    key={s.id}
                    className={`slice-badge ${mine && s.changed ? "changed" : ""}`}
                    title={
                      mine && s.changed
                        ? s.conflict
                          ? "Your saved changes need review"
                          : s.lock && s.lock.member !== state.member
                            ? `Your saved changes; ${s.lock.name} has the reservation`
                            : "Your saved changes"
                        : "Reserved"
                    }
                  >
                    {s.name}
                    {mine && s.changed && (
                      <span className="badge-state">
                        {s.conflict ? "Needs review" : "Changed"}
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">No reserved slices</p>
            )}
          </article>
        );
      })}
    </section>
  );
}
function SectionRow({
  section: s,
  state,
  run,
  me,
  busy,
  online,
  onStart,
  onResolve,
  onTakeover,
  onAsk,
  onRelease,
  onChanges,
  onRevert,
}: {
  section: Section;
  state: Snapshot;
  run: Run;
  me: string | null;
  busy: boolean;
  online: boolean;
  onStart: (takeover: boolean) => void;
  onResolve: () => void;
  onTakeover: () => void;
  onAsk: () => void;
  onRelease: () => void;
  onChanges: () => void;
  onRevert: () => void;
}) {
  const mine = s.lock?.member === me;
  return (
    <tr className="section-row">
      <td className="section-name">
        <span className={`section-symbol ${s.id === "shared" ? "shared" : ""}`}>
          <Folder size={18} />
        </span>
        <span
          className="slice-number"
          aria-label={
            s.id === "shared" ? "Shared settings" : `Slice ${Number(s.id) + 1}`
          }
        >
          {s.id === "shared" ? "—" : Number(s.id) + 1}
        </span>
        <SliceName state={state} section={s} busy={busy} run={run} />
      </td>
      <td data-label="Reserved by">
        {s.lock ? (
          <span className="working-person">
            <span className="avatar mini">{initials(s.lock.name)}</span>
            {mine ? "You" : s.lock.name}
          </span>
        ) : (
          <span className="subtle">Available</span>
        )}
      </td>
      <td>
        {(() => {
          const status = sliceStatus(s);
          return status.tone === "clean" ? (
            <span className="clean-status">
              <Check size={14} />
              {status.label}
            </span>
          ) : (
            <span
              className={`pill ${status.tone === "amber" ? "amber-pill" : "blue-pill"}`}
              title={
                s.incoming
                  ? `${s.incoming.author}: ${s.incoming.summary}`
                  : undefined
              }
            >
              {status.label}
            </span>
          );
        })()}
      </td>
      <td className="row-action">
        <div
          className={`row-action-buttons${s.changed ? " slice-edit-actions" : ""}`}
        >
          {s.changed && (
            <>
              <button
                className="button small"
                disabled={busy}
                onClick={onChanges}
              >
                View changes
              </button>
              <button
                className="button small"
                disabled={busy || !!s.installationPending}
                onClick={onRevert}
              >
                Revert
              </button>
            </>
          )}
          {s.conflict ? (
            <>
              <button
                className="button small"
                disabled={busy}
                onClick={onResolve}
              >
                Review conflict
              </button>
              {s.lock && !mine && (
                <button
                  className="button small"
                  disabled={busy || !online}
                  onClick={onTakeover}
                >
                  Take over
                </button>
              )}
            </>
          ) : mine ? (
            s.changed ? (
              <span className="subtle" title="Share your changes to release it">
                Reserved by you
              </span>
            ) : (
              <button
                className="button small"
                disabled={busy || !online || !!s.installationPending}
                title={
                  s.installationPending
                    ? "Finish applying the update first"
                    : undefined
                }
                onClick={onRelease}
              >
                Release
              </button>
            )
          ) : s.lock ? (
            <>
              <button
                className="button small"
                disabled={busy || !online}
                onClick={onAsk}
              >
                Ask to finish
              </button>
              <button
                className="button small"
                disabled={busy || !online}
                onClick={onTakeover}
              >
                Take over
              </button>
            </>
          ) : (
            <button
              className="button small"
              disabled={busy || !online}
              onClick={() => onStart(false)}
            >
              Start work
              <ArrowUpRight size={13} />
            </button>
          )}
        </div>
      </td>
    </tr>
  );
}
export function SharePanel({
  state,
  busy,
  draft,
  setDraft,
  run,
  onClose,
  task,
  onSelection,
  onConflict,
  onShared,
  error,
}: {
  state: Snapshot;
  busy: string;
  draft: Draft;
  setDraft: (d: Draft) => void;
  run: Run;
  onClose: () => void;
  task?: import("./types").WorkflowTask;
  onSelection?: (selected: string[]) => void;
  onConflict?: (id: string) => void;
  onShared?: () => void | Promise<void>;
  error?: string;
}) {
  const [progress, setProgress] = useState("");
  const [keepReserved, setKeepReserved] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    const off = listen<{ project: string; message: string }>(
      "publication-progress",
      (e) => {
        if (e.payload.project === state.project?.id)
          setProgress(e.payload.message);
      },
    );
    return () => {
      void off.then((f) => f());
    };
  }, [state.project?.id]);
  useEffect(() => {
    void run("Reading saved changes", () => call("refresh_saved_changes"));
  }, []);
  useEffect(() => {
    if (busy === "Sharing your update") setProgress("");
  }, [busy]);
  const changed = state.sections.filter((s) => s.changed);
  const [fallbackSelected, setSelected] = useState(
    changed.filter((s) => !s.conflict).map((s) => s.id),
  );
  const selected = task
    ? task.manualSelection
      ? task.selected
      : changed.filter((s) => !s.conflict).map((s) => s.id)
    : fallbackSelected;
  const changeSelection = (next: string[]) => {
    setSelected(next);
    onSelection?.(next);
  };
  const eligible = selected.filter((id) =>
    changed.some((s) => s.id === id && !s.conflict),
  );
  const blocked = changed.filter((s) => s.conflict && selected.includes(s.id));
  return (
    <Modal title="Share update" onClose={onClose} closeDisabled={!!busy}>
      <div className="modal-progress-slot">
        {busy && (
          <div className="progress" role="status">
            <LoaderCircle size={16} className="spin" />
            <span>
              {busy === "Sharing your update"
                ? progress || "Preparing your update…"
                : `${busy}…`}
            </span>
          </div>
        )}
      </div>
      {error && (
        <p role="alert" className="inline-error">
          {error}
        </p>
      )}
      {!state.online && (
        <p role="status">
          You’re offline. Your draft is kept on this PC, and you can share once
          the project connection is back.
        </p>
      )}
      {state.metadataPending && (
        <p role="status">
          Update details are still downloading. Review them before sharing; your
          draft is saved.
        </p>
      )}
      {!changed.length && <p>No saved slice changes detected yet.</p>}
      <div className="share-sections">
        {changed.map((s) => (
          <div key={s.id} className="share-slice-review">
            <label>
              <input
                type="checkbox"
                checked={selected.includes(s.id)}
                disabled={!!busy}
                onChange={(e) =>
                  changeSelection(
                    e.target.checked
                      ? [...selected, s.id]
                      : selected.filter((x) => x !== s.id),
                  )
                }
              />
              <span>{s.name}</span>
              <small>
                {s.conflict ? "Conflict — review below" : "Saved changes"}
              </small>
            </label>
            <SavedAssetReview section={s.id} disabled={!!busy} />
          </div>
        ))}
      </div>
      {changed
        .filter((s) => s.conflict)
        .map((s) => (
          <button
            key={s.id}
            className="button small"
            disabled={!!busy}
            onClick={() => onConflict?.(s.id)}
          >
            Review {s.name}
          </button>
        ))}
      <label className="field">
        What changed? <span>Required</span>
        <input
          autoFocus
          disabled={!!busy}
          maxLength={MAX_SUMMARY_UNITS}
          placeholder="Fixed passengers falling through the boat deck"
          value={draft.summary}
          onChange={(e) => setDraft({ ...draft, summary: e.target.value })}
        />
      </label>
      <label className="field">
        Details <span>Optional</span>
        <textarea
          rows={4}
          disabled={!!busy}
          maxLength={MAX_DETAILS_UNITS}
          placeholder="Details your teammates should know…"
          value={draft.details}
          onChange={(e) => setDraft({ ...draft, details: e.target.value })}
        />
      </label>
      {blocked.length > 0 && (
        <p role="status">
          Review {blocked.map((s) => s.name).join(", ")} before sharing{" "}
          {blocked.length === 1 ? "it" : "them"}, or untick{" "}
          {blocked.length === 1 ? "it" : "them"}.
        </p>
      )}
      <label className="check-field">
        <input
          type="checkbox"
          checked={keepReserved}
          disabled={!!busy}
          onChange={(e) => setKeepReserved(e.target.checked)}
        />
        Keep these slices reserved
      </label>
      <div className="modal-actions share-actions">
        <button className="button" disabled={!!busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className="button primary"
          disabled={
            !!busy ||
            blocked.length > 0 ||
            state.metadataPending ||
            !canShare(draft.summary, eligible, state.online)
          }
          onClick={() =>
            run("Sharing your update", async () => {
              await call("save_draft", { draft, projectId: state.project!.id });
              await call("share_update", {
                sections: eligible,
                draft,
                keepReserved,
              });
              setDraft({ summary: "", details: "" });
              await (onShared ?? onClose)();
            })
          }
        >
          {busy === "Sharing your update" ? (
            <LoaderCircle size={16} className="spin" />
          ) : (
            <ArrowUpRight size={17} />
          )}
          Share update
        </button>
      </div>
    </Modal>
  );
}

import { useSerialWrites } from "./serialWrites";
import { useEffect, useState } from "react";
import { call } from "./bridge";
import { SharePanel } from "./ProjectPage";
import { InstallFlow } from "./InstallFlow";
import { ConflictPanel } from "./ConflictPanel";
import type {
  Snapshot,
  WorkflowTask,
  Draft,
  Run,
  ConflictPreview,
  InstallPlan,
} from "./types";
export type TaskRequest = {
  kind: "share" | "apply";
  section?: string;
  nonce: number;
};
export function Workflow({
  state,
  busy,
  run,
  draft,
  setDraft,
  request,
  onHistory,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
  draft: Draft;
  setDraft: (d: Draft) => void;
  request: TaskRequest | null;
  onHistory?: () => void;
}) {
  const [task, setTask] = useState<WorkflowTask | null>(state.task ?? null),
    [open, setOpen] = useState(false),
    [conflict, setConflict] = useState<ConflictPreview | null>(null),
    [plan, setPlan] = useState<InstallPlan | null>(null),
    [shareError, setShareError] = useState(""),
    [reviewError, setReviewError] = useState("");
  const runShare: Run = (label, action) => {
    setShareError("");
    return run(label, action, { surface: "modal", onError: setShareError });
  };
  const saves = useSerialWrites();
  const projectId = state.project!.id;
  const pending =
    !!state.installation &&
    !["verified", "undone"].includes(state.installation.phase);
  function persist(next: WorkflowTask | null) {
    setTask(next);
    const saved = saves.enqueue(() =>
      call("save_task", { projectId, task: next }),
    );
    // Show save failures as a normal task error.
    void saved.catch((e) =>
      run("Saving task", async () => {
        throw e;
      }),
    );
  }
  const review = (section: string) => {
    setReviewError("");
    return run(
      "Opening the conflict",
      async () => {
        setConflict(await call("preview_conflict", { section }));
      },
      { surface: "modal", onError: setReviewError },
    );
  };
  useEffect(() => {
    if (!request) return;
    setReviewError("");
    const next =
      task?.kind === request.kind
        ? task
        : { kind: request.kind, selected: [], manualSelection: false };
    persist(next);
    setOpen(true);
    setPlan(null);
    if (request.section) void review(request.section);
  }, [request?.nonce]);
  useEffect(() => {
    if (state.task?.installationId)
      setTask((old) =>
        old && old.kind === state.task?.kind
          ? { ...old, installationId: state.task.installationId }
          : old,
      );
  }, [state.task?.installationId]);
  const close = () => {
    setOpen(false);
    setConflict(null);
    setPlan(null);
  };
  const completeTask = async () => {
    await saves.flush();
    await call("save_task", { projectId, task: null });
    setTask(null);
    setOpen(false);
    setPlan(null);
  };
  const done = () => run("Finishing task", completeTask);
  const showApply = task?.kind === "apply" || pending;
  return (
    <>
      {!open && pending && (
        <div className="banner task-resume">
          <div>
            <strong>
              {state.installation?.localRevert
                ? "Finish reverting slice"
                : "Finish applying updates"}
            </strong>
          </div>
          <button
            className="button"
            disabled={!!busy}
            onClick={() => {
              if (!task)
                persist({
                  kind: "apply",
                  selected: [],
                  manualSelection: false,
                  installationId: state.installation?.id ?? null,
                });
              setOpen(true);
            }}
          >
            Resume
          </button>
        </div>
      )}
      {open && conflict ? (
        <ConflictPanel
          online={state.online}
          key={conflict.id}
          preview={conflict}
          busy={busy}
          run={run}
          onClose={() => setConflict(null)}
        />
      ) : open && showApply ? (
        <InstallFlow
          state={state}
          busy={busy}
          run={run}
          plan={pending ? null : plan}
          onClose={close}
          onHistory={
            onHistory
              ? () => {
                  close();
                  onHistory();
                }
              : undefined
          }
          error={reviewError}
          completion={
            !!state.installation &&
            task?.installationId === state.installation.id
          }
          onConflict={review}
          onDone={done}
          onInstalled={() => setPlan(null)}
          onReview={() => {
            if (pending) {
              setPlan(null);
              return;
            }
            setReviewError("");
            void run(
              "Checking downloaded slices",
              async () => setPlan(await call("preview_install")),
              { surface: "modal", onError: setReviewError },
            );
          }}
        />
      ) : open && task?.kind === "share" ? (
        <SharePanel
          state={state}
          busy={busy}
          draft={draft}
          setDraft={setDraft}
          run={runShare}
          task={task}
          onSelection={(selected) =>
            persist({ ...task, selected, manualSelection: true })
          }
          onConflict={review}
          onClose={close}
          onShared={completeTask}
          error={shareError || reviewError}
        />
      ) : null}
    </>
  );
}

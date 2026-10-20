import { useSerialWrites } from "./serialWrites";
import { defaultCloudProviders, providerName } from "./providers";
import { Modal, Progress } from "./components";
import { beginDemo, endDemo, isDemo } from "./demo";
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ShieldCheck,
  LayoutGrid,
  History,
  Settings2,
  SlidersHorizontal,
  Users,
  ArrowLeft,
  RotateCcw,
  X,
  CheckCheck,
} from "lucide-react";
import { call, desktop } from "./bridge";
import { friendlyError } from "./domain";
import type { Draft, Run, Snapshot } from "./types";
import { Workflow, type TaskRequest } from "./Workflow";
import { RecoveryNotice, ProjectConnectionNotice } from "./RecoveryNotice";
import { ProjectPage } from "./ProjectPage";
import { HistoryPage } from "./HistoryPage";
import { Setup } from "./Setup";
import { AppSettings, ProjectSettings } from "./Settings";
import { UpdateBanner, useAppUpdateCheck } from "./ApplicationUpdates";
import { invitationMessage } from "./TeamPanel";
export const empty: Snapshot = {
  settings: {
    endpoint: "https://collabs.miliastra.dev",
    saveRoot: "",
    oauth: {
      microsoftClientId: "",
      googleClientId: "",
      googleClientSecret: "",
    },
    autostart: false,
    autoApply: true,
    screenRecognition: false,
  },
  stage: null,
  project: null,
  member: null,
  sections: [],
  draft: { summary: "", details: "" },
  online: false,
  protocolMismatch: false,
  message: "",
  compatibility: {
    directInstall: false,
    liveDetection: false,
    restoreVersions: false,
    reason: "Choose a stage to apply updates.",
  },
  game: { running: false, foreground: false },
};
function Overlay() {
  const [message, setMessage] = useState("");
  useEffect(() => {
    document.documentElement.classList.add("overlay-body");
    document.body.classList.add("overlay-body");
    const p = listen<string>("overlay-message", (e) => setMessage(e.payload));
    return () => {
      p.then((f) => f());
      document.body.classList.remove("overlay-body");
      document.documentElement.classList.remove("overlay-body");
    };
  }, []);
  return message ? (
    <div className="game-notice">
      <span className="logo-mark">
        <img src="/app-logo.png" alt="" />
      </span>
      <div>
        <strong>miliastra/collab</strong>
        <p>{message}</p>
      </div>
    </div>
  ) : null;
}
export default function App() {
  return new URLSearchParams(location.search).has("overlay") ? (
    <Overlay />
  ) : (
    <Workspace />
  );
}
function Workspace() {
  const [state, setState] = useState<Snapshot>(empty),
    [tab, setTab] = useState<
      "project" | "history" | "project-settings" | "settings"
    >("project"),
    [busy, setBusy] = useState(""),
    [busySurface, setBusySurface] = useState<"page" | "modal">("page"),
    [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [setup, setSetup] = useState(false),
    [request, setRequest] = useState<TaskRequest | null>(null),
    [invitation, setInvitation] = useState<{
      project: string;
      projectName: string;
      link: string;
      key: string;
    } | null>(null),
    [draft, setDraft] = useState<Draft>(empty.draft),
    [askAutostart, setAskAutostart] = useState(false),
    [updateLater, setUpdateLater] = useState(false);
  const projectRef = useRef<string | null>(null);
  const reloadRevisionRef = useRef(0);
  const savedDraftRef = useRef(JSON.stringify(empty.draft));
  const draftTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingDraftRef = useSerialWrites();
  const reload = useCallback(async () => {
    const revision = ++reloadRevisionRef.current;
    if (!desktop && !isDemo()) {
      setState(empty);
      setDraft(empty.draft);
      return;
    }
    try {
      const demoAtStart = !desktop && isDemo();
      const s = await call("snapshot");
      if (
        revision !== reloadRevisionRef.current ||
        (!desktop && isDemo()) !== demoAtStart
      )
        return;
      setState(s);
      if (projectRef.current !== s.project?.id) {
        projectRef.current = s.project?.id ?? null;
        setRequest(null);
        setInvitation(null);
        savedDraftRef.current = JSON.stringify(s.draft);
        setDraft(s.draft);
      }
      const invitation = await call("pending_invitation");
      if (revision === reloadRevisionRef.current && invitation) setSetup(true);
    } catch (e) {
      if (revision === reloadRevisionRef.current) setError(friendlyError(e));
    }
  }, []);
  useEffect(() => {
    reload();
    const demoReload = () => {
      void reload();
    };
    const previewNotice = (e: Event) =>
      setNotice((e as CustomEvent<string>).detail);
    window.addEventListener("preview-notice", previewNotice);
    window.addEventListener("demo-changed", demoReload);
    if (!desktop)
      return () => {
        window.removeEventListener("demo-changed", demoReload);
        window.removeEventListener("preview-notice", previewNotice);
      };
    const hs = [
      listen("draft-shared", () => setDraft({ summary: "", details: "" })),
      listen("state-changed", reload),
      listen("invitation-opened", reload),
      listen("review-and-apply", () =>
        setRequest({ kind: "apply", nonce: Date.now() }),
      ),
      listen<string>("invitation-error", (e) => setError(e.payload)),
      // Alerts also emit state-changed, which reloads the snapshot.
      listen<string>("notice", (e) => setNotice(e.payload)),
    ];
    return () => {
      hs.forEach((p) => p.then((f) => f()));
      window.removeEventListener("demo-changed", demoReload);
      window.removeEventListener("preview-notice", previewNotice);
    };
  }, [reload]);
  useEffect(() => {
    if (!state.project && (tab === "history" || tab === "project-settings"))
      setTab("project");
  }, [state.project, tab]);
  // An error belongs to the page it happened on.
  useEffect(() => setError(""), [tab]);
  useEffect(() => {
    if (!notice) return;
    const t = setTimeout(() => setNotice(""), 7000);
    return () => clearTimeout(t);
  }, [notice]);
  useEffect(() => {
    if (
      !state.project ||
      (!desktop && !isDemo()) ||
      JSON.stringify(draft) === savedDraftRef.current
    )
      return;
    const projectId = state.project.id;
    const value = JSON.stringify(draft);
    const t = setTimeout(() => {
      const pending = pendingDraftRef.enqueue(() =>
        call("save_draft", { draft, projectId }),
      );
      void pending
        .then(() => {
          if (projectRef.current === projectId) savedDraftRef.current = value;
        })
        .catch((e) =>
          setError(`Could not save changelog draft: ${friendlyError(e)}`),
        );
    }, 250);
    draftTimerRef.current = t;
    return () => clearTimeout(t);
  }, [draft, state.project?.id]);
  const run: Run = async (label, action, options) => {
    setBusySurface(options?.surface ?? "page");
    setBusy(label);
    setError("");
    try {
      await action();
      await reload();
    } catch (e) {
      const message = friendlyError(e);
      if (options?.onError) options.onError(message);
      else setError(message);
    } finally {
      setBusy("");
    }
  };
  const goToProjects = () =>
    run("Opening your projects", async () => {
      if (state.project) {
        if (draftTimerRef.current) clearTimeout(draftTimerRef.current);
        // Let a running autosave finish before saving the latest text.
        await pendingDraftRef.flush().catch(() => {});
        await call("save_draft", { projectId: state.project.id, draft });
        savedDraftRef.current = JSON.stringify(draft);
        await call("leave_project");
      }
      setRequest(null);
      setSetup(false);
      setInvitation(null);
      setTab("project");
    });
  const appUpdate = useAppUpdateCheck(state.protocolMismatch);
  const me = state.project?.members.find((m) => m.id === state.member);
  // Matches the count on the Share update button.
  const shareable = state.sections.filter(
    (s) => s.changed && !s.conflict,
  ).length;
  return (
    <div className="shell">
      <aside className="sidebar">
        <a
          className="brand"
          href="#"
          onClick={(e) => {
            e.preventDefault();
            setTab("project");
          }}
        >
          <span className="logo-mark">
            <img src="/app-logo.png" alt="" />
          </span>
          <span>miliastra/collab</span>
        </a>
        {state.testProfile && (
          <span className="test-profile" title="Test profile">
            Test profile {state.testProfile}
          </span>
        )}
        {state.project && (
          <button
            className="all-projects"
            disabled={!!busy}
            onClick={goToProjects}
          >
            <ArrowLeft size={17} />
            All projects
          </button>
        )}
        {state.project && (
          <div className="project-switch">
            <strong title={state.project.name}>{state.project.name}</strong>
            <small>{providerName(state.project.storage.kind)}</small>
          </div>
        )}
        <nav aria-label="Main navigation">
          {(
            [
              ["project", LayoutGrid, state.project ? "Slices" : "Projects"],
              ["history", History, "History"],
              ["project-settings", SlidersHorizontal, "Project settings"],
              ["settings", Settings2, "App settings"],
            ] as const
          )
            .filter(
              ([id]) =>
                state.project ||
                (id !== "history" && id !== "project-settings"),
            )
            .map(([id, Icon, title]) => (
              <button
                key={id}
                className={tab === id ? "active" : ""}
                aria-current={tab === id ? "page" : undefined}
                onClick={() => setTab(id)}
              >
                <Icon size={18} />
                {title}
                {id === "project" && shareable > 0 && (
                  <span className="nav-count">{shareable}</span>
                )}
              </button>
            ))}
        </nav>
      </aside>
      <main>
        <header className="topbar">
          <div className="top-actions">
            {state.project && (
              <>
                <button
                  className="button small"
                  disabled={!!busy}
                  onClick={() =>
                    run("Checking for changes", () => call("refresh"))
                  }
                >
                  <RotateCcw size={14} />
                  Check for changes
                </button>
                {me && (
                  <button
                    className="button small"
                    disabled={!!busy}
                    onClick={() =>
                      run("Preparing invitation", async () => {
                        const project = state.project!;
                        const key = await call("folder_privacy_key");
                        const link = await call("invite");
                        if (projectRef.current === project.id)
                          setInvitation({
                            project: project.id,
                            projectName: project.name,
                            link,
                            key,
                          });
                      })
                    }
                  >
                    <Users size={14} />
                    Invite creators
                  </button>
                )}
              </>
            )}
          </div>
        </header>
        <div className="content">
          {appUpdate && (state.protocolMismatch || !updateLater) && (
            <UpdateBanner
              update={appUpdate}
              required={state.protocolMismatch}
              busy={busy}
              run={run}
              onLater={() => setUpdateLater(true)}
            />
          )}
          <ProjectConnectionNotice state={state} busy={busy} run={run} />
          {!desktop && isDemo() && (
            <div className="banner demo-banner">
              <div>
                <strong>Browser preview</strong>
                <p>Use the Windows app to work with real files.</p>
              </div>
              <button
                className="button small"
                disabled={!!busy}
                onClick={() => {
                  endDemo();
                  setTab("project");
                }}
              >
                Leave preview
              </button>
            </div>
          )}
          {!desktop && !isDemo() && (
            <div className="banner">
              <ShieldCheck size={18} />
              <div>
                <strong>Desktop preview</strong>
                <p>Open the Windows app to connect a project.</p>
              </div>
            </div>
          )}
          {error && (
            <div className="banner error" role="alert">
              <span>{error}</span>
              <button aria-label="Dismiss error" onClick={() => setError("")}>
                <X size={17} />
              </button>
            </div>
          )}
          <RecoveryNotice
            state={state}
            busy={busy}
            run={run}
            onAccounts={() => setTab("settings")}
            onTeam={() => setTab("project-settings")}
          />
          <div className="progress-slot" data-testid="page-progress-slot">
            {busy && busySurface === "page" && <Progress label={`${busy}…`} />}
          </div>
          {state.project && (
            <Workflow
              key={state.project.id}
              state={state}
              busy={busy}
              run={run}
              draft={draft}
              setDraft={setDraft}
              request={request}
              onHistory={() => setTab("history")}
            />
          )}
          {tab === "project" && (
            <ProjectPage
              state={state}
              busy={busy}
              run={run}
              onSetup={() => {
                setSetup(true);
              }}
              // The sample project only exists in the browser preview.
              onDemo={
                desktop
                  ? undefined
                  : () => {
                      beginDemo();
                      setTab("project");
                    }
              }
              onShare={() => setRequest({ kind: "share", nonce: Date.now() })}
              onApply={() => setRequest({ kind: "apply", nonce: Date.now() })}
              onConflict={(section) =>
                setRequest({ kind: "apply", section, nonce: Date.now() })
              }
              onHistory={() => setTab("history")}
            />
          )}
          {tab === "history" && (
            <HistoryPage state={state} busy={busy} run={run} />
          )}
          {tab === "project-settings" && (
            <ProjectSettings state={state} busy={busy} run={run} />
          )}
          {tab === "settings" && (
            <AppSettings state={state} busy={busy} run={run} />
          )}
        </div>
      </main>
      {notice && !desktop && (
        <div className="toast" role="status">
          <CheckCheck size={19} />
          <span>{notice}</span>
          <button
            onClick={() => setNotice("")}
            aria-label="Dismiss notification"
          >
            <X size={16} />
          </button>
        </div>
      )}
      {invitation && (
        <Modal title="Invite a creator" onClose={() => setInvitation(null)}>
          <label className="field">
            Invitation message
            <textarea
              className="invite-message"
              aria-label="Invitation message"
              readOnly
              rows={9}
              value={invitationMessage(
                invitation.projectName,
                invitation.link,
                invitation.key,
              )}
              onFocus={(e) => e.target.select()}
            />
          </label>
          <p>Send this message to your teammate.</p>
          {state.project?.storage.kind === "folder" && (
            <p>Give them access to the team folder too.</p>
          )}
          <div className="modal-actions">
            <button
              className="text-button"
              onClick={() =>
                run("Copying invitation link", async () => {
                  await navigator.clipboard.writeText(invitation.link);
                  setNotice("Invitation link copied.");
                })
              }
            >
              Copy link only
            </button>
            <button className="button" onClick={() => setInvitation(null)}>
              Done
            </button>
            <button
              className="button primary"
              onClick={() =>
                run("Copying invitation", async () => {
                  await navigator.clipboard.writeText(
                    invitationMessage(
                      invitation.projectName,
                      invitation.link,
                      invitation.key,
                    ),
                  );
                  setNotice(
                    "Invitation copied. Send the message to your teammate.",
                  );
                })
              }
            >
              Copy invitation
            </button>
          </div>
        </Modal>
      )}
      {setup && (
        <Setup
          onBeforeJoin={async () => {
            if (!state.project) return;
            if (draftTimerRef.current) clearTimeout(draftTimerRef.current);
            await pendingDraftRef.flush().catch(() => {});
            await call("save_draft", { projectId: state.project.id, draft });
            await call("leave_project");
          }}
          providers={state.cloudProviders ?? defaultCloudProviders}
          storageAccounts={state.storageAccounts}
          run={run}
          busy={busy}
          onClose={() => {
            setSetup(false);
            call("dismiss_invitation").catch(() => {});
          }}
          onDone={() => {
            setSetup(false);
            if (desktop && !state.settings.autostart && !state.testProfile) {
              let asked = false;
              try {
                asked = localStorage.getItem("autostart-asked") === "1";
                localStorage.setItem("autostart-asked", "1");
              } catch {}
              if (!asked) setAskAutostart(true);
            }
            reload();
          }}
        />
      )}
      {askAutostart && (
        <Modal
          title="Keep your team in sync"
          onClose={() => setAskAutostart(false)}
        >
          <p>
            miliastra/collab reserves the slices you edit and brings in your
            teammates’ updates while it runs. Start it with Windows so this
            happens without you having to remember.
          </p>
          <div className="modal-actions">
            <button className="button" onClick={() => setAskAutostart(false)}>
              Not now
            </button>
            <button
              className="button primary"
              autoFocus
              onClick={() =>
                run("Saving preference", async () => {
                  await call("save_settings", {
                    settings: { ...state.settings, autostart: true },
                  });
                  setAskAutostart(false);
                })
              }
            >
              Start with Windows
            </button>
          </div>
        </Modal>
      )}
    </div>
  );
}

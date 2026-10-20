import { MAX_NAME_UNITS } from "./commands.generated";
import {
  defaultCloudProviders,
  isCloudProvider,
  providerName,
} from "./providers";
import { useEffect, useRef, useState } from "react";
import { ChevronRight, Folder, LoaderCircle } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { call, desktop } from "./bridge";
import { friendlyError } from "./domain";
import { Modal } from "./components";
import { JoinWizard } from "./JoinWizard";
import { ConnectionSuccess, StorageConnection } from "./StorageConnection";
import type { Invitation, Run, Stage, StorageAccounts } from "./types";
export function Setup({
  storageAccounts,
  run,
  busy,
  onClose,
  onDone,
  onBeforeJoin,
  providers = defaultCloudProviders,
}: {
  providers?: string[];
  storageAccounts?: StorageAccounts;
  run: Run;
  busy: string;
  onClose: () => void;
  onDone: () => void;
  onBeforeJoin?: () => Promise<void>;
}) {
  const [name, setName] = useState(""),
    [display, setDisplay] = useState("");
  const [stages, setStages] = useState<Stage[]>([]),
    [stage, setStage] = useState("");
  const [kind, setKind] = useState<string>(() =>
      providers.includes("onedrive") ? "onedrive" : (providers[0] ?? "folder"),
    ),
    [folder, setFolder] = useState<string | null>(null);
  const [invitation, setInvitation] = useState<Invitation | null>(null);
  const [error, setError] = useState(""),
    [connectedProvider, setConnectedProvider] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false),
    [progress, setProgress] = useState(""),
    [choosingStage, setChoosingStage] = useState(false);
  const submittingRef = useRef(false);
  useEffect(() => {
    let current = true;
    void call("discover")
      .then((s) => {
        if (current) {
          setStages(s);
          if (s.length === 1) setStage(s[0].path);
        }
      })
      .catch((e) => {
        if (current) setError(friendlyError(e));
      });
    const pending = () =>
      call("pending_invitation")
        .then((i) => {
          if (current && i) setInvitation(i);
        })
        .catch((e) => {
          if (current) setError(friendlyError(e));
        });
    void pending();
    const off = desktop ? listen("invitation-opened", pending) : null;
    return () => {
      current = false;
      void off?.then((f) => f());
    };
  }, []);
  useEffect(() => {
    if (!desktop) return;
    const off = listen<string>("project-setup-progress", (e) => {
      if (submittingRef.current) setProgress(e.payload);
    });
    return () => {
      void off.then((f) => f());
    };
  }, []);
  const chooseFolder = () =>
    run("Choosing shared folder", async () => {
      const selected = await call("choose_folder");
      if (selected) setFolder(selected);
    });
  const submit = async () => {
    if (submittingRef.current || busy) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError("");
    setProgress("Preparing project…");
    try {
      await run(
        "Creating project",
        async () => {
          try {
            await call("create_project", {
              name,
              displayName: display,
              stagePath: stage,
              kind,
              folder,
            });
            onDone();
          } catch (e) {
            setError(friendlyError(e));
            throw e;
          }
        },
        { surface: "modal", onError: setError },
      );
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };
  if (invitation)
    return (
      <JoinWizard
        key={`${invitation.endpoint}/${invitation.project}`}
        invitation={invitation}
        storageAccounts={storageAccounts}
        providers={providers}
        busy={busy}
        run={run}
        onClose={onClose}
        onDone={onDone}
        onBeforeJoin={onBeforeJoin}
      />
    );
  const cloudReady =
    kind === "folder" || storageAccounts?.[kind] === "connected";
  return (
    <Modal
      title={
        connectedProvider
          ? `Connected to ${connectedProvider}`
          : submitting
            ? "Creating project"
            : "Create project"
      }
      onClose={connectedProvider ? () => setConnectedProvider(null) : onClose}
      closeDisabled={submitting}
    >
      {connectedProvider ? (
        <ConnectionSuccess onDone={() => setConnectedProvider(null)} />
      ) : submitting ? (
        <div
          className="setup-progress"
          role="status"
          aria-label="Project setup progress"
          aria-live="polite"
          aria-busy="true"
        >
          <LoaderCircle size={28} className="spin" />
          <strong>{progress}</strong>
          <p>Keep this window open until setup finishes.</p>
        </div>
      ) : (
        <>
          {error && (
            <div className="inline-error" role="alert">
              <p>{error}</p>
            </div>
          )}
          <label className="field">
            Your name
            <input
              maxLength={MAX_NAME_UNITS}
              placeholder="Display name"
              value={display}
              onChange={(e) => setDisplay(e.target.value)}
            />
          </label>
          <label className="field">
            Project name
            <input
              maxLength={MAX_NAME_UNITS}
              placeholder="Project name"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          <label className="field">
            Keep the team folder in
            <select
              value={kind}
              onChange={(e) => {
                setKind(e.target.value as typeof kind);
                setFolder(null);
              }}
            >
              <option value="folder">Shared folder (advanced)</option>
              {providers.map((provider) => (
                <option key={provider} value={provider}>
                  {providerName(provider)}
                </option>
              ))}
            </select>
          </label>
          {isCloudProvider(kind) && (
            <StorageConnection
              provider={kind}
              status={storageAccounts?.[kind]}
              run={run}
              busy={busy}
              onConnected={setConnectedProvider}
            />
          )}
          {kind === "folder" && !folder && (
            <div className="folder-picker">
              <button
                className="button"
                disabled={!!busy}
                onClick={chooseFolder}
              >
                <Folder size={16} />
                Choose shared folder
              </button>
              <p className="muted">
                Choose a folder everyone on the team can open, separate from
                game saves.
              </p>
              <p className="muted">
                Works with a Google Drive or OneDrive folder synced to this PC.
                Share it with your teammates there.
              </p>
            </div>
          )}
          {folder && (
            <details className="quiet-details">
              <summary>Shared folder</summary>
              <p>{folder}</p>
              <button
                className="text-button"
                disabled={!!busy}
                onClick={chooseFolder}
              >
                Change folder
              </button>
            </details>
          )}
          {stages.length === 1 && !choosingStage ? (
            <div className="detected-stage">
              <strong>{stages[0].name}</strong>
              <button
                className="text-button"
                onClick={() => setChoosingStage(true)}
              >
                Change stage
              </button>
            </div>
          ) : (
            <label className="field">
              Your Miliastra stage
              <select value={stage} onChange={(e) => setStage(e.target.value)}>
                <option value="">Choose a stage…</option>
                {stages.map((s) => (
                  <option key={s.path} value={s.path}>
                    {s.name}
                    {stages.filter((x) => x.name === s.name).length > 1
                      ? ` (account ${s.account}, stage ${s.level})`
                      : ""}
                  </option>
                ))}
              </select>
            </label>
          )}
          {(!stages.length || choosingStage) && (
            <p className="muted">
              Can’t find your stage? Save it in Miliastra after splitting it,
              then try again.
            </p>
          )}
          {(!stages.length || choosingStage) && (
            <button
              className="text-button"
              disabled={!!busy}
              onClick={() =>
                run("Locating stage", async () => {
                  const path = await call("choose_folder");
                  if (path) {
                    const found = await call("inspect_stage", { path });
                    setStages([found]);
                    setStage(found.path);
                    setChoosingStage(false);
                  }
                })
              }
            >
              Locate a stage manually
            </button>
          )}
          <div className="modal-actions">
            <button className="button" disabled={!!busy} onClick={onClose}>
              Cancel
            </button>
            <button
              className="button primary"
              disabled={
                !!busy ||
                !display.trim() ||
                !stage ||
                !name.trim() ||
                !cloudReady ||
                (kind === "folder" && !folder)
              }
              onClick={submit}
            >
              Create project
              <ChevronRight size={16} />
            </button>
          </div>
        </>
      )}
    </Modal>
  );
}

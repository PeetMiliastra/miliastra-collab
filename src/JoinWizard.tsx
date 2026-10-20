import { MAX_NAME_UNITS } from "./commands.generated";
import { isCloudProvider, providerName } from "./providers";
import { validPrivacyKey } from "./privacyKey";
import { useEffect, useRef, useState } from "react";
import { Check, Folder, LoaderCircle } from "lucide-react";
import { call } from "./bridge";
import { friendlyError } from "./domain";
import { Modal } from "./components";
import { ConnectionSuccess, StorageConnection } from "./StorageConnection";
import type { Invitation, Run, Stage, StorageAccounts } from "./types";
type Info = {
  projectName: string;
  ownerName?: string | null;
  storageKind: string;
  privacyKeyRequired?: boolean;
  mappedFolder: string | null;
};
/** After this long, the joiner is told to ask the owner directly. */
const ASK_OWNER_AFTER = 2 * 60 * 1000;
type Step =
  | "configure"
  | "access"
  | "privacy"
  | "locate"
  | "download"
  | "import"
  | "select"
  | "joining";
export function JoinWizard({
  invitation,
  storageAccounts,
  providers,
  busy,
  run,
  onClose,
  onDone,
  onBeforeJoin,
}: {
  invitation: Invitation;
  storageAccounts?: StorageAccounts;
  providers: string[];
  busy: string;
  run: Run;
  onClose: () => void;
  onDone: () => void;
  onBeforeJoin?: () => Promise<void>;
}) {
  const [info, setInfo] = useState<Info | null>(null),
    [display, setDisplay] = useState("");
  const [folder, setFolder] = useState<string | null>(null),
    [step, setStep] = useState<Step>("configure");
  const [error, setError] = useState(""),
    [retry, setRetry] = useState(0),
    [detailsRetry, setDetailsRetry] = useState(0),
    [loading, setLoading] = useState(true);
  const [hasBase, setHasBase] = useState(false),
    [download, setDownload] = useState("");
  const [stages, setStages] = useState<Stage[]>([]),
    [stage, setStage] = useState("");
  const [connected, setConnected] = useState<string | null>(null),
    [waiting, setWaiting] = useState(false);
  // Shown when the owner's app is offline or sharing takes too long.
  const [askOwner, setAskOwner] = useState(false),
    [accountEmail, setAccountEmail] = useState("");
  const latest = useRef({ run, onDone, onBeforeJoin });
  latest.current = { run, onDone, onBeforeJoin };
  const accessRequest = useRef<{
    id: string;
    at: number;
    grantedAt: number | null;
  } | null>(null);
  const started = useRef(false);
  const unlockAttempt = useRef(false);
  const [privacyKey, setPrivacyKey] = useState("");
  const supported =
    !!info &&
    (info.storageKind === "folder" ||
      (info.storageKind !== "locked" && providers.includes(info.storageKind)));
  useEffect(() => {
    let current = true;
    setLoading(true);
    setError("");
    void call("invitation_details", { invitation })
      .then((v) => {
        if (current) {
          setInfo(v);
          setFolder(v.mappedFolder);
          if (v.privacyKeyRequired) {
            setStep("privacy");
            if (unlockAttempt.current)
              setError(
                "This key could not unlock the project. Ask the owner for the current key.",
              );
          }
        }
      })
      .catch((e) => {
        if (current) setError(friendlyError(e));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [invitation, detailsRetry]);
  // After Join: wait for folder access (the owner's app sharing it, then the provider
  // catching up), with one request at a time.
  useEffect(() => {
    if (step !== "access" || !info) return;
    let current = true,
      timer: ReturnType<typeof setTimeout> | undefined;
    const check = async () => {
      try {
        const result = await call("check_invitation_storage", {
          invitation,
          folder,
          stagePath: null,
        });
        if (!current) return;
        setHasBase(!!result?.baseStage);
        setWaiting(false);
        setAskOwner(false);
        setStep("locate");
      } catch (e) {
        if (!current) return;
        if (info.storageKind === "folder") {
          setError(friendlyError(e));
          return;
        }
        try {
          if (!accessRequest.current) {
            const ready = await call("invitation_privacy_key_ready", {
              invitation,
            });
            if (!current) return;
            if (!ready) {
              setStep("privacy");
              return;
            }
            const id = await call("request_folder_access", {
              invitation,
              displayName: display.trim(),
            });
            if (!current) return;
            accessRequest.current = { id, at: Date.now(), grantedAt: null };
          }
          setWaiting(true);
          const status = await call("folder_access_status", {
            invitation,
            id: accessRequest.current.id,
          });
          if (!current) return;
          setAskOwner(
            status.ownerOnline === false ||
              Date.now() - accessRequest.current.at >= ASK_OWNER_AFTER,
          );
          if (status.status === "failed") {
            accessRequest.current = null;
            if (status.message.startsWith("The Encryption Key"))
              setStep("privacy");
            throw Error(
              status.message ||
                "The owner could not share the team folder. Ask them to check their storage connection, then retry.",
            );
          }
          if (status.status === "granted") {
            accessRequest.current.grantedAt ??= Date.now();
            if (Date.now() - accessRequest.current.grantedAt >= 5 * 60 * 1000)
              throw Error(
                "The team folder still cannot be opened. Check your storage account and retry.",
              );
          }
          timer = setTimeout(check, 5000);
        } catch (requestError) {
          if (current) setError(friendlyError(requestError));
        }
      }
    };
    void check();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [step, info, invitation, folder, retry]);
  useEffect(() => {
    if (!askOwner || accountEmail || !info) return;
    if (!isCloudProvider(info.storageKind)) return;
    let current = true;
    void call("storage_account_email", { provider: info.storageKind })
      .then((email) => {
        if (current) setAccountEmail(email);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [askOwner, accountEmail, info]);
  useEffect(() => {
    if (step !== "locate") return;
    let current = true;
    void (async () => {
      try {
        const path = await call("find_project_stage", {
          invitation,
          folder,
        });
        if (!current) return;
        if (path) {
          setStage(path);
          setStep("joining");
        } else if (hasBase) setStep("download");
        else {
          const all = await call("discover");
          if (!current) return;
          setStages(all);
          if (all.length === 1) setStage(all[0].path);
          setStep("select");
        }
      } catch (e) {
        if (current) setError(friendlyError(e));
      }
    })();
    return () => {
      current = false;
    };
  }, [step, invitation, folder, hasBase, retry]);
  useEffect(() => {
    if (step !== "download") return;
    let current = true;
    void call("download_base_stage", { invitation, folder })
      .then((path) => {
        if (current) {
          setDownload(path);
          setStep("import");
        }
      })
      .catch((e) => {
        if (current) setError(friendlyError(e));
      });
    return () => {
      current = false;
    };
  }, [step, invitation, folder, retry]);
  useEffect(() => {
    if (step !== "import") return;
    let current = true,
      timer: ReturnType<typeof setTimeout> | undefined;
    const look = async () => {
      try {
        const path = await call("find_project_stage", {
          invitation,
          folder,
        });
        if (!current) return;
        if (path) {
          setStage(path);
          setStep("joining");
        } else timer = setTimeout(look, 3000);
      } catch (e) {
        if (current) setError(friendlyError(e));
      }
    };
    void look();
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [step, invitation, folder, retry]);
  useEffect(() => {
    if (step !== "joining") return;
    let current = true;
    void latest.current.run(
      "Joining project",
      async () => {
        await latest.current.onBeforeJoin?.();
        if (!current) return;
        await call("join_project", {
          invitation,
          displayName: display.trim(),
          stagePath: stage,
          folder,
        });
        if (current) latest.current.onDone();
      },
      {
        surface: "modal",
        onError: (message) => {
          if (current) setError(message);
        },
      },
    );
    return () => {
      current = false;
    };
  }, [step, invitation, folder, stage, retry]);
  const retryStep = () => {
    setError("");
    if (step === "access" || step === "privacy") accessRequest.current = null;
    if (step === "privacy") {
      if (info?.privacyKeyRequired) {
        setStep("configure");
        setDetailsRetry((n) => n + 1);
      } else setStep("access");
    }
    if (step === "configure") setDetailsRetry((n) => n + 1);
    else setRetry((n) => n + 1);
  };
  const choosing = step === "configure",
    joining = step === "joining" && !error;
  const cloud = isCloudProvider(info?.storageKind) ? info.storageKind : null;
  const ready =
    !!info &&
    supported &&
    !loading &&
    !!display.trim() &&
    (cloud ? storageAccounts?.[cloud] === "connected" : !!folder);
  const labels = [
    info?.privacyKeyRequired ? "Unlock" : "Connect",
    "Get stage",
    "Import",
    "Join",
  ];
  const index =
    choosing || (step === "privacy" && info?.privacyKeyRequired)
      ? 0
      : step === "privacy" ||
          step === "access" ||
          step === "locate" ||
          step === "download"
        ? 1
        : step === "import" || step === "select"
          ? 2
          : 3;
  const message =
    step === "access"
      ? waiting
        ? `Waiting for ${info?.ownerName ?? "the project owner"} to give you access to the team folder…`
        : "Connecting to the team folder…"
      : step === "locate"
        ? "Looking for your stage…"
        : step === "download"
          ? "Downloading the starting stage…"
          : "Joining project…";
  return (
    <Modal
      title={
        connected
          ? `Connected to ${connected}`
          : `Join ${info?.projectName ?? "project"}`
      }
      onClose={connected ? () => setConnected(null) : onClose}
      closeDisabled={joining || !!busy}
    >
      {connected ? (
        <ConnectionSuccess onDone={() => setConnected(null)} />
      ) : (
        <>
          <ol className="join-steps" aria-label="Join progress">
            {labels.map((label, i) => (
              <li
                key={label}
                aria-current={i === index ? "step" : undefined}
                className={i < index ? "complete" : ""}
              >
                <span>{i < index ? <Check size={13} /> : i + 1}</span>
                {label}
              </li>
            ))}
          </ol>
          {error && (
            <div role="alert" className="inline-error">
              <p>{error}</p>
              <button
                className="button small"
                disabled={!!busy}
                onClick={retryStep}
              >
                Retry
              </button>
              {step === "access" && cloud && supported && (
                <button
                  className="text-button"
                  disabled={!!busy}
                  onClick={() => {
                    started.current = false;
                    accessRequest.current = null;
                    setError("");
                    setStep("configure");
                  }}
                >
                  Change account
                </button>
              )}
            </div>
          )}
          {choosing ? (
            <>
              {loading && <p role="status">Checking invitation…</p>}
              {info && !supported && (
                <p role="alert">
                  This project uses Google Drive, which is unavailable in this
                  version. Ask the owner to use OneDrive or a shared folder.
                </p>
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
              {cloud && supported && (
                <StorageConnection
                  provider={cloud}
                  status={storageAccounts?.[cloud]}
                  run={run}
                  busy={busy}
                  onConnected={setConnected}
                />
              )}
              {info?.storageKind === "folder" && (
                <div className="folder-picker">
                  {folder ? <p>Shared folder selected</p> : null}
                  <button
                    className="button"
                    disabled={!!busy}
                    onClick={() =>
                      run(
                        "Choosing shared folder",
                        async () => {
                          const path = await call("choose_folder");
                          if (path) setFolder(path);
                        },
                        { surface: "modal", onError: setError },
                      )
                    }
                  >
                    <Folder size={16} />
                    {folder ? "Change shared folder" : "Choose shared folder"}
                  </button>
                </div>
              )}
            </>
          ) : step === "privacy" ? (
            <>
              <h3>Paste the project’s Encryption Key</h3>
              <p>
                Copy the Encryption Key from your teammate’s invitation message.
                The key stays on your device and is never sent to the
                collaboration service.
              </p>
              <label className="field">
                Encryption Key
                <input
                  aria-label="Encryption Key"
                  type="password"
                  autoComplete="off"
                  value={privacyKey}
                  onChange={(e) => setPrivacyKey(e.target.value)}
                  placeholder="Paste the project Encryption Key"
                />
              </label>
              <button
                className="button primary"
                disabled={!!busy || !validPrivacyKey(privacyKey)}
                onClick={() =>
                  void run(
                    "Saving Encryption Key",
                    async () => {
                      await call("save_folder_privacy_key", {
                        invitation,
                        key: privacyKey.trim(),
                      });
                      setPrivacyKey("");
                      setError("");
                      accessRequest.current = null;
                      if (info?.privacyKeyRequired) {
                        unlockAttempt.current = true;
                        setInfo(null);
                        setStep("configure");
                        setDetailsRetry((n) => n + 1);
                      } else setStep("access");
                    },
                    { surface: "modal", onError: setError },
                  )
                }
              >
                Continue with Encryption Key
              </button>
              {!info?.privacyKeyRequired && (
                <button
                  className="text-button"
                  onClick={() => {
                    setError("");
                    setStep("access");
                  }}
                >
                  Retry folder connection
                </button>
              )}
            </>
          ) : step === "import" ? (
            <div className="join-import">
              <h3>Import the starting stage</h3>
              <p>
                In Miliastra, import{" "}
                <strong>{download.split(/[\\/]/).pop()}</strong> as a new stage,
                then open and save it.
              </p>
              <p className="muted">The file is shown in Explorer.</p>
              {!error && (
                <p role="status">
                  <LoaderCircle size={16} className="spin" />
                  Waiting for your imported stage…
                </p>
              )}
              <button
                className="text-button"
                disabled={!!busy}
                onClick={() =>
                  void run(
                    "Finding imported stages",
                    async () => {
                      setError("");
                      setStages([]);
                      setStage("");
                      setStep("select");
                      const all = await call("discover");
                      setStages(all);
                      setStage(all.length === 1 ? all[0].path : "");
                    },
                    { surface: "modal", onError: setError },
                  )
                }
              >
                Choose imported stage
              </button>
            </div>
          ) : step === "select" ? (
            <>
              <h3>Choose your stage</h3>
              <p>
                {hasBase
                  ? "Select the stage you imported and saved in Miliastra."
                  : "This project has no starting stage to download. Select your copy in Miliastra."}
              </p>
              <label className="field">
                Your Miliastra stage
                <select
                  value={stage}
                  onChange={(e) => setStage(e.target.value)}
                >
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
              <button
                className="text-button"
                disabled={!!busy}
                onClick={() =>
                  run(
                    "Locating stage",
                    async () => {
                      const path = await call("choose_folder");
                      if (path) {
                        const found = await call("inspect_stage", {
                          path,
                        });
                        setStages((old) => [
                          ...old.filter((s) => s.path !== found.path),
                          found,
                        ]);
                        setStage(found.path);
                      }
                    },
                    { surface: "modal", onError: setError },
                  )
                }
              >
                Locate a stage manually
              </button>
            </>
          ) : (
            !error && (
              <div
                className="setup-progress"
                role="status"
                aria-label="Join setup progress"
                aria-live="polite"
              >
                <LoaderCircle size={28} className="spin" />
                <strong>{message}</strong>
                {step === "access" && waiting && !askOwner && (
                  <p>
                    This happens automatically as soon as{" "}
                    {info?.ownerName ?? "the owner"} has miliastra/collab open.
                    You can leave this window open, or close it and open the
                    invitation again later.
                  </p>
                )}
                {step === "access" && waiting && askOwner && (
                  <div className="ask-owner" role="note">
                    <strong>
                      Ask {info?.ownerName ?? "the project owner"} to share the
                      project folder with you
                    </strong>
                    <p>
                      {info?.ownerName ?? "The owner"} can open the team folder
                      in miliastra/collab under Project settings, or in{" "}
                      {providerName(info?.storageKind)}, and share it with{" "}
                      {accountEmail ? (
                        <strong>{accountEmail}</strong>
                      ) : (
                        "your account"
                      )}{" "}
                      with edit access. This window continues by itself as soon
                      as you have access.
                    </p>
                    {accountEmail && (
                      <button
                        className="button"
                        onClick={() =>
                          void navigator.clipboard
                            ?.writeText(accountEmail)
                            .catch(() => {})
                        }
                      >
                        Copy my email
                      </button>
                    )}
                  </div>
                )}
              </div>
            )
          )}
          <div className="modal-actions">
            <button
              className="button"
              disabled={joining || !!busy}
              onClick={onClose}
            >
              Cancel
            </button>
            {choosing && (
              <button
                className="button primary"
                disabled={!ready || !!busy}
                onClick={() => {
                  if (started.current) return;
                  started.current = true;
                  setError("");
                  setStep("access");
                }}
              >
                Join project
              </button>
            )}
            {step === "select" && (
              <button
                className="button primary"
                disabled={!stage || !!busy}
                onClick={() => {
                  setError("");
                  setStep("joining");
                }}
              >
                Use this stage
              </button>
            )}
          </div>
        </>
      )}
    </Modal>
  );
}

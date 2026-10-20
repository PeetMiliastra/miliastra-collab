import { useState } from "react";
import { AlertTriangle, WifiOff } from "lucide-react";
import { call } from "./bridge";
import { providerName } from "./providers";
import { Modal } from "./components";
import { ConnectionSuccess } from "./StorageConnection";
import type { Run, Snapshot } from "./types";

export function ProjectConnectionNotice({
  state,
  busy,
  run,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
}) {
  if (
    !state.project ||
    state.removed ||
    (state.online && !state.protocolMismatch)
  )
    return null;
  return (
    <div className="banner" role="status" aria-label="Project connection">
      {state.protocolMismatch ? (
        <AlertTriangle size={18} />
      ) : (
        <WifiOff size={18} />
      )}
      <div>
        <strong>
          {state.protocolMismatch
            ? "Update required"
            : "Project connection unavailable"}
        </strong>
        <p>
          {state.protocolMismatch
            ? "This app and the project service use incompatible versions. Update miliastra/collab. If your app is already up to date, ask the project owner to update the service."
            : "Your app cannot reach the project service. Changes stay local; reservations and sharing resume when it reconnects."}
        </p>
      </div>
      <button
        className="button small"
        disabled={!!busy}
        onClick={() => run("Reconnecting to project", () => call("refresh"))}
      >
        Retry connection
      </button>
    </div>
  );
}

export function storageRecovery(state: Snapshot) {
  if (!state.project) return null;
  const provider = state.project.storage.kind,
    account = state.storageAccounts?.[provider];
  const issue =
    account === "notConnected" || account === "reconnect"
      ? "reconnect"
      : state.storageIssue;
  const owner =
    state.project.members.find((m) => m.owner)?.name ?? "the project owner";
  const ownsProject = state.project.members.some(
    (m) => m.owner && m.id === state.member,
  );
  switch (issue) {
    case "reconnect":
      return {
        title: `Reconnect ${providerName(provider)}`,
        message:
          "Sign in again to download and share updates. Your saved work stays on this PC.",
        action: "reconnect",
        button: `Reconnect ${providerName(provider)}`,
      };
    case "access":
      return {
        title: "Team folder access is blocked",
        message:
          provider === "folder"
            ? "Check that this PC can read and write the shared folder, then retry."
            : ownsProject
              ? "Check that your connected account has editing access to the team folder, then retry."
              : `Ask ${owner} to give your connected account editing access to the team folder, then retry.`,
        action: provider === "folder" ? "folder" : "owner",
        button: provider === "folder" ? "Open team folder" : "View team",
      };
    case "missing":
      return {
        title: "An update file is not available",
        message: `The team folder or a published file could not be found. ${ownsProject ? "Check the team folder" : `Ask ${owner} to check the team folder`}. Downloads will retry automatically.`,
        action: "folder",
        button: "Open team folder",
      };
    case "network":
      return {
        title: "Cannot reach the team folder",
        message:
          "Check your internet or shared-folder connection. Downloads will retry automatically; your saved work is safe.",
        action: "retry",
        button: "Retry download",
      };
    case "busy":
      return {
        title: "The storage provider is busy",
        message:
          "The storage provider is temporarily unavailable or has asked us to wait. Downloads will retry automatically.",
        action: "retry",
        button: "Retry download",
      };
    case "validation":
      return {
        title: "An update failed verification",
        message:
          "The downloaded file does not match the published update. It has not been applied. Retry; if this continues, ask the author to check the files in the team folder.",
        action: "retry",
        button: "Retry download",
      };
    case "unknown":
      return {
        title: "An update could not be downloaded",
        message:
          "Retry the download. If it keeps failing, check your storage account and the team folder. Your saved work is safe.",
        action: provider === "folder" ? "folder" : "settings",
        button: provider === "folder" ? "Open team folder" : "Storage accounts",
      };
    default:
      return state.metadataPending
        ? {
            title: "Waiting for update details",
            message:
              "Details are downloading from the team folder. Your cached history and saved work are preserved.",
            action: "retry",
            button: "Check for changes",
          }
        : null;
  }
}
export function RecoveryNotice({
  state,
  busy,
  run,
  onAccounts,
  onTeam,
}: {
  state: Snapshot;
  busy: string;
  run: Run;
  onAccounts: () => void;
  onTeam: () => void;
}) {
  const [connected, setConnected] = useState(false);
  const recovery = storageRecovery(state);
  const retry = () => run("Retrying downloads", () => call("refresh"));
  return (
    <>
      {connected && (
        <Modal
          title={`Connected to ${providerName(state.project?.storage.kind ?? "")}`}
          onClose={() => setConnected(false)}
        >
          <ConnectionSuccess onDone={() => setConnected(false)} />
        </Modal>
      )}
      {recovery && (
        <div className="banner" role="status">
          <div>
            <strong>{recovery.title}</strong>
            <p>{recovery.message}</p>
          </div>
          <div className="recovery-actions">
            <button
              className="button small"
              disabled={!!busy}
              onClick={() => {
                if (recovery.action === "reconnect")
                  void run("Opening secure sign-in", async () => {
                    await call("sign_in", {
                      provider: state.project!.storage.kind,
                    });
                    setConnected(true);
                    await call("refresh");
                  });
                else if (recovery.action === "owner") onTeam();
                else if (recovery.action === "folder")
                  void run("Opening team folder", () => call("open_storage"));
                else if (recovery.action === "settings") onAccounts();
                else void retry();
              }}
            >
              {recovery.button}
            </button>
            {recovery.action !== "retry" && (
              <button
                className="text-button"
                disabled={!!busy}
                onClick={() => void retry()}
              >
                Retry download
              </button>
            )}
          </div>
        </div>
      )}
    </>
  );
}

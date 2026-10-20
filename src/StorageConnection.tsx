import { CheckCircle2, Cloud } from "lucide-react";
import { providerName } from "./providers";
import { call, desktop } from "./bridge";
import type { Run, StorageAccounts } from "./types";
export function StorageConnection({
  provider,
  status,
  run,
  busy,
  onConnected,
}: {
  provider: string;
  status?: StorageAccounts[string];
  onConnected: (name: string) => void;
  run: Run;
  busy: string;
}) {
  const name = providerName(provider);
  const connected = status === "connected";
  const label =
    status === "connected"
      ? "Connected"
      : status === "notConnected"
        ? "Not connected"
        : status === "reconnect"
          ? "Reconnect needed"
          : status === "unavailable"
            ? "Unable to check connection"
            : "Checking connection…";
  return (
    <div className="provider-row" role="group" aria-label={`${name} account`}>
      <Cloud size={21} />
      <div className="provider-info">
        <strong>{name}</strong>
        <span
          className={`connection-state ${connected ? "connected" : ""}`}
          role="status"
        >
          {connected && <CheckCircle2 size={14} aria-hidden="true" />}
          {label}
        </span>
      </div>
      {!connected && (
        <button
          className="button small"
          disabled={!!busy || !desktop}
          aria-label={`${status === "reconnect" ? "Reconnect" : "Connect"} ${name}`}
          onClick={() =>
            run("Opening secure sign-in", async () => {
              await call("sign_in", { provider });
              onConnected(name);
            })
          }
        >
          {status === "reconnect" ? "Reconnect" : "Connect"}
        </button>
      )}
      {(connected || status === "reconnect") && (
        <button
          className="text-button"
          disabled={!!busy || !desktop}
          aria-label={`Disconnect ${name}`}
          onClick={() =>
            run("Disconnecting storage", () => call("disconnect", { provider }))
          }
        >
          Disconnect
        </button>
      )}
    </div>
  );
}

export function ConnectionSuccess({ onDone }: { onDone: () => void }) {
  return (
    <>
      <p className="connection-confirmation">Your account is ready to use.</p>
      <div className="modal-actions">
        <button className="button primary" autoFocus onClick={onDone}>
          Done
        </button>
      </div>
    </>
  );
}

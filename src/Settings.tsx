import { AppearanceSettings } from "./AppearanceSettings";
import { defaultCloudProviders } from "./providers";
import { useState } from "react";
import { ExternalLink } from "lucide-react";
import { ApplicationUpdates } from "./ApplicationUpdates";
import { EncryptionKeyRestore } from "./EncryptionKeyRestore";
import { TeamPanel } from "./TeamPanel";
import { StorageConnection, ConnectionSuccess } from "./StorageConnection";
import { Confirm, Modal } from "./components";
import { call, desktop } from "./bridge";
import type { Snapshot, Run } from "./types";
type Props = { state: Snapshot; busy: string; run: Run };
export function AppSettings({ state, busy, run }: Props) {
  const [connected, setConnected] = useState<string | null>(null);
  return (
    <section className="settings-page" aria-label="App settings">
      {connected && (
        <Modal
          title={"Connected to " + connected}
          onClose={() => setConnected(null)}
        >
          <ConnectionSuccess onDone={() => setConnected(null)} />
        </Modal>
      )}
      <div className="page-heading">
        <h1>App settings</h1>
      </div>
      <AppearanceSettings />
      <div className="settings-card">
        <h2>Startup</h2>
        <label className="toggle-row">
          <strong>Start with Windows</strong>
          <input
            type="checkbox"
            checked={state.settings.autostart}
            disabled={!!busy || !desktop || !!state.testProfile}
            onChange={(e) =>
              run("Saving preference", () =>
                call("save_settings", {
                  settings: { ...state.settings, autostart: e.target.checked },
                }),
              )
            }
          />
        </label>
      </div>
      <div className="settings-card">
        <h2>Incoming project updates</h2>
        <label className="toggle-row">
          <span>
            <strong>
              Apply teammates’ updates automatically when the stage isn’t open
              in Miliastra
            </strong>
          </span>
          <input
            type="checkbox"
            checked={state.settings.autoApply ?? true}
            disabled={!!busy || !desktop}
            onChange={(e) =>
              run("Saving preference", () =>
                call("save_settings", {
                  settings: { ...state.settings, autoApply: e.target.checked },
                }),
              )
            }
          />
        </label>
      </div>
      <div className="settings-card">
        <h2>Storage accounts</h2>
        {(state.cloudProviders ?? defaultCloudProviders).map((provider) => (
          <StorageConnection
            key={provider}
            provider={provider}
            status={state.storageAccounts?.[provider]}
            busy={busy}
            run={run}
            onConnected={setConnected}
          />
        ))}
      </div>
      <ApplicationUpdates busy={busy} run={run} />
    </section>
  );
}
export function ProjectSettings({ state, busy, run }: Props) {
  const [leaving, setLeaving] = useState(false);
  if (!state.project) return null;
  return (
    <section className="settings-page" aria-label="Project settings">
      <div className="page-heading">
        <h1>Project settings</h1>
      </div>
      <div className="settings-card">
        <h2>Team folder</h2>
        <button
          className="button team-folder-button"
          disabled={!!busy}
          onClick={() => run("Opening team folder", () => call("open_storage"))}
        >
          <ExternalLink size={15} />
          Open team folder
        </button>
        <div className="toggle-row">
          <span>
            <strong>Starting stage for new teammates</strong>
            <small className="toggle-help">
              New teammates import this stage in Miliastra before joining.
            </small>
          </span>
          <button
            className="button small"
            disabled={!!busy || !desktop}
            onClick={() =>
              run("Updating starting stage", () => call("publish_base_stage"))
            }
          >
            Update starting stage
          </button>
        </div>
      </div>
      <TeamPanel state={state} busy={busy} run={run} />
      <details className="settings-card key-recovery">
        <summary>Troubleshooting</summary>
        <EncryptionKeyRestore />
      </details>
      <div className="settings-actions">
        <button
          className="text-button danger"
          disabled={!!busy}
          onClick={() => setLeaving(true)}
        >
          Leave this project
        </button>
      </div>
      {leaving && (
        <Confirm
          title={`Leave ${state.project.name}?`}
          confirmLabel="Leave project"
          danger
          busy={!!busy}
          onClose={() => setLeaving(false)}
          onConfirm={() => {
            setLeaving(false);
            void run("Leaving project", () => call("forget_project"));
          }}
        >
          <p>
            It’s removed from your projects. Your saved files, drafts and the
            team’s history are kept.
          </p>
        </Confirm>
      )}
    </section>
  );
}

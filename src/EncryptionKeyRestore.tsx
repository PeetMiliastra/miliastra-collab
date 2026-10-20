import { useState } from "react";
import { validPrivacyKey } from "./privacyKey";
import { call } from "./bridge";
import { friendlyError } from "./domain";
export function EncryptionKeyRestore() {
  const [key, setKey] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  return (
    <div className="encryption-key-restore">
      <p>
        If this device is missing the project’s Encryption Key, or the team has
        replaced it, restore it from a teammate’s invitation message.
      </p>
      <label className="field">
        Encryption Key
        <input
          aria-label="Encryption Key"
          type="password"
          autoComplete="off"
          maxLength={40000}
          placeholder="Paste your team’s Encryption Key"
          value={key}
          disabled={busy}
          onChange={(e) => setKey(e.target.value)}
        />
      </label>
      <button
        className="button small"
        disabled={busy || !validPrivacyKey(key)}
        onClick={async () => {
          setBusy(true);
          setError("");
          setNotice("");
          try {
            await call("save_folder_privacy_key", {
              invitation: null,
              key: key.trim(),
            });
            setKey("");
            setNotice("Encryption Key restored on this device.");
          } catch (e) {
            setError(friendlyError(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Restoring…" : "Restore"}
      </button>
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
    </div>
  );
}

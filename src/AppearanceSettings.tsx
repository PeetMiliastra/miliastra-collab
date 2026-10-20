import { useState, useSyncExternalStore } from "react";
import {
  getAppearance,
  setAppearance,
  subscribeAppearance,
  type Appearance,
} from "./appearance";

export function AppearanceSettings() {
  const preference = useSyncExternalStore(subscribeAppearance, getAppearance);
  const [error, setError] = useState("");
  return (
    <div className="settings-card">
      <h2>Appearance</h2>
      <label className="appearance-field">
        <strong>Color mode</strong>
        <select
          value={preference}
          onChange={(event) => {
            try {
              setAppearance(event.target.value as Appearance);
              setError("");
            } catch {
              setError("Could not save appearance on this device. Try again.");
            }
          }}
        >
          <option value="system">Use Windows setting</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </select>
      </label>
      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}

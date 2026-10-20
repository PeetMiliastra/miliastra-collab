export type Appearance = "system" | "light" | "dark";
export const APPEARANCE_KEY = "miliastra-collab-appearance";
const SYSTEM_DARK = "(prefers-color-scheme: dark)";
const listeners = new Set<() => void>();
let preference: Appearance = "system";
let media: MediaQueryList | null = null;
let initialized = false;

function validPreference(value: string | null): Appearance {
  return value === "light" || value === "dark" ? value : "system";
}
function readPreference(): Appearance {
  try {
    return validPreference(localStorage.getItem(APPEARANCE_KEY));
  } catch {
    return "system";
  }
}
function apply() {
  const theme =
    preference === "system" ? (media?.matches ? "dark" : "light") : preference;
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  listeners.forEach((listener) => listener());
}
export function initializeAppearance(): () => void {
  if (initialized) return () => {};
  initialized = true;
  preference = readPreference();
  media =
    typeof window.matchMedia === "function"
      ? window.matchMedia(SYSTEM_DARK)
      : null;
  const onSystemChange = () => {
    if (preference === "system") apply();
  };
  const onStorage = (event: StorageEvent) => {
    if (
      (event.storageArea === null || event.storageArea === localStorage) &&
      (event.key === APPEARANCE_KEY || event.key === null)
    ) {
      preference = readPreference();
      apply();
    }
  };
  media?.addEventListener("change", onSystemChange);
  window.addEventListener("storage", onStorage);
  apply();
  return () => {
    media?.removeEventListener("change", onSystemChange);
    window.removeEventListener("storage", onStorage);
    initialized = false;
    media = null;
  };
}
export function getAppearance(): Appearance {
  return preference;
}
export function setAppearance(value: Appearance) {
  // Persist before changing the UI so a storage error cannot claim it was saved.
  localStorage.setItem(APPEARANCE_KEY, value);
  preference = value;
  apply();
}
export function subscribeAppearance(listener: () => void): () => void {
  if (!initialized) initializeAppearance();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

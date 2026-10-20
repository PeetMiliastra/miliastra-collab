// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from "vitest";
let appearance: typeof import("./appearance");
let stop = () => {};
let dark = false;
let media: MediaQueryList;
beforeEach(async () => {
  vi.resetModules();
  localStorage.clear();
  dark = false;
  media = new EventTarget() as MediaQueryList;
  Object.defineProperty(media, "matches", { get: () => dark });
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => media),
  );
  delete document.documentElement.dataset.theme;
  document.documentElement.style.removeProperty("color-scheme");
  appearance = await import("./appearance");
});
afterEach(() => {
  stop();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  document.documentElement.style.removeProperty("color-scheme");
});
function system(value: boolean) {
  dark = value;
  media.dispatchEvent(new Event("change"));
}
it("starts with the Windows appearance when there is no saved choice", () => {
  dark = true;
  stop = appearance.initializeAppearance();
  expect(appearance.getAppearance()).toBe("system");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(document.documentElement.style.colorScheme).toBe("dark");
});
it("restores the local choice before rendering even when Windows uses another mode", () => {
  localStorage.setItem(appearance.APPEARANCE_KEY, "dark");
  stop = appearance.initializeAppearance();
  expect(appearance.getAppearance()).toBe("dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(matchMedia).toHaveBeenCalledWith("(prefers-color-scheme: dark)");
});
it("stores explicit choices and only follows Windows when System is selected", () => {
  stop = appearance.initializeAppearance();
  const notify = vi.fn();
  const unsubscribe = appearance.subscribeAppearance(notify);
  appearance.setAppearance("light");
  expect(localStorage.getItem(appearance.APPEARANCE_KEY)).toBe("light");
  notify.mockClear();
  system(true);
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(notify).not.toHaveBeenCalled();
  appearance.setAppearance("system");
  expect(document.documentElement.dataset.theme).toBe("dark");
  system(false);
  expect(document.documentElement.dataset.theme).toBe("light");
  unsubscribe();
  appearance.setAppearance("dark");
  expect(notify).toHaveBeenCalledTimes(2);
});
it("syncs a choice from another app window and handles clearing local storage", () => {
  stop = appearance.initializeAppearance();
  localStorage.setItem(appearance.APPEARANCE_KEY, "dark");
  window.dispatchEvent(new StorageEvent("storage", { key: "unrelated" }));
  expect(document.documentElement.dataset.theme).toBe("light");
  window.dispatchEvent(
    new StorageEvent("storage", { key: appearance.APPEARANCE_KEY }),
  );
  expect(appearance.getAppearance()).toBe("dark");
  localStorage.clear();
  window.dispatchEvent(new StorageEvent("storage", { key: null }));
  expect(appearance.getAppearance()).toBe("system");
});
it("uses System for invalid or unreadable local preferences", () => {
  localStorage.setItem(appearance.APPEARANCE_KEY, "unknown");
  stop = appearance.initializeAppearance();
  expect(appearance.getAppearance()).toBe("system");
  stop();
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw Error("Storage blocked");
  });
  dark = true;
  stop = appearance.initializeAppearance();
  expect(document.documentElement.dataset.theme).toBe("dark");
});
it("does not change the saved appearance when storage refuses a write", () => {
  stop = appearance.initializeAppearance();
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw Error("Storage full");
  });
  expect(() => appearance.setAppearance("dark")).toThrow("Storage full");
  expect(appearance.getAppearance()).toBe("system");
  expect(document.documentElement.dataset.theme).toBe("light");
});
it("removes event handlers when initialization is disposed", () => {
  stop = appearance.initializeAppearance();
  stop();
  system(true);
  localStorage.setItem(appearance.APPEARANCE_KEY, "dark");
  window.dispatchEvent(
    new StorageEvent("storage", { key: appearance.APPEARANCE_KEY }),
  );
  expect(document.documentElement.dataset.theme).toBe("light");
});

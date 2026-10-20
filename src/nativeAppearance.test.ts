// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { waitFor } from "@testing-library/react";
const store = vi.hoisted(() => ({
  preference: "system" as "system" | "light" | "dark",
  listeners: new Set<() => void>(),
  setTheme: vi.fn(async (_value: string | null) => {}),
}));
vi.mock("./appearance", () => ({
  getAppearance: () => store.preference,
  subscribeAppearance: (listener: () => void) => {
    store.listeners.add(listener);
    return () => store.listeners.delete(listener);
  },
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: store.setTheme }),
}));
import { connectWindowAppearance } from "./nativeAppearance";
let stop = () => {};
beforeEach(() => {
  store.preference = "system";
  store.setTheme.mockReset().mockResolvedValue(undefined);
});
afterEach(() => {
  stop();
  store.listeners.clear();
  vi.restoreAllMocks();
});
function change(value: "system" | "light" | "dark") {
  store.preference = value;
  store.listeners.forEach((listener) => listener());
}
it("uses the native system theme at startup and forwards explicit preferences", async () => {
  stop = connectWindowAppearance();
  await waitFor(() => expect(store.setTheme).toHaveBeenCalledWith(null));
  change("dark");
  await waitFor(() => expect(store.setTheme).toHaveBeenLastCalledWith("dark"));
  change("light");
  await waitFor(() => expect(store.setTheme).toHaveBeenLastCalledWith("light"));
  change("system");
  await waitFor(() => expect(store.setTheme).toHaveBeenLastCalledWith(null));
});
it("serializes native changes and applies the latest preference after a slow call", async () => {
  let complete!: () => void;
  store.setTheme.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        complete = resolve;
      }),
  );
  stop = connectWindowAppearance();
  await waitFor(() => expect(store.setTheme).toHaveBeenCalledTimes(1));
  change("dark");
  change("light");
  expect(store.setTheme).toHaveBeenCalledTimes(1);
  complete();
  await waitFor(() => expect(store.setTheme).toHaveBeenCalledTimes(3));
  expect(store.setTheme).toHaveBeenLastCalledWith("light");
  stop();
  change("dark");
  expect(store.setTheme).toHaveBeenCalledTimes(3);
});
it("recovers from a failed native call so later preferences can still apply", async () => {
  vi.spyOn(console, "warn").mockImplementation(() => {});
  store.setTheme.mockRejectedValueOnce(Error("Window unavailable"));
  stop = connectWindowAppearance();
  await waitFor(() => expect(console.warn).toHaveBeenCalledOnce());
  change("dark");
  await waitFor(() => expect(store.setTheme).toHaveBeenLastCalledWith("dark"));
});

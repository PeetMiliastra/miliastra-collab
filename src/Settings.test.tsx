// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from "@testing-library/react";
import { afterEach, it, expect, vi } from "vitest";
const ipc = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ desktop: true, call: ipc }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
import { AppSettings, ProjectSettings } from "./Settings";
import { empty } from "./App";
import { APPEARANCE_KEY, setAppearance } from "./appearance";
import type { Run, Snapshot } from "./types";
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  setAppearance("system");
  localStorage.removeItem(APPEARANCE_KEY);
  vi.clearAllMocks();
});
const run: Run = async (_, f) => {
  await f();
};
const state: Snapshot = {
  ...empty,
  online: true,
  member: "me",
  project: {
    id: "p",
    name: "Fisch",
    storage: { kind: "folder", location: "" },
    members: [{ id: "me", name: "Owner", owner: true }],
    heads: {},
    locks: [],
    sequence: 1,
  },
};
it("keeps accounts, startup and app updates in App settings", () => {
  render(<AppSettings state={state} busy="" run={run} />);
  expect(screen.getByRole("heading", { name: "App settings" })).toBeVisible();
  expect(
    screen.getByRole("checkbox", { name: "Start with Windows" }),
  ).toBeEnabled();
  expect(
    screen.getByRole("button", { name: "Connect OneDrive" }),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Connect Google Drive" }),
  ).toBeNull();
  expect(
    screen.getByRole("button", { name: "Check for an app update" }),
  ).toBeVisible();
  expect(screen.queryByText("Starting stage for new teammates")).toBeNull();
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "Apply teammates’ updates automatically when the stage isn’t open in Miliastra",
    }),
  );
  expect(ipc).toHaveBeenCalledWith("save_settings", {
    settings: { ...state.settings, autoApply: false },
  });
  expect(screen.queryByRole("heading", { name: "Team" })).toBeNull();
  expect(screen.queryByLabelText("Encryption Key")).toBeNull();
  fireEvent.click(screen.getByRole("checkbox", { name: "Start with Windows" }));
  expect(ipc).toHaveBeenCalledWith("save_settings", {
    settings: { ...state.settings, autostart: true },
  });
});
it("keeps team, starting stage, Encryption Key restoration and leaving in Project settings", async () => {
  render(<ProjectSettings state={state} busy="" run={run} />);
  expect(
    screen.getByRole("heading", { name: "Project settings" }),
  ).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Open team folder" }),
  ).toBeVisible();
  expect(screen.getByRole("button", { name: "Restore" })).not.toBeVisible();
  const troubleshooting = screen
    .getByText("Troubleshooting")
    .closest("details")!;
  expect(troubleshooting).not.toHaveAttribute("open");
  troubleshooting.open = true;
  fireEvent(troubleshooting, new Event("toggle"));
  expect(screen.getByRole("button", { name: "Restore" })).toBeDisabled();
  expect(
    screen.getByText(/missing the project’s Encryption Key/),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: /Show.*key|Replace.*key/i }),
  ).toBeNull();
  expect(screen.queryByRole("button", { name: "Connect OneDrive" })).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Check for an app update" }),
  ).toBeNull();
  expect(screen.getByText(/New teammates import this stage/)).toBeVisible();
  expect(screen.queryByText(/from.*providers/)).toBeNull();
  fireEvent.click(
    screen.getByRole("button", { name: "Update starting stage" }),
  );
  await waitFor(() => expect(ipc).toHaveBeenCalledWith("publish_base_stage"));
  expect(
    screen.getByRole("button", { name: "Leave this project" }),
  ).toBeVisible();
});

it("changes appearance locally, restores it in settings, and stays available offline", () => {
  const view = render(
    <AppSettings state={{ ...state, online: false }} busy="" run={run} />,
  );
  expect(screen.getByRole("combobox", { name: "Color mode" })).toHaveValue(
    "system",
  );
  fireEvent.change(screen.getByRole("combobox", { name: "Color mode" }), {
    target: { value: "dark" },
  });
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem(APPEARANCE_KEY)).toBe("dark");
  expect(ipc.mock.calls.some(([command]) => command === "save_settings")).toBe(
    false,
  );
  view.unmount();
  render(<AppSettings state={state} busy="" run={run} />);
  expect(screen.getByRole("combobox", { name: "Color mode" })).toHaveValue(
    "dark",
  );
});
it("reports a failed appearance save without falsely selecting the unsaved mode", () => {
  render(<AppSettings state={state} busy="" run={run} />);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw Error("Storage full");
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Color mode" }), {
    target: { value: "dark" },
  });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Could not save appearance on this device",
  );
  expect(screen.getByRole("combobox", { name: "Color mode" })).toHaveValue(
    "system",
  );
});

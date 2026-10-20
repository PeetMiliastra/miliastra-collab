// @vitest-environment jsdom
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
  act,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const ipc = vi.hoisted(() => ({
  call: vi.fn(),
  listeners: new Map<string, (event?: unknown) => unknown>(),
}));
vi.mock("./bridge", () => ({ desktop: true, call: ipc.call }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, cb: (event?: unknown) => unknown) => {
    ipc.listeners.set(event, cb);
    return () => {};
  }),
}));
import App, { empty } from "./App";
import "@testing-library/jest-dom/vitest";
import type { Snapshot } from "./types";
let state: Snapshot;
beforeEach(() => {
  state = {
    ...empty,
    online: true,
    member: "me",
    draft: { summary: "Recovered draft", details: "Keep this private" },
    project: {
      id: "first",
      name: "Test world",
      storage: { kind: "folder", location: "" },
      members: [{ id: "me", name: "Robin", owner: true }],
      locks: [],
      heads: {},
      sequence: 1,
    },
    sections: [
      {
        id: "5",
        name: "Boats",
        changed: true,
        conflict: false,
        lock: null,
        downloaded: true,
        installed: true,
      },
    ],
  };
  ipc.call.mockReset();
  ipc.listeners.clear();
  ipc.call.mockImplementation(async (command: string) =>
    command === "snapshot"
      ? structuredClone(state)
      : command === "connections"
        ? []
        : null,
  );
});
afterEach(cleanup);
it("hydrates recovered drafts without writing stale form state, then saves user edits to the correct project", async () => {
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  await new Promise((resolve) => setTimeout(resolve, 350));
  expect(
    ipc.call.mock.calls.filter(([cmd]) => cmd === "save_draft"),
  ).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: /Share update/ }));
  const field = screen.getByPlaceholderText(
    "Fixed passengers falling through the boat deck",
  );
  expect((field as HTMLTextAreaElement).value).toBe("Recovered draft");
  fireEvent.change(field, { target: { value: "Edited draft" } });
  await waitFor(() =>
    expect(ipc.call).toHaveBeenCalledWith("save_draft", {
      projectId: "first",
      draft: { summary: "Edited draft", details: "Keep this private" },
    }),
  );
});

it("shows and copies the full service invitation link", async () => {
  const link =
    "https://collabs.miliastra.dev/invite/11111111-1111-4111-8111-111111111111#22222222-2222-4222-8222-222222222222";
  ipc.call.mockImplementation(async (command: string) =>
    command === "snapshot"
      ? structuredClone(state)
      : command === "connections"
        ? []
        : command === "invite"
          ? link
          : command === "folder_privacy_key"
            ? "mcollab-key-v2:test"
            : null,
  );
  const writeText = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "Invite creators" }));
  const field = await screen.findByRole("textbox", {
    name: "Invitation message",
  });
  const message = (field as HTMLTextAreaElement).value;
  expect(message).toContain(link);
  expect(message).toContain("Test world");
  expect(message).toContain("Encryption Key: mcollab-key-v2:test");
  expect(message).toContain("https://miliastra.dev");
  expect(link).not.toContain("mcollab-key");
  fireEvent.click(screen.getByRole("button", { name: "Copy invitation" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(message));
  fireEvent.click(screen.getByRole("button", { name: "Copy link only" }));
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(link));
  expect(
    screen.queryByRole("button", { name: "Copy Encryption Key" }),
  ).toBeNull();
  expect(screen.queryByText("Project encryption")).toBeNull();
  expect(ipc.call).toHaveBeenCalledWith("folder_privacy_key");
  expect(ipc.call).toHaveBeenCalledWith("invite");
});

it("offers no demo project in the desktop app", async () => {
  state = { ...empty };
  render(<App />);
  await screen.findByRole("heading", { name: "Your projects" });
  expect(
    screen.queryByRole("button", { name: "Explore a demo project" }),
  ).toBeNull();
  expect(ipc.call).not.toHaveBeenCalledWith("begin_practice");
});

function projectNavigation() {
  const savedProject = state.project!;
  let savedDraft = structuredClone(state.draft);
  ipc.call.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      if (command === "snapshot") return structuredClone(state);
      if (command === "connections")
        return [{ id: savedProject.id, name: savedProject.name }];
      if (command === "save_draft")
        savedDraft = structuredClone(args!.draft as Snapshot["draft"]);
      if (command === "leave_project") state = { ...empty };
      if (command === "reopen_project")
        state = {
          ...state,
          project: savedProject,
          draft: savedDraft,
          sections: [
            {
              id: "5",
              name: "Boats",
              changed: true,
              conflict: false,
              lock: null,
              downloaded: true,
              installed: true,
            },
          ],
        };
      return null;
    },
  );
}

it("returns to the project list, flushes a just-edited draft, and reopens the saved workspace", async () => {
  projectNavigation();
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: /Share update/ }));
  fireEvent.change(
    screen.getByPlaceholderText(
      "Fixed passengers falling through the boat deck",
    ),
    { target: { value: "Latest unsent draft" } },
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Close" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  fireEvent.click(screen.getByRole("button", { name: "All projects" }));
  await screen.findByRole("heading", { name: "Your projects" });
  const commands = ipc.call.mock.calls.map(([cmd]) => cmd);
  expect(commands.indexOf("save_draft")).toBeLessThan(
    commands.indexOf("leave_project"),
  );
  expect(ipc.call).toHaveBeenCalledWith("save_draft", {
    projectId: "first",
    draft: { summary: "Latest unsent draft", details: "Keep this private" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Test world" }));
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: /Share update/ }));
  expect(
    (
      screen.getByPlaceholderText(
        "Fixed passengers falling through the boat deck",
      ) as HTMLTextAreaElement
    ).value,
  ).toBe("Latest unsent draft");
});

it("offers overview navigation from Settings", async () => {
  projectNavigation();
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "App settings" }));
  fireEvent.click(screen.getByRole("button", { name: "All projects" }));
  await screen.findByRole("heading", { name: "Your projects" });
  expect(ipc.call).toHaveBeenCalledWith("leave_project");
  expect(ipc.call).not.toHaveBeenCalledWith("leave_practice");
  expect(screen.queryByRole("button", { name: "All projects" })).toBeNull();
});

it("keeps the workspace and draft open if saving before navigation fails", async () => {
  projectNavigation();
  const normal = ipc.call.getMockImplementation()!;
  ipc.call.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      if (command === "save_draft") throw Error("The disk is full");
      return normal(command, args);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "All projects" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "The disk is full",
  );
  expect(ipc.call).not.toHaveBeenCalledWith("leave_project");
  expect(screen.getByRole("heading", { name: "Test world" })).toBeTruthy();
});

it("waits for an older autosave before flushing the latest draft and leaving", async () => {
  projectNavigation();
  const normal = ipc.call.getMockImplementation()!;
  let completeOlder!: () => void;
  let firstSave = true;
  ipc.call.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      if (command === "save_draft" && firstSave) {
        firstSave = false;
        await new Promise<void>((resolve) => {
          completeOlder = resolve;
        });
      }
      return normal(command, args);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: /Share update/ }));
  const field = screen.getByPlaceholderText(
    "Fixed passengers falling through the boat deck",
  );
  fireEvent.change(field, { target: { value: "Older autosave" } });
  await waitFor(() => expect(completeOlder).toBeTypeOf("function"));
  fireEvent.change(field, { target: { value: "Newest draft" } });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Close" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  fireEvent.click(screen.getByRole("button", { name: "All projects" }));
  expect(ipc.call).not.toHaveBeenCalledWith("leave_project");
  completeOlder();
  await screen.findByRole("heading", { name: "Your projects" });
  expect(
    ipc.call.mock.calls
      .filter(([cmd]) => cmd === "save_draft")
      .map(([, args]) => args.draft.summary),
  ).toEqual(["Older autosave", "Newest draft"]);
});

it("shows plain screen titles, useful status and actions without promotional copy", async () => {
  const normal = ipc.call.getMockImplementation()!;
  ipc.call.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      if (command === "history_page") return { entries: [], next: null };
      return normal(command, args);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  expect(screen.queryByText("Practice project")).toBeNull();
  expect(screen.getByRole("button", { name: "All projects" })).toBeTruthy();
  expect(
    document.querySelector(".eyebrow, .summary-grid, .privacy-note, .footer"),
  ).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  expect(await screen.findByRole("heading", { name: "History" })).toBeTruthy();
  expect(screen.getByRole("combobox", { name: "Slice" })).toBeTruthy();
  expect(screen.queryByRole("textbox", { name: "Search history" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "App settings" }));
  expect(
    await screen.findByRole("heading", { name: "App settings" }),
  ).toBeTruthy();
  expect(
    screen.queryByRole("button", { name: "Test notification" }),
  ).toBeNull();
  expect(screen.queryByText("Storage app registrations")).toBeNull();
  expect(screen.queryByText("Project recovery")).toBeNull();
});

it("ignores an older workspace refresh after a newer overview snapshot", async () => {
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  const oldWorkspace = structuredClone(state);
  let completeOlder!: (value: Snapshot) => void;
  let first = true;
  const normal = ipc.call.getMockImplementation()!;
  ipc.call.mockImplementation(
    async (command: string, args?: Record<string, unknown>) => {
      if (command === "snapshot") {
        if (first) {
          first = false;
          return new Promise<Snapshot>((resolve) => {
            completeOlder = resolve;
          });
        }
        return structuredClone(empty);
      }
      return normal(command, args);
    },
  );
  void ipc.listeners.get("state-changed")!();
  void ipc.listeners.get("state-changed")!();
  await screen.findByRole("heading", { name: "Your projects" });
  await act(async () => {
    completeOlder(oldWorkspace);
  });
  expect(screen.queryByRole("heading", { name: "Test world" })).toBeNull();
  expect(screen.getByRole("heading", { name: "Your projects" })).toBeTruthy();
});

it("shows the current app brand in the project navigation", async () => {
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  const brand = screen.getByRole("link", { name: "miliastra/collab" });
  expect(brand.querySelector("img")?.getAttribute("src")).toBe("/app-logo.png");
  expect(brand.querySelector("img")?.getAttribute("alt")).toBe("");
  expect(screen.queryByText(/Miliastra Together/i)).toBeNull();
});

it("shows storage account connections on Settings and refreshes after connect or disconnect", async () => {
  state.storageAccounts = { google: "connected", onedrive: "notConnected" };
  state.cloudProviders = ["onedrive", "google"];
  ipc.call.mockImplementation(
    async (command: string, args?: { provider: "google" | "onedrive" }) => {
      if (command === "sign_in")
        state.storageAccounts![args!.provider] = "connected";
      if (command === "disconnect")
        state.storageAccounts![args!.provider] = "notConnected";
      return command === "snapshot"
        ? structuredClone(state)
        : command === "connections"
          ? []
          : null;
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "App settings" }));
  const google = screen.getByRole("group", { name: "Google Drive account" });
  const onedrive = screen.getByRole("group", { name: "OneDrive account" });
  expect(within(google).getByRole("status").textContent).toBe("Connected");
  expect(within(onedrive).getByRole("status").textContent).toBe(
    "Not connected",
  );
  fireEvent.click(
    within(onedrive).getByRole("button", { name: "Connect OneDrive" }),
  );
  const success = await screen.findByRole("dialog", {
    name: "Connected to OneDrive",
  });
  fireEvent.click(within(success).getByRole("button", { name: "Done" }));
  expect(
    screen.queryByRole("dialog", { name: "Connected to OneDrive" }),
  ).toBeNull();
  await within(onedrive).findByRole("button", { name: "Disconnect OneDrive" });
  expect(within(onedrive).getByRole("status").textContent).toBe("Connected");
  fireEvent.click(
    within(google).getByRole("button", { name: "Disconnect Google Drive" }),
  );
  await within(google).findByRole("button", { name: "Connect Google Drive" });
  expect(within(google).getByRole("status").textContent).toBe("Not connected");
  cleanup();
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "App settings" }));
  expect(
    within(screen.getByRole("group", { name: "OneDrive account" })).getByRole(
      "status",
    ).textContent,
  ).toBe("Connected");
});

it("shows provider sign-in failures without a success acknowledgement or connected status", async () => {
  state.storageAccounts = { google: "notConnected", onedrive: "notConnected" };
  ipc.call.mockImplementation(async (command: string) => {
    if (command === "sign_in")
      throw Error(
        "Sign-in failed (invalid_request): Consumer userAudience cannot use common",
      );
    return command === "snapshot"
      ? structuredClone(state)
      : command === "connections"
        ? []
        : null;
  });
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  fireEvent.click(screen.getByRole("button", { name: "App settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Connect OneDrive" }));
  expect((await screen.findByRole("alert")).textContent).toContain(
    "Consumer userAudience cannot use common",
  );
  expect(
    screen.queryByRole("dialog", { name: "Connected to OneDrive" }),
  ).toBeNull();
  expect(
    within(screen.getByRole("group", { name: "OneDrive account" })).getByRole(
      "status",
    ).textContent,
  ).toBe("Not connected");
});

it("opens an incoming invitation over the active workspace without losing the current draft", async () => {
  const invitation = {
    version: 1,
    endpoint: "https://collabs.miliastra.dev",
    project: "new-project",
    invite: "secret",
  };
  let incoming = false;
  ipc.call.mockImplementation(async (command: string) =>
    command === "snapshot"
      ? structuredClone(state)
      : command === "connections" || command === "discover"
        ? []
        : command === "invitation_details"
          ? {
              projectName: "Invited world",
              storageKind: "folder",
              mappedFolder: null,
            }
          : command === "pending_invitation" && incoming
            ? invitation
            : null,
  );
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  incoming = true;
  await act(async () => {
    await ipc.listeners.get("invitation-opened")?.();
  });
  const dialog = await screen.findByRole("dialog", {
    name: "Join Invited world",
  });
  expect(
    within(dialog).getByRole("list", { name: "Join progress" }),
  ).toBeTruthy();
  expect(screen.getByRole("heading", { name: "Test world" })).toBeTruthy();
  expect(ipc.call).not.toHaveBeenCalledWith("leave_project");
  expect(state.draft.summary).toBe("Recovered draft");
});

it("keeps Share progress and failures inside the dialog and retains the draft for retry", async () => {
  const normal = ipc.call.getMockImplementation()!;
  let rejectUpload!: (e: Error) => void;
  let completeUpload!: () => void;
  let attempts = 0;
  ipc.call.mockImplementation(async (command, args) => {
    if (command === "share_update") {
      attempts++;
      await new Promise<void>((resolve, reject) => {
        completeUpload = resolve;
        rejectUpload = reject;
      });
    }
    return normal(command, args);
  });
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  expect(document.querySelector(".identity")).toBeNull();
  const slot = screen.getByTestId("page-progress-slot");
  fireEvent.click(screen.getByRole("button", { name: /Share update/ }));
  const dialog = screen.getByRole("dialog", { name: "Share update" });
  const button = within(dialog).getByRole("button", {
    name: "Share update",
  });
  await waitFor(() => expect(button).toBeEnabled());
  expect(
    within(dialog).queryByRole("button", { name: "Refresh saved changes" }),
  ).toBeNull();
  expect(within(dialog).queryByText(/Your draft is kept/)).toBeNull();
  fireEvent.click(button);
  await waitFor(() => expect(attempts).toBe(1));
  expect(within(dialog).getByRole("status")).toHaveTextContent(
    "Preparing your update",
  );
  expect(slot.querySelector("[role=status]")).toBeNull();
  expect(screen.getByTestId("page-progress-slot")).toBe(slot);
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
  await act(async () => {
    ipc.listeners.get("publication-progress")!({
      payload: { project: "other", message: "Wrong project" },
    });
  });
  expect(within(dialog).getByRole("status")).not.toHaveTextContent(
    "Wrong project",
  );
  await act(async () => {
    ipc.listeners.get("publication-progress")!({
      payload: { project: "first", message: "Uploading Boats (1 of 1)…" },
    });
  });
  expect(within(dialog).getByRole("status")).toHaveTextContent(
    "Uploading Boats (1 of 1)",
  );
  await act(async () => {
    rejectUpload(Error("Storage unavailable"));
  });
  expect(within(dialog).getByRole("alert")).toHaveTextContent(
    "Storage unavailable",
  );
  expect(
    within(dialog).getByRole("textbox", { name: /What changed/ }),
  ).toHaveValue("Recovered draft");
  expect(button).toBeEnabled();
  fireEvent.click(button);
  await waitFor(() => expect(attempts).toBe(2));
  expect(within(dialog).getByRole("status")).toHaveTextContent(
    "Preparing your update",
  );
  await act(async () => {
    completeUpload();
  });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(ipc.call).toHaveBeenCalledWith("save_task", {
    projectId: "first",
    task: null,
  });
});
it("retains the page progress slot through a pending reservation", async () => {
  let complete!: () => void;
  const normal = ipc.call.getMockImplementation()!;
  ipc.call.mockImplementation(async (command, args) => {
    if (command === "start_work")
      await new Promise<void>((resolve) => {
        complete = resolve;
      });
    return normal(command, args);
  });
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  const slot = screen.getByTestId("page-progress-slot");
  expect(slot).toBeEmptyDOMElement();
  fireEvent.click(screen.getByRole("button", { name: /Start work/ }));
  await waitFor(() => expect(complete).toBeTypeOf("function"));
  expect(within(slot).getByRole("status")).toHaveTextContent("Reserving slice");
  await act(async () => {
    complete();
  });
  await waitFor(() => expect(slot).toBeEmptyDOMElement());
  expect(screen.getByTestId("page-progress-slot")).toBe(slot);
});

it("hides Google Drive when this build doesn't offer it", async () => {
  state.cloudProviders = ["onedrive"];
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "App settings" }));
  await screen.findByRole("group", { name: "OneDrive account" });
  expect(
    screen.queryByRole("group", { name: "Google Drive account" }),
  ).toBeNull();
});

it("shows a retry action while shared update details are unavailable", async () => {
  state.metadataPending = true;
  render(<App />);
  expect(
    await screen.findByText(/Waiting for update details/),
  ).toBeInTheDocument();
  expect(
    screen.getAllByRole("button", { name: "Check for changes" }).length,
  ).toBeGreaterThan(0);
});

it("places the project connection warning above storage recovery and project content", async () => {
  state.online = false;
  state.project!.storage.kind = "onedrive";
  state.storageAccounts = { onedrive: "reconnect" };
  render(<App />);
  await screen.findByRole("heading", { name: "Test world" });
  const connection = screen.getByRole("status", { name: "Project connection" });
  const storage = screen
    .getByText("Reconnect OneDrive", { selector: "strong" })
    .closest(".banner")!;
  expect(document.querySelector(".content")!.firstElementChild).toBe(
    connection,
  );
  expect(
    connection.compareDocumentPosition(storage) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
  expect(
    screen.queryByText(/It includes the invitation and Encryption/),
  ).toBeNull();
});
it("offers the required app update as soon as the service reports a version mismatch", async () => {
  state.online = false;
  state.protocolMismatch = true;
  ipc.call.mockImplementation(async (command: string) =>
    command === "snapshot"
      ? structuredClone(state)
      : command === "check_application_update"
        ? { version: "0.2.0", notes: "", canReplace: true }
        : command === "connections"
          ? []
          : null,
  );
  render(<App />);
  const banner = await screen.findByRole("status", { name: "App update" });
  expect(within(banner).getByText("Update required")).toBeVisible();
  expect(within(banner).queryByRole("button", { name: "Later" })).toBeNull();
  fireEvent.click(
    within(banner).getByRole("button", { name: "Restart and update" }),
  );
  await waitFor(() =>
    expect(ipc.call).toHaveBeenCalledWith("install_application_update"),
  );
});

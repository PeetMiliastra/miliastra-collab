// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { StrictMode } from "react";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
  act,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, it, expect, vi } from "vitest";
const ipc = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ desktop: true, call: ipc }));
import { JoinWizard } from "./JoinWizard";
import type { Run } from "./types";
const invitation = {
  version: 1,
  endpoint: "https://collabs.example",
  project: "p",
  invite: "private",
};
const run: Run = async (_, action, options) => {
  try {
    await action();
  } catch (e) {
    options?.onError?.(String(e).replace(/^Error: /, ""));
  }
};
const props = {
  invitation,
  providers: ["onedrive"] as ("onedrive" | "google")[],
  storageAccounts: {
    google: "notConnected" as const,
    onedrive: "connected" as const,
  },
  busy: "",
  run,
  onClose: vi.fn(),
  onDone: vi.fn(),
  onBeforeJoin: vi.fn(async () => {}),
};
let handler: (command: string, args?: any) => unknown;
beforeEach(() => {
  vi.clearAllMocks();
  handler = (command) =>
    command === "invitation_details"
      ? {
          projectName: "Shared world",
          storageKind: "onedrive",
          mappedFolder: null,
        }
      : command === "invitation_privacy_key_ready"
        ? true
        : command === "check_invitation_storage"
          ? { baseStage: true }
          : command === "find_project_stage"
            ? "matching-stage"
            : null;
  ipc.mockImplementation(async (command, args) => handler(command, args));
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});
async function join() {
  await screen.findByRole("dialog", { name: "Join Shared world" });
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Join project" }));
}
async function tick(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}
it("waits for Join before checking storage and joins an existing matching stage exactly once", async () => {
  render(
    <StrictMode>
      <JoinWizard {...props} />
    </StrictMode>,
  );
  await screen.findByRole("dialog", { name: "Join Shared world" });
  expect(
    ipc.mock.calls.some(
      ([cmd]) => cmd === "check_invitation_storage" || cmd === "join_project",
    ),
  ).toBe(false);
  await join();
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "matching-stage",
    folder: null,
  });
  expect(ipc.mock.calls.filter(([cmd]) => cmd === "join_project")).toHaveLength(
    1,
  );
  expect(ipc.mock.calls.some(([cmd]) => cmd === "download_base_stage")).toBe(
    false,
  );
  expect(props.onBeforeJoin).toHaveBeenCalledOnce();
  expect(screen.queryByText(/Files go in|Sign in with the account/)).toBeNull();
});
it("waits for automatic sharing, downloads once, prompts import and automatically discovers the imported stage", async () => {
  let access = 0,
    find = 0,
    status = 0;
  handler = (command) => {
    if (command === "invitation_details")
      return {
        projectName: "Shared world",
        ownerName: "Alex",
        storageKind: "onedrive",
        mappedFolder: null,
      };
    if (command === "check_invitation_storage") {
      if (++access < 3) throw Error("No folder access yet");
      return { baseStage: true };
    }
    if (command === "invitation_privacy_key_ready") return true;
    if (command === "request_folder_access") return "request-id";
    if (command === "folder_access_status")
      return { status: ++status === 1 ? "pending" : "granted", message: "" };
    if (command === "find_project_stage")
      return ++find < 3 ? null : "imported-stage";
    if (command === "download_base_stage")
      return "C:/Downloads/Starting world.gil";
  };
  render(<JoinWizard {...props} />);
  await screen.findByRole("dialog", { name: "Join Shared world" });
  vi.useFakeTimers();
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Join project" }));
  await tick(0);
  expect(
    screen.getByRole("status", { name: "Join setup progress" }),
  ).toHaveTextContent("Waiting for Alex to give you access");
  expect(
    screen.getByRole("status", { name: "Join setup progress" }),
  ).toHaveTextContent("as soon as Alex has miliastra/collab open");
  expect(screen.queryByRole("textbox")).toBeNull();
  await tick(10000);
  expect(
    screen.getByRole("heading", { name: "Import the starting stage" }),
  ).toBeVisible();
  expect(screen.getByText("Starting world.gil")).toBeVisible();
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "request_folder_access"),
  ).toHaveLength(1);
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "download_base_stage"),
  ).toHaveLength(1);
  expect(ipc.mock.calls.some(([cmd]) => cmd === "join_project")).toBe(false);
  await tick(3000);
  expect(props.onDone).toHaveBeenCalledOnce();
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "imported-stage",
    folder: null,
  });
  expect(props.onBeforeJoin).toHaveBeenCalledOnce();
});
it("retains a manually chosen shared-folder mapping across an access failure and retry", async () => {
  let fail = true;
  handler = (command) => {
    if (command === "invitation_details")
      return {
        projectName: "Shared world",
        storageKind: "folder",
        mappedFolder: null,
      };
    if (command === "choose_folder") return "private-exchange";
    if (command === "check_invitation_storage" && fail)
      throw Error("Folder unavailable");
    if (command === "check_invitation_storage") return { baseStage: true };
    if (command === "find_project_stage") return "matching-stage";
  };
  render(<JoinWizard {...props} />);
  await screen.findByRole("dialog", { name: "Join Shared world" });
  await join();
  expect(screen.getByRole("button", { name: "Join project" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Choose shared folder" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Join project" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Join project" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Folder unavailable",
  );
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "matching-stage",
    folder: "private-exchange",
  });
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "choose_folder"),
  ).toHaveLength(1);
});
it("keeps failed downloads on their step and retries without making membership or losing the name", async () => {
  let fail = true;
  const normal = handler;
  handler = (command) => {
    if (command === "find_project_stage") return null;
    if (command === "download_base_stage") {
      if (fail) throw Error("Checksum failed");
      return "C:/Downloads/World.gil";
    }
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  await join();
  expect(await screen.findByRole("alert")).toHaveTextContent("Checksum failed");
  expect(props.onBeforeJoin).not.toHaveBeenCalled();
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByRole("heading", { name: "Import the starting stage" });
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "download_base_stage"),
  ).toHaveLength(2);
  expect(props.onDone).not.toHaveBeenCalled();
});
it("stops polling and never joins after the waiting wizard is cancelled", async () => {
  const normal = handler;
  handler = (command) => {
    if (command === "check_invitation_storage") throw Error("Waiting");
    if (command === "request_folder_access") return "request";
    if (command === "folder_access_status")
      return { status: "pending", message: "" };
    return normal(command);
  };
  const view = render(<JoinWizard {...props} />);
  await join();
  await screen.findByText(/Waiting for the project owner/);
  vi.useFakeTimers();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(props.onClose).toHaveBeenCalledOnce();
  view.unmount();
  const count = ipc.mock.calls.length;
  await tick(20000);
  expect(ipc).toHaveBeenCalledTimes(count);
  expect(props.onBeforeJoin).not.toHaveBeenCalled();
  expect(props.onDone).not.toHaveBeenCalled();
});
it("provides an explicit stage choice if the project has no starting stage", async () => {
  const normal = handler;
  handler = (command) =>
    command === "check_invitation_storage"
      ? { baseStage: false }
      : command === "find_project_stage"
        ? null
        : command === "discover"
          ? [
              { path: "one", name: "Fisch", account: "A", level: "1" },
              { path: "two", name: "Fisch", account: "A", level: "2" },
            ]
          : normal(command);
  render(<JoinWizard {...props} />);
  await join();
  await screen.findByRole("heading", { name: "Choose your stage" });
  expect(screen.getByRole("button", { name: "Use this stage" })).toBeDisabled();
  expect(
    screen.getByRole("option", { name: "Fisch (account A, stage 2)" }),
  ).toBeVisible();
  fireEvent.change(
    screen.getByRole("combobox", { name: "Your Miliastra stage" }),
    { target: { value: "two" } },
  );
  fireEvent.click(screen.getByRole("button", { name: "Use this stage" }));
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "two",
    folder: null,
  });
});
it("rejects disabled Google Drive invitations without exposing its sign-in or checking storage", async () => {
  handler = (command) =>
    command === "invitation_details"
      ? {
          projectName: "Shared world",
          storageKind: "google",
          mappedFolder: null,
        }
      : null;
  render(<JoinWizard {...props} />);
  await screen.findByRole("alert");
  expect(
    screen.queryByRole("group", { name: "Google Drive account" }),
  ).toBeNull();
  expect(screen.getByRole("button", { name: "Join project" })).toBeDisabled();
  expect(
    ipc.mock.calls.some(([cmd]) => cmd === "check_invitation_storage"),
  ).toBe(false);
});
it("blocks unresolved invitations, retries in place and preserves the entered name", async () => {
  let fail = true;
  const normal = handler;
  handler = (command) => {
    if (command === "invitation_details" && fail)
      throw Error("Service unavailable");
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Service unavailable",
  );
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  expect(screen.getByRole("button", { name: "Join project" })).toBeDisabled();
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await screen.findByRole("dialog", { name: "Join Shared world" });
  expect(screen.getByDisplayValue("Robin")).toBeVisible();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Join project" })).toBeEnabled(),
  );
});
it("connects only the invitation provider, acknowledges success in one modal and keeps form state", async () => {
  const view = render(
    <JoinWizard
      {...props}
      storageAccounts={{ google: "notConnected", onedrive: "notConnected" }}
    />,
  );
  await screen.findByRole("dialog", { name: "Join Shared world" });
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Connect OneDrive" }));
  await screen.findByRole("dialog", { name: "Connected to OneDrive" });
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.getByDisplayValue("Robin")).toBeVisible();
  view.rerender(<JoinWizard {...props} />);
  expect(
    within(screen.getByRole("group", { name: "OneDrive account" })).getByRole(
      "status",
    ),
  ).toHaveTextContent("Connected");
  expect(
    screen.queryByRole("group", { name: "Google Drive account" }),
  ).toBeNull();
});
it("retains the detected stage after a failed final join and retries only the final step", async () => {
  let fail = true;
  const normal = handler;
  handler = (command) => {
    if (command === "join_project" && fail)
      throw Error("Temporary service error");
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  await join();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Temporary service error",
  );
  fail = false;
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "find_project_stage"),
  ).toHaveLength(1);
  expect(ipc.mock.calls.filter(([cmd]) => cmd === "join_project")).toHaveLength(
    2,
  );
});

it("requires the invitation message’s Encryption Key before requesting cloud folder access", async () => {
  const normal = handler;
  let saved = false;
  handler = (command) => {
    if (command === "check_invitation_storage") throw Error("No folder access");
    if (command === "invitation_privacy_key_ready") return saved;
    if (command === "save_folder_privacy_key") {
      saved = true;
      return null;
    }
    if (command === "request_folder_access") return "encrypted-request";
    if (command === "folder_access_status")
      return { status: "pending", message: "" };
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  await join();
  await screen.findByRole("heading", {
    name: "Paste the project’s Encryption Key",
  });
  expect(ipc.mock.calls.some(([c]) => c === "request_folder_access")).toBe(
    false,
  );
  const next = screen.getByRole("button", {
    name: "Continue with Encryption Key",
  });
  expect(next).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Encryption Key"), {
    target: { value: "mcollab-key-v2:" + "a".repeat(160) },
  });
  fireEvent.click(next);
  await screen.findByText(/Waiting for the project owner/);
  expect(ipc).toHaveBeenCalledWith("save_folder_privacy_key", {
    invitation,
    key: "mcollab-key-v2:" + "a".repeat(160),
  });
  expect(
    ipc.mock.calls.filter(([c]) => c === "request_folder_access"),
  ).toHaveLength(1);
  expect(
    ipc.mock.calls.filter(([c]) => c === "request_folder_access")[0][1],
  ).not.toHaveProperty("key");
});

it("lets a wrong-key join retry the connection without losing the display name", async () => {
  const normal = handler;
  let connected = false;
  handler = (command) => {
    if (command === "check_invitation_storage") {
      if (!connected) throw Error("No access");
      return { baseStage: true };
    }
    if (command === "request_folder_access") return "request";
    if (command === "folder_access_status")
      return {
        status: "failed",
        message:
          "The Encryption Key did not match. Ask the owner for the current key, then retry.",
      };
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  await join();
  await screen.findByRole("heading", {
    name: "Paste the project’s Encryption Key",
  });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Encryption Key did not match",
  );
  connected = true;
  fireEvent.click(screen.getByRole("button", { name: /^Retry$/ }));
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "matching-stage",
    folder: null,
  });
});

it("requires the key before revealing names, provider or folder, even for shared-folder projects", async () => {
  const normal = handler;
  let unlocked = false;
  handler = (command) => {
    if (command === "invitation_details")
      return unlocked
        ? {
            projectName: "Shared world",
            storageKind: "folder",
            mappedFolder: "shared-folder",
            privacyKeyRequired: false,
          }
        : {
            projectName: "Encrypted project",
            storageKind: "locked",
            mappedFolder: null,
            privacyKeyRequired: true,
          };
    if (command === "save_folder_privacy_key") {
      unlocked = true;
      return null;
    }
    return normal(command);
  };
  render(<JoinWizard {...props} />);
  await screen.findByRole("heading", {
    name: "Paste the project’s Encryption Key",
  });
  expect(screen.queryByPlaceholderText("Display name")).not.toBeInTheDocument();
  expect(screen.queryByText("Shared world")).not.toBeInTheDocument();
  expect(
    ipc.mock.calls.some(([cmd]) =>
      [
        "check_invitation_storage",
        "request_folder_access",
        "join_project",
      ].includes(cmd),
    ),
  ).toBe(false);
  fireEvent.change(screen.getByLabelText("Encryption Key"), {
    target: { value: "mcollab-key-v2:" + "a".repeat(160) },
  });
  fireEvent.click(
    screen.getByRole("button", { name: "Continue with Encryption Key" }),
  );
  await screen.findByRole("dialog", { name: "Join Shared world" });
  expect(screen.getByPlaceholderText("Display name")).toBeVisible();
  expect(screen.getByText("Shared folder selected")).toBeVisible();
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "invitation_details"),
  ).toHaveLength(2);
  expect(
    ipc.mock.calls.some(([cmd]) => cmd === "check_invitation_storage"),
  ).toBe(false);
});
it("asks the joiner to request the folder from an offline owner, or when sharing is slow", async () => {
  let ownerOnline = false;
  handler = (command) => {
    if (command === "invitation_details")
      return {
        projectName: "Shared world",
        ownerName: "Alex",
        storageKind: "onedrive",
        mappedFolder: null,
      };
    if (command === "check_invitation_storage")
      throw Error("No folder access yet");
    if (command === "invitation_privacy_key_ready") return true;
    if (command === "request_folder_access") return "request";
    if (command === "folder_access_status")
      return { status: "pending", message: "", ownerOnline };
    if (command === "storage_account_email") return "robin@example.com";
  };
  render(<JoinWizard {...props} />);
  await screen.findByRole("dialog", { name: "Join Shared world" });
  vi.useFakeTimers();
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Join project" }));
  await tick(0);
  const note = () => screen.queryByRole("note");
  expect(note()).toHaveTextContent(
    "Ask Alex to share the project folder with you",
  );
  expect(note()).toHaveTextContent("robin@example.com");
  expect(screen.getByRole("button", { name: "Copy my email" })).toBeVisible();
  // Owner comes online: back to waiting for the automatic share.
  ownerOnline = true;
  await tick(5000);
  expect(note()).toBeNull();
  expect(
    screen.getByRole("status", { name: "Join setup progress" }),
  ).toHaveTextContent("as soon as Alex has miliastra/collab open");
  // Still no access after two minutes, although the owner is online.
  await tick(2 * 60 * 1000);
  await tick(5000);
  expect(note()).toHaveTextContent(
    "Ask Alex to share the project folder with you",
  );
  vi.useRealTimers();
});

it("lets creators choose an imported copy explicitly and ignores late automatic matches", async () => {
  const normal = handler;
  let searches = 0;
  let finishSearch: (path: string) => void = () => {};
  let finishDiscovery: (stages: unknown[]) => void = () => {};
  handler = (command, args) => {
    if (command === "find_project_stage") {
      if (++searches === 1) return null;
      return new Promise<string>((resolve) => {
        finishSearch = resolve;
      });
    }
    if (command === "download_base_stage") return "C:/Downloads/World.gil";
    if (command === "discover")
      return new Promise<unknown[]>((resolve) => {
        finishDiscovery = resolve;
      });
    return normal(command, args);
  };
  render(<JoinWizard {...props} />);
  await join();
  fireEvent.click(
    await screen.findByRole("button", { name: "Choose imported stage" }),
  );
  await screen.findByRole("heading", { name: "Choose your stage" });
  expect(
    screen.getByText("Select the stage you imported and saved in Miliastra."),
  ).toBeVisible();
  expect(screen.queryByText(/This project has no starting stage/)).toBeNull();
  const useStage = screen.getByRole("button", { name: "Use this stage" });
  expect(useStage).toBeDisabled();
  await act(async () => finishSearch("late-automatic-copy"));
  expect(ipc.mock.calls.some(([command]) => command === "join_project")).toBe(
    false,
  );
  await act(async () =>
    finishDiscovery([
      { path: "first-copy", name: "World", account: "A", level: "1" },
      { path: "second-copy", name: "World", account: "A", level: "2" },
    ]),
  );
  fireEvent.change(screen.getByLabelText("Your Miliastra stage"), {
    target: { value: "second-copy" },
  });
  fireEvent.click(useStage);
  await waitFor(() => expect(props.onDone).toHaveBeenCalledOnce());
  expect(ipc).toHaveBeenCalledWith("join_project", {
    invitation,
    displayName: "Robin",
    stagePath: "second-copy",
    folder: null,
  });
});

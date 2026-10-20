// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
  act,
  within,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, it, expect, vi } from "vitest";
const ipc = vi.hoisted(() => ({
  call: vi.fn(),
  progress: undefined as undefined | ((event: { payload: string }) => void),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (_: string, callback: (event: { payload: string }) => void) => {
      if (_ === "project-setup-progress") ipc.progress = callback;
      return () => {};
    },
  ),
}));
vi.mock("./bridge", () => ({ desktop: true, call: ipc.call }));
import { Setup } from "./Setup";
import { empty } from "./App";
afterEach(cleanup);
it("shows stage identifiers only when names and accounts cannot distinguish choices", async () => {
  ipc.call.mockImplementation(async (command: string) =>
    command === "discover"
      ? [
          { path: "first", name: "Fisch", account: "A", level: "10" },
          { path: "second", name: "Fisch", account: "A", level: "20" },
          { path: "third", name: "Racing", account: "A", level: "30" },
        ]
      : null,
  );
  render(
    <Setup
      busy=""
      run={async (_, action) => {
        await action();
      }}
      onClose={() => {}}
      onDone={() => {}}
    />,
  );
  expect(
    await screen.findByRole("option", { name: "Fisch (account A, stage 10)" }),
  ).toBeTruthy();
  expect(
    screen.getByRole("option", { name: "Fisch (account A, stage 20)" }),
  ).toBeTruthy();
  expect(screen.getByRole("option", { name: "Racing" })).toBeTruthy();
  expect(screen.queryByText(/Can’t find your stage/)).toBeNull();
});

function creationForm(create: () => Promise<unknown>) {
  ipc.call.mockImplementation(async (command: string) => {
    if (command === "discover")
      return [{ path: "stage", name: "Fisch", account: "A", level: "10" }];
    if (command === "create_project") return create();
    return null;
  });
  const done = vi.fn(),
    close = vi.fn();
  function Harness() {
    const [open, setOpen] = useState(true);
    return open ? (
      <Setup
        providers={["onedrive", "google"]}
        storageAccounts={{ google: "connected", onedrive: "notConnected" }}
        busy=""
        run={async (_, action) => {
          try {
            await action();
          } catch {}
        }}
        onClose={close}
        onDone={() => {
          done();
          setOpen(false);
        }}
      />
    ) : (
      <h1>Project created</h1>
    );
  }
  render(<Harness />);
  return { done, close };
}
async function fillCreation() {
  await screen.findByText("Fisch");
  fireEvent.change(screen.getByPlaceholderText("Display name"), {
    target: { value: "Robin" },
  });
  fireEvent.change(screen.getByPlaceholderText("Project name"), {
    target: { value: "Fishing" },
  });
  fireEvent.change(
    screen.getByRole("combobox", { name: "Keep the team folder in" }),
    {
      target: { value: "google" },
    },
  );
  return screen.getAllByRole("button", { name: "Create project" }).at(-1)!;
}
it("shows immediate in-dialog progress, prevents duplicate submission/closing, and closes after success", async () => {
  let finish!: () => void;
  const create = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const { done, close } = creationForm(create);
  const submit = await fillCreation();
  act(() => {
    fireEvent.click(submit);
    fireEvent.click(submit);
  });
  expect(
    screen.getByRole("status", { name: "Project setup progress" }).textContent,
  ).toContain("Preparing project");
  expect(
    (screen.getByRole("button", { name: "Close" }) as HTMLButtonElement)
      .disabled,
  ).toBe(true);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(close).not.toHaveBeenCalled();
  await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
  act(() =>
    ipc.progress?.({ payload: "Uploading saved sections (25 of 65)…" }),
  );
  expect(
    screen.getByRole("status", { name: "Project setup progress" }).textContent,
  ).toContain("25 of 65");
  await act(async () => finish());
  await screen.findByRole("heading", { name: "Project created" });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(done).toHaveBeenCalledOnce();
});
it("shows a creation failure inside the form, keeps its values and allows retry", async () => {
  let fail = true;
  const create = vi.fn(async () => {
    if (fail) throw Error("The collaboration service is unavailable");
  });
  creationForm(create);
  fireEvent.click(await fillCreation());
  expect((await screen.findByRole("alert")).textContent).toContain(
    "collaboration service is unavailable",
  );
  expect(
    (screen.getByPlaceholderText("Project name") as HTMLInputElement).value,
  ).toBe("Fishing");
  expect(
    screen.queryByRole("status", { name: "Project setup progress" }),
  ).toBeNull();
  expect(screen.queryByText("Coordination service")).toBeNull();
  fail = false;
  fireEvent.click(
    screen.getAllByRole("button", { name: "Create project" }).at(-1)!,
  );
  await screen.findByRole("heading", { name: "Project created" });
  expect(create).toHaveBeenCalledTimes(2);
});

it("shows a previously connected provider during setup and updates after sign-in", async () => {
  ipc.call.mockImplementation(async (command: string) =>
    command === "discover" ? [] : null,
  );
  const props = {
    settings: empty.settings,
    busy: "",
    run: async (_: string, action: () => Promise<unknown>) => {
      await action();
    },
    onClose: () => {},
    onDone: () => {},
  };
  const view = render(
    <Setup
      providers={["onedrive", "google"]}
      {...props}
      storageAccounts={{ google: "connected", onedrive: "notConnected" }}
    />,
  );
  fireEvent.change(
    screen.getByRole("combobox", { name: "Keep the team folder in" }),
    {
      target: { value: "google" },
    },
  );
  expect(screen.queryByText(/Files go in miliastra-collab/)).toBeNull();
  const google = screen.getByRole("group", { name: "Google Drive account" });
  expect(within(google).getByRole("status").textContent).toBe("Connected");
  expect(
    within(google).queryByRole("button", { name: "Connect Google Drive" }),
  ).toBeNull();
  expect(
    within(google).getByRole("button", { name: "Disconnect Google Drive" }),
  ).toBeTruthy();
  view.rerender(
    <Setup
      providers={["onedrive", "google"]}
      {...props}
      storageAccounts={{ google: "reconnect", onedrive: "notConnected" }}
    />,
  );
  expect(within(google).getByRole("status").textContent).toBe(
    "Reconnect needed",
  );
  expect(
    within(google).getByRole("button", { name: "Reconnect Google Drive" }),
  ).toBeTruthy();
  view.rerender(
    <Setup
      providers={["onedrive", "google"]}
      {...props}
      storageAccounts={{ google: "connected", onedrive: "notConnected" }}
    />,
  );
  expect(within(google).getByRole("status").textContent).toBe("Connected");
});

it("acknowledges a successful sign-in in one modal, preserves setup entries, and never acknowledges failure", async () => {
  let finish!: () => void;
  let fail = false;
  ipc.call.mockImplementation(async (command: string) => {
    if (command === "discover") return [];
    if (command === "sign_in") {
      if (fail) throw Error("Sign-in was cancelled");
      await new Promise<void>((resolve) => {
        finish = resolve;
      });
    }
    return null;
  });
  const close = vi.fn();
  render(
    <Setup
      providers={["onedrive", "google"]}
      storageAccounts={{ google: "notConnected", onedrive: "notConnected" }}
      busy=""
      run={async (_, action) => {
        try {
          await action();
        } catch {}
      }}
      onClose={close}
      onDone={() => {}}
    />,
  );
  fireEvent.change(screen.getByPlaceholderText("Project name"), {
    target: { value: "My world" },
  });
  fireEvent.change(
    screen.getByRole("combobox", { name: "Keep the team folder in" }),
    {
      target: { value: "google" },
    },
  );
  fireEvent.click(screen.getByRole("button", { name: "Connect Google Drive" }));
  expect(
    screen.queryByRole("dialog", { name: "Connected to Google Drive" }),
  ).toBeNull();
  await act(async () => finish());
  await screen.findByRole("dialog", {
    name: "Connected to Google Drive",
  });
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(close).not.toHaveBeenCalled();
  expect(
    screen.queryByRole("dialog", { name: "Connected to Google Drive" }),
  ).toBeNull();
  expect(
    (screen.getByPlaceholderText("Project name") as HTMLInputElement).value,
  ).toBe("My world");
  fail = true;
  fireEvent.click(screen.getByRole("button", { name: "Connect Google Drive" }));
  await act(async () => {});
  expect(
    screen.queryByRole("dialog", { name: "Connected to Google Drive" }),
  ).toBeNull();
});

it("offers only the cloud providers this build supports when creating a project", async () => {
  ipc.call.mockImplementation(async (command: string) =>
    command === "discover"
      ? [{ path: "stage", name: "Fisch", account: "A", level: "10" }]
      : null,
  );
  render(
    <Setup
      providers={["onedrive"]}
      storageAccounts={{ google: "notConnected", onedrive: "notConnected" }}
      busy=""
      run={async (_, action) => {
        await action();
      }}
      onClose={vi.fn()}
      onDone={vi.fn()}
    />,
  );
  const storage = await screen.findByRole("combobox", {
    name: "Keep the team folder in",
  });
  const options = within(storage)
    .getAllByRole("option")
    .map((o) => o.textContent);
  expect(options).toEqual(["Shared folder (advanced)", "OneDrive"]);
  expect(storage).toHaveValue("onedrive");
  expect(
    screen.getByRole("button", { name: "Connect OneDrive" }),
  ).toBeVisible();
  fireEvent.change(storage, { target: { value: "folder" } });
  expect(
    screen.getByRole("button", { name: "Choose shared folder" }),
  ).toBeVisible();
});

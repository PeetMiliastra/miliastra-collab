// @vitest-environment jsdom
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
import { afterEach, it, expect, vi } from "vitest";
import { Workflow } from "./Workflow";
import { empty } from "./App";
import type { Snapshot, ConflictPreview, Revision } from "./types";
const ipc = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ desktop: true, call: ipc }));
afterEach(() => {
  cleanup();
  ipc.mockReset();
});
const revision: Revision = {
  section: "5",
  name: "Boats",
  hash: "a".repeat(64),
  contentHash: "b".repeat(64),
  artifact: "History/11111111-1111-4111-8111-111111111111/6.gis",
  parent: null,
  size: 100,
};
const state: Snapshot = {
  ...empty,
  online: true,
  member: "me",
  project: {
    id: "p",
    name: "Fisch",
    storage: { kind: "folder", location: "" },
    members: [],
    locks: [],
    heads: { "5": revision },
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
  compatibility: {
    directInstall: true,
    liveDetection: false,
    restoreVersions: true,
    reason: "",
  },
};
const run = async (_: string, f: () => Promise<unknown>) => {
  await f();
};
const props = {
  state,
  busy: "",
  run,
  draft: { summary: "Keep my changelog", details: "More context" },
  setDraft: vi.fn(),
  request: null,
};
it("keeps Share and its explicit selection after dismissal and reconstructs it after restart", async () => {
  ipc.mockResolvedValue(null);
  const view = render(
    <Workflow {...props} request={{ kind: "share", nonce: 1 }} />,
  );
  await screen.findByRole("dialog", { name: "Share update" });
  fireEvent.click(screen.getByRole("checkbox", { name: /Boats/ }));
  await waitFor(() =>
    expect(ipc).toHaveBeenLastCalledWith("save_task", {
      projectId: "p",
      task: { kind: "share", selected: [], manualSelection: true },
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  view.unmount();
  render(
    <Workflow
      {...props}
      state={{
        ...state,
        task: { kind: "share", selected: [], manualSelection: true },
      }}
    />,
  );
  expect(screen.queryByText("Share draft")).toBeNull();
  cleanup();
  render(
    <Workflow
      {...props}
      state={{
        ...state,
        task: { kind: "share", selected: [], manualSelection: true },
      }}
      request={{ kind: "share", nonce: 2 }}
    />,
  );
  expect(screen.getByRole("checkbox", { name: /Boats/ })).not.toBeChecked();
  expect(screen.getByDisplayValue("Keep my changelog")).toBeVisible();
  expect(screen.getByRole("button", { name: "Share update" })).toBeDisabled();
});
it("includes newly saved slices in the open Share context without discarding its changelog", async () => {
  ipc.mockResolvedValue(null);
  const view = render(
    <Workflow
      {...props}
      state={{ ...state, sections: [] }}
      request={{ kind: "share", nonce: 1 }}
    />,
  );
  await screen.findByText("No saved slice changes detected yet.");
  await waitFor(() =>
    expect(ipc).toHaveBeenCalledWith("refresh_saved_changes"),
  );
  view.rerender(<Workflow {...props} request={{ kind: "share", nonce: 1 }} />);
  expect(screen.getByRole("checkbox", { name: /Boats/ })).toBeChecked();
  expect(screen.getByDisplayValue("Keep my changelog")).toBeVisible();
  expect(screen.getByRole("button", { name: "Share update" })).toBeEnabled();
});
it("reviews conflicts inside Share and returns to the unchanged draft", async () => {
  const preview: ConflictPreview = {
    id: "c",
    project: "p",
    section: "5",
    name: "Boats",
    mine: revision,
    theirs: revision,
    baseline: null,
    folder: "private",
    at: 1,
    latestUpdate: null,
    canKeepMine: true,
    canUseTheirs: true,
    reason: "",
    draft: { summary: "", details: "" },
  };
  ipc.mockImplementation(async (cmd) =>
    cmd === "preview_conflict" ? preview : null,
  );
  render(
    <Workflow
      {...props}
      state={{ ...state, sections: [{ ...state.sections[0], conflict: true }] }}
      request={{ kind: "share", nonce: 1 }}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Review Boats" }));
  await screen.findByRole("dialog", { name: "Resolve Boats" });
  expect(screen.queryByRole("dialog", { name: "Share update" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Keep both for now" }));
  expect(screen.getByRole("dialog", { name: "Share update" })).toBeVisible();
  expect(screen.getByDisplayValue("Keep my changelog")).toBeVisible();
  expect(screen.getByRole("button", { name: "Share update" })).toBeDisabled();
});
it("resumes interrupted file verification after restart without a reload acknowledgement", async () => {
  ipc.mockResolvedValue(null);
  const record = {
    id: "job",
    phase: "awaitingReload" as const,
    journal: "private/journal.json",
    revisions: [revision],
    error: null,
  };
  render(
    <Workflow
      {...props}
      state={{
        ...state,
        task: { kind: "apply", selected: [], manualSelection: false },
        installation: record,
      }}
    />,
  );
  expect(ipc).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Resume" }));
  expect(screen.getByRole("dialog", { name: "Apply updates" })).toBeVisible();
  expect(screen.queryByRole("checkbox")).toBeNull();
  fireEvent.click(
    screen.getByRole("button", {
      name: "Check files and finish",
    }),
  );
  await waitFor(() => expect(ipc).toHaveBeenCalledWith("verify_install"));
});
it("continues directly from review and installation to completion in a single context", async () => {
  ipc.mockImplementation(async (cmd) =>
    cmd === "preview_install" ? { id: "plan", revisions: [revision] } : null,
  );
  const request = { kind: "apply" as const, nonce: 1 };
  const current = {
    ...state,
    sections: [{ ...state.sections[0], changed: false, installed: false }],
  };
  const view = render(
    <Workflow {...props} state={current} request={request} />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Continue" }));
  await screen.findByRole("button", { name: "Back up and apply" });
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Back up and apply" }));
  await waitFor(() =>
    expect(ipc).toHaveBeenCalledWith("install_updates", {
      planId: "plan",
      editorReady: true,
    }),
  );
  view.rerender(
    <Workflow
      {...props}
      state={{
        ...current,
        sections: [{ ...current.sections[0], installed: true }],
        installation: {
          id: "job",
          phase: "verified",
          journal: "private/journal.json",
          revisions: [revision],
          error: null,
        },
      }}
      request={request}
    />,
  );
  expect(
    screen.getByRole("heading", { name: "Updates applied" }),
  ).toBeVisible();
  expect(
    screen.getByText("Open the stage in Miliastra to load them."),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: /Undo/ })).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Check files and finish" }),
  ).toBeNull();
  expect(ipc.mock.calls.some(([cmd]) => cmd === "verify_install")).toBe(false);
  expect(
    ipc.mock.calls.filter(([cmd]) => cmd === "preview_install"),
  ).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
  expect(screen.queryByText("Last installation")).toBeNull();
});

it("shows a failed conflict preparation inside Share and preserves the draft for retry", async () => {
  ipc.mockImplementation(async (cmd) => {
    if (cmd === "preview_conflict")
      throw Error("The teammate version is not downloaded yet");
    return null;
  });
  const run: import("./types").Run = async (_, action, options) => {
    try {
      await action();
    } catch (e) {
      options?.onError?.(String(e));
    }
  };
  render(
    <Workflow
      {...props}
      run={run}
      state={{ ...state, sections: [{ ...state.sections[0], conflict: true }] }}
      request={{ kind: "share", nonce: 10 }}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Review Boats" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "not downloaded yet",
  );
  expect(screen.getByRole("dialog", { name: "Share update" })).toBeVisible();
  expect(screen.getByDisplayValue("Keep my changelog")).toBeVisible();
});

it.each(["verified", "undone"] as const)(
  "does not show a banner for a %s installation or slice revert after restart",
  (phase) => {
    for (const localRevert of [false, true]) {
      for (const savedTask of [false, true]) {
        const view = render(
          <Workflow
            {...props}
            state={{
              ...state,
              installation: {
                id: "job",
                phase,
                localRevert,
                journal: "private/journal.json",
                revisions: [revision],
                error: null,
              },
              task: savedTask
                ? {
                    kind: "apply",
                    selected: [],
                    manualSelection: false,
                    installationId: "job",
                  }
                : null,
            }}
          />,
        );
        expect(view.container.querySelector(".task-resume")).toBeNull();
        expect(screen.queryByRole("dialog")).toBeNull();
        view.unmount();
      }
    }
  },
);
it.each([false, true])(
  "hides a dismissed apply task and reopens through the normal action (previous installation: %s)",
  (previousInstallation) => {
    ipc.mockResolvedValue(null);
    const current: Snapshot = {
      ...state,
      sections: [{ ...state.sections[0], changed: false, installed: false }],
      installation: previousInstallation
        ? {
            id: "old-job",
            phase: "verified",
            journal: "private/journal.json",
            revisions: [revision],
            error: null,
          }
        : null,
      task: { kind: "apply", selected: [], manualSelection: false },
    };
    const view = render(<Workflow {...props} state={current} />);
    expect(view.container.querySelector(".task-resume")).toBeNull();
    expect(screen.queryByRole("button", { name: "Resume" })).toBeNull();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(ipc).not.toHaveBeenCalled();
    view.rerender(
      <Workflow
        {...props}
        state={current}
        request={{ kind: "apply", nonce: 1 }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "Review downloaded updates" }),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Continue later" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(view.container.querySelector(".task-resume")).toBeNull();
  },
);
it("keeps recovery available without a saved task", () => {
  ipc.mockResolvedValue(null);
  render(
    <Workflow
      {...props}
      state={{
        ...state,
        installation: {
          id: "job",
          phase: "awaitingReload",
          journal: "private/journal.json",
          revisions: [revision],
          error: "Copied files could not be verified",
        },
      }}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Resume" }));
  expect(
    screen.getByRole("heading", { name: "Applying was interrupted" }),
  ).toBeVisible();
});

it("closes recovery before opening History", () => {
  const history = vi.fn();
  render(
    <Workflow
      {...props}
      onHistory={history}
      state={{
        ...state,
        installation: {
          id: "broken",
          phase: "interrupted",
          journal: "private/journal.json",
          revisions: [revision],
          error: "Copy interrupted",
        },
      }}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Resume" }));
  fireEvent.click(screen.getByRole("button", { name: "Open History" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(history).toHaveBeenCalledOnce();
});

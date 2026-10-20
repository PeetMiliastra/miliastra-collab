// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { afterEach, it, expect, vi } from "vitest";
import { InstallFlow } from "./InstallFlow";
import { empty } from "./App";
import type { Revision, Snapshot, Installation } from "./types";
const native = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("./bridge", () => ({ desktop: true, call: native }));
afterEach(() => {
  cleanup();
  native.mockClear();
});
const revision: Revision = {
  section: "5",
  name: "Boats",
  hash: "a".repeat(64),
  contentHash: "b".repeat(64),
  parent: null,
  artifact: "History/11111111-1111-4111-8111-111111111111/6.gis",
  size: 100,
};
const run = async (_: string, action: () => Promise<unknown>) => {
  await action();
};
it("requires saved-and-left-stage confirmation before a real installation command", async () => {
  render(
    <InstallFlow
      state={empty}
      busy=""
      run={run}
      plan={{ id: "preview", revisions: [revision] }}
      onClose={() => {}}
    />,
  );
  const button = screen.getByRole("button", { name: "Back up and apply" });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(native).not.toHaveBeenCalled();
  fireEvent.click(
    screen.getByRole("checkbox", {
      name: "I saved my work and unloaded this stage in Miliastra.",
    }),
  );
  fireEvent.click(button);
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("install_updates", {
      planId: "preview",
      editorReady: true,
    }),
  );
});
it("offers a file-only retry for interrupted verification without a reload confirmation", async () => {
  const record: Installation = {
    id: "job",
    phase: "awaitingReload",
    journal: "backup/journal.json",
    revisions: [revision],
    error: null,
  };
  const state: Snapshot = { ...empty, installation: record };
  render(
    <InstallFlow
      state={state}
      busy=""
      run={run}
      plan={null}
      onClose={() => {}}
    />,
  );
  const button = screen.getByRole("button", {
    name: "Check files and finish",
  });
  expect((button as HTMLButtonElement).disabled).toBe(false);
  expect(screen.queryByRole("checkbox")).toBeNull();
  expect(native).not.toHaveBeenCalled();
  expect(screen.queryByText(/reopened|then finish here/i)).toBeNull();
  expect(screen.getByLabelText("Progress")).not.toHaveTextContent("Reopen");
  fireEvent.click(button);
  await waitFor(() => expect(native).toHaveBeenCalledWith("verify_install"));
});
it("recovers an interrupted Undo file check and prevents checks while busy", async () => {
  const state: Snapshot = {
    ...empty,
    installation: {
      id: "undo-job",
      phase: "undoReload",
      journal: "backup/journal.json",
      revisions: [revision],
      error: null,
    },
  };
  const props = { state, run, plan: null, onClose: () => {} };
  const view = render(<InstallFlow {...props} busy="Checking files" />);
  const button = screen.getByRole("button", {
    name: "Check files and finish",
  });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(button);
  expect(native).not.toHaveBeenCalled();
  view.rerender(<InstallFlow {...props} busy="" />);
  expect(screen.queryByRole("checkbox")).toBeNull();
  fireEvent.click(button);
  await waitFor(() => expect(native).toHaveBeenCalledWith("verify_install"));
});

it("finishes restored files without exposing Undo controls outside History", () => {
  render(
    <InstallFlow
      state={{
        ...empty,
        sections: [
          {
            id: "5",
            name: "Boats",
            changed: false,
            conflict: false,
            downloaded: true,
            installed: false,
            lock: null,
          },
        ],
        installation: {
          id: "job",
          phase: "undone",
          journal: "backup",
          revisions: [revision],
          error: null,
        },
      }}
      completion
      busy=""
      run={run}
      plan={null}
      onClose={() => {}}
    />,
  );
  expect(screen.getByRole("heading", { name: "Undo complete" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Done" })).toBeTruthy();
});

it("keeps the local Revert result without extra recovery controls", () => {
  render(
    <InstallFlow
      state={{
        ...empty,
        installation: {
          id: "revert",
          localRevert: true,
          phase: "verified",
          journal: "backup/journal.json",
          revisions: [revision],
          error: null,
        },
      }}
      busy=""
      run={run}
      plan={null}
      onClose={() => {}}
    />,
  );
  expect(screen.getByRole("dialog", { name: "Revert slice" })).toBeTruthy();
  expect(screen.getByText("Slice reverted")).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Undo/ })).toBeNull();
});

it("keeps installation failures and progress inside the active modal", async () => {
  const run: import("./types").Run = async (_, action, options) => {
    try {
      await action();
    } catch (e) {
      options?.onError?.(String(e));
    }
  };
  native.mockRejectedValueOnce(Error("The saved slice changed; review again"));
  const view = render(
    <InstallFlow
      state={empty}
      busy=""
      run={run}
      plan={{ id: "review", revisions: [revision] }}
      onClose={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Back up and apply" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("slice changed");
  view.rerender(
    <InstallFlow
      state={empty}
      busy="Backing up slices"
      run={run}
      plan={{ id: "review", revisions: [revision] }}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("Backing up slices");
  expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Continue later" })).toBeDisabled();
});

it("allows reviewing a downloaded conflict while offline", () => {
  const review = vi.fn();
  render(
    <InstallFlow
      state={{
        ...empty,
        online: false,
        sections: [
          {
            id: "5",
            name: "Boats",
            changed: true,
            conflict: true,
            lock: null,
            downloaded: true,
            installed: false,
          },
        ],
      }}
      busy=""
      run={run}
      plan={null}
      onClose={() => {}}
      onConflict={review}
    />,
  );
  const button = screen.getByRole("button", { name: "Review Boats" });
  expect(button).toBeEnabled();
  fireEvent.click(button);
  expect(review).toHaveBeenCalledWith("5");
});

it("finishes an installed update with a loading reminder and no Undo or technical details", () => {
  render(
    <InstallFlow
      state={{
        ...empty,
        installation: {
          id: "done",
          phase: "verified",
          journal: "backup",
          revisions: [revision],
          error: null,
        },
      }}
      busy=""
      run={run}
      plan={null}
      onClose={vi.fn()}
    />,
  );
  expect(
    screen.getByRole("heading", { name: "Updates applied" }),
  ).toBeVisible();
  expect(
    screen.getByText("Open the stage in Miliastra to load them."),
  ).toBeVisible();
  expect(screen.queryByText(/Undo/i)).toBeNull();
  expect(screen.queryByText("Installation details")).toBeNull();
  expect(screen.queryByText("backup")).toBeNull();
  expect(screen.getByRole("button", { name: "Done" })).toBeEnabled();
  expect(screen.getByLabelText("Progress")).not.toHaveTextContent("Reopen");
  expect(
    screen.queryByRole("button", { name: /reopened|Check files/i }),
  ).toBeNull();
  expect(native).not.toHaveBeenCalled();
});

it("routes interrupted-copy recovery to History without an Undo control", () => {
  const history = vi.fn();
  render(
    <InstallFlow
      state={{
        ...empty,
        installation: {
          id: "broken",
          phase: "interrupted",
          journal: "private/journal.json",
          revisions: [revision],
          error: "Copy interrupted",
        },
      }}
      busy=""
      run={run}
      plan={null}
      onClose={vi.fn()}
      onHistory={history}
    />,
  );
  expect(screen.queryByText(/Undo/i)).toBeNull();
  expect(screen.queryByText("private/journal.json")).toBeNull();
  expect(screen.getByRole("alert")).toHaveTextContent("Copy interrupted");
  fireEvent.click(screen.getByRole("button", { name: "Open History" }));
  expect(history).toHaveBeenCalledOnce();
});

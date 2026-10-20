// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
const ipc = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock("./bridge", () => ({ desktop: false, call: ipc.call }));
import { HistoryPage } from "./HistoryPage";
import { empty } from "./App";
import type { Snapshot, Update } from "./types";
afterEach(cleanup);
const state: Snapshot = {
  ...empty,
  member: "me",
  project: {
    id: "p",
    name: "Fisch",
    storage: { kind: "folder", location: "" },
    members: [{ id: "me", name: "Robin", owner: true }],
    heads: {},
    locks: [],
    sequence: 1,
  },
};
const update = (id: string, at: number): Update => ({
  id,
  summary: id,
  at,
  author: "Alex",
  authorId: "alex",
  sequence: 1,
  details: "Full changelog",
  revisions: [],
  restores: null,
});
const run = async (_: string, action: () => Promise<unknown>) => {
  await action();
};
it("groups paginated history by local date without duplicating the day heading and keeps full changelog details without correction controls", async () => {
  const today = new Date();
  today.setHours(12, 0, 0, 0);
  const at = today.getTime() / 1000;
  ipc.call.mockImplementation(async (cmd, args) =>
    cmd === "history_page"
      ? args.filter.before
        ? { entries: [update("Earlier today", at - 60)], next: null }
        : { entries: [update("Latest change", at)], next: 7 }
      : null,
  );
  render(<HistoryPage state={state} busy="" run={run} />);
  await screen.findByText("Latest change");
  expect(screen.getAllByRole("heading", { name: "Today" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Load earlier changes" }));
  await screen.findByText("Earlier today");
  expect(screen.getAllByRole("heading", { name: "Today" })).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: /Latest change/ }));
  expect(screen.getByRole("dialog", { name: "Latest change" })).toBeVisible();
  expect(screen.getByText("Full changelog")).toBeVisible();
  expect(screen.getByText(/^Update #1 by Alex, /)).toBeVisible();
  expect(screen.queryByText("Revision details")).toBeNull();
  expect(screen.queryByText(/Review and apply shows/)).toBeNull();
  expect(screen.queryByText("Add a correction")).toBeNull();
  expect(
    screen.queryByRole("button", { name: "Append correction" }),
  ).toBeNull();
});
it("rejects stale slice-filter responses and removes other filters and exports", async () => {
  let resolveOld!: (value: unknown) => void;
  ipc.call.mockImplementation(async (cmd, args) => {
    if (cmd !== "history_page") return null;
    if (args.filter.section === "old")
      return new Promise((resolve) => {
        resolveOld = resolve;
      });
    return {
      entries: [update(args.filter.section || "Initial", 1)],
      next: null,
    };
  });
  render(
    <HistoryPage
      state={{
        ...state,
        sections: ["old", "new"].map((id) => ({
          id,
          name: id,
          changed: false,
          conflict: false,
          lock: null,
          downloaded: true,
          installed: true,
        })),
      }}
      busy=""
      run={run}
    />,
  );
  await screen.findByText("Initial");
  fireEvent.change(screen.getByRole("combobox", { name: "Slice" }), {
    target: { value: "old" },
  });
  await waitFor(() => expect(resolveOld).toBeTypeOf("function"));
  fireEvent.change(screen.getByRole("combobox", { name: "Slice" }), {
    target: { value: "new" },
  });
  await screen.findByRole("heading", { name: "new" });
  resolveOld({ entries: [update("Wrong old result", 1)], next: null });
  await waitFor(() =>
    expect(screen.queryByText("Wrong old result")).toBeNull(),
  );
  expect(screen.queryByText("More filters")).toBeNull();
  expect(screen.queryByRole("textbox", { name: "Search history" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Export history" })).toBeNull();
  await waitFor(() =>
    expect(ipc.call).toHaveBeenCalledWith("history_page", {
      filter: { section: "new", before: null },
    }),
  );
});
it("restores an older slice version with a required reason, and explains when it can't", async () => {
  const old = {
    section: "5",
    name: "Boats",
    hash: "a".repeat(64),
    contentHash: "c".repeat(64),
    parent: null,
    artifact: "History/x/6.gis",
    size: 10,
  };
  const entry = {
    ...update("Older boats", 1792000000),
    sequence: 4,
    revisions: [old],
  };
  const current: Snapshot = {
    ...state,
    compatibility: { ...state.compatibility, restoreVersions: true },
    online: true,
    project: {
      ...state.project!,
      heads: { "5": { ...old, hash: "b".repeat(64) } },
    },
    sections: [
      {
        id: "5",
        name: "Boats",
        changed: false,
        conflict: false,
        lock: null,
        downloaded: true,
        installed: true,
      },
    ],
  };
  ipc.call.mockImplementation(async (cmd) =>
    cmd === "history_page" ? { entries: [entry], next: null } : null,
  );
  const view = render(<HistoryPage state={current} busy="" run={run} />);
  fireEvent.click(await screen.findByText("Older boats"));
  fireEvent.click(screen.getByRole("button", { name: "Restore this version" }));
  const dialog = screen.getByRole("dialog", { name: "Restore Boats?" });
  const confirm = () =>
    [...dialog.querySelectorAll("button")].find(
      (b) => b.textContent === "Restore this version",
    )!;
  expect(confirm()).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "The new layout broke docking" },
  });
  fireEvent.click(confirm());
  await waitFor(() =>
    expect(ipc.call).toHaveBeenCalledWith("restore_version", {
      updateId: entry.id,
      section: "5",
      reason: "The new layout broke docking",
    }),
  );
  // With unshared local changes, restoring is explained and disabled.
  view.rerender(
    <HistoryPage
      state={{
        ...current,
        sections: [{ ...current.sections[0], changed: true }],
      }}
      busy=""
      run={run}
    />,
  );
  fireEvent.click(await screen.findByText("Older boats"));
  fireEvent.click(screen.getByRole("button", { name: "Restore this version" }));
  expect(screen.getByRole("alert")).toHaveTextContent("unshared changes");
});

it("offers restoration of the initialized baseline only when a stage supports restore", async () => {
  const baseline = {
    ...update("Initialized project", 1),
    sequence: 1,
    revisions: [
      {
        section: "5",
        name: "Boats",
        hash: "a".repeat(64),
        contentHash: "c".repeat(64),
        parent: null,
        artifact: "History/baseline/6.gis",
        size: 10,
      },
    ],
  };
  ipc.call.mockImplementation(async (cmd) =>
    cmd === "history_page" ? { entries: [baseline], next: null } : null,
  );
  const ready = {
    ...state,
    online: true,
    compatibility: { ...state.compatibility, restoreVersions: true },
  };
  const view = render(<HistoryPage state={ready} busy="" run={run} />);
  fireEvent.click(await screen.findByText("Initialized project"));
  expect(
    screen.getByRole("button", { name: "Restore this version" }),
  ).toBeEnabled();
  view.rerender(
    <HistoryPage
      state={{
        ...ready,
        compatibility: { ...ready.compatibility, restoreVersions: false },
      }}
      busy=""
      run={run}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "Restore this version" }),
  ).toBeNull();
});

it("reads and paginates cached History offline without invoking a refresh command", async () => {
  ipc.call.mockClear();
  ipc.call.mockImplementation(async (cmd, args) => {
    if (cmd !== "history_page") throw Error("Unexpected online command");
    return {
      entries: [
        update(
          args.filter.before ? "Cached older update" : "Cached recent update",
          1,
        ),
      ],
      next: args.filter.before ? null : 3,
    };
  });
  render(<HistoryPage state={{ ...state, online: false }} busy="" run={run} />);
  await screen.findByText("Cached recent update");
  fireEvent.click(screen.getByRole("button", { name: "Load earlier changes" }));
  fireEvent.click(await screen.findByText("Cached older update"));
  expect(
    screen.getByRole("dialog", { name: "Cached older update" }),
  ).toBeVisible();
  expect(ipc.call.mock.calls.every(([cmd]) => cmd === "history_page")).toBe(
    true,
  );
});

it("keeps local installation recovery in History, even while offline", async () => {
  ipc.call.mockReset();
  const ipcRun: import("./types").Run = async (_, action, options) => {
    try {
      await action();
    } catch (error) {
      options?.onError?.(String(error));
    }
  };
  ipc.call.mockImplementation(async (cmd) => {
    if (cmd === "history_page") return { entries: [], next: null };
    return null;
  });
  render(
    <HistoryPage
      state={{
        ...state,
        online: false,
        installation: {
          id: "job",
          phase: "interrupted",
          journal: "private/journal.json",
          revisions: [],
          error: "Copy interrupted",
        },
      }}
      busy=""
      run={ipcRun}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Undo last update" }));
  expect(
    screen.getByRole("dialog", { name: "Undo last update" }),
  ).toBeVisible();
  const restore = screen.getByRole("button", {
    name: "Restore previous slices",
  });
  expect(restore).toBeDisabled();
  expect(ipc.call.mock.calls.some(([cmd]) => cmd === "undo_install")).toBe(
    false,
  );
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(restore);
  await screen.findByRole("heading", { name: "Undo complete" });
  expect(ipc.call).toHaveBeenCalledWith("undo_install", { editorReady: true });
  expect(screen.queryByText("private/journal.json")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.queryByRole("button", { name: "Undo last update" })).toBeNull();
});

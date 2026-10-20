// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { InstallationUndo } from "./InstallationUndo";
import type { Installation, Run } from "./types";
const native = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ call: native }));
afterEach(() => {
  cleanup();
  native.mockReset();
});
const record: Installation = {
  id: "job",
  phase: "verified",
  journal: "private/journal.json",
  revisions: [],
  error: null,
};
const run: Run = async (_, action, options) => {
  try {
    await action();
  } catch (error) {
    options?.onError?.(String(error));
  }
};
it("requires fresh unload confirmation each time the History action is opened", () => {
  render(<InstallationUndo record={record} busy="" run={run} />);
  fireEvent.click(screen.getByRole("button", { name: "Undo last update" }));
  expect(
    screen.getByRole("button", { name: "Restore previous slices" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  expect(
    screen.getByRole("button", { name: "Restore previous slices" }),
  ).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(native).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Undo last update" }));
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(
    screen.getByRole("button", { name: "Restore previous slices" }),
  ).toBeDisabled();
  expect(screen.queryByText(record.journal)).toBeNull();
});
it("keeps refused restores in the History dialog for retry without false completion", async () => {
  native
    .mockRejectedValueOnce(Error("Boats has new local edits"))
    .mockResolvedValueOnce(null);
  render(<InstallationUndo record={record} busy="" run={run} />);
  fireEvent.click(screen.getByRole("button", { name: "Undo last update" }));
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(
    screen.getByRole("button", { name: "Restore previous slices" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("new local edits");
  expect(screen.queryByRole("heading", { name: "Undo complete" })).toBeNull();
  const button = screen.getByRole("button", {
    name: "Restore previous slices",
  });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  await screen.findByRole("heading", { name: "Undo complete" });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(native).toHaveBeenCalledTimes(2);
  expect(native).toHaveBeenLastCalledWith("undo_install", {
    editorReady: true,
  });
});
it("blocks duplicate restores and dismissal while the local operation runs", async () => {
  let finish!: () => void;
  native.mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  render(<InstallationUndo record={record} busy="" run={run} />);
  fireEvent.click(screen.getByRole("button", { name: "Undo last update" }));
  fireEvent.click(screen.getByRole("checkbox"));
  const button = screen.getByRole("button", {
    name: "Restore previous slices",
  });
  fireEvent.click(button);
  expect(button).toBeDisabled();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();
  expect(screen.getByRole("checkbox")).toBeDisabled();
  fireEvent.click(button);
  expect(native).toHaveBeenCalledExactlyOnceWith("undo_install", {
    editorReady: true,
  });
  await act(async () => {
    finish();
  });
  await screen.findByRole("heading", { name: "Undo complete" });
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.queryByRole("button", { name: "Undo last update" })).toBeNull();
});
it("finishes interrupted restoration by checking files, without a second write", async () => {
  native.mockResolvedValue(null);
  const current = { ...record, phase: "undoReload" as const };
  const view = render(
    <InstallationUndo record={current} busy="Other operation" run={run} />,
  );
  expect(screen.getByRole("button", { name: "Finish undoing" })).toBeDisabled();
  view.rerender(<InstallationUndo record={current} busy="" run={run} />);
  fireEvent.click(screen.getByRole("button", { name: "Finish undoing" }));
  expect(screen.queryByRole("checkbox")).toBeNull();
  fireEvent.click(
    screen.getByRole("button", { name: "Check files and finish" }),
  );
  await screen.findByRole("heading", { name: "Undo complete" });
  expect(native).toHaveBeenCalledExactlyOnceWith("verify_install");
});
it("does not offer already restored installations again", () => {
  render(
    <InstallationUndo
      record={{ ...record, phase: "undone" }}
      busy=""
      run={run}
    />,
  );
  expect(screen.queryByRole("button")).toBeNull();
  expect(native).not.toHaveBeenCalled();
});

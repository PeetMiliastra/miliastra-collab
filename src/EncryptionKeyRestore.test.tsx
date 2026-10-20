// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { afterEach, beforeEach, it, expect, vi } from "vitest";
const ipc = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ call: ipc }));
import { EncryptionKeyRestore } from "./EncryptionKeyRestore";
beforeEach(() => {
  ipc.mockReset();
  ipc.mockResolvedValue(null);
});
afterEach(cleanup);
it("offers only local key restoration with an explanation and validation", async () => {
  render(<EncryptionKeyRestore />);
  expect(ipc).not.toHaveBeenCalled();
  expect(screen.getAllByRole("button")).toHaveLength(1);
  const restore = screen.getByRole("button", { name: "Restore" });
  expect(restore).toBeDisabled();
  expect(
    screen.getByText(/restore it from a teammate’s invitation message/),
  ).toBeVisible();
  fireEvent.change(screen.getByLabelText("Encryption Key"), {
    target: { value: "invalid" },
  });
  expect(restore).toBeDisabled();
  fireEvent.change(screen.getByLabelText("Encryption Key"), {
    target: { value: " mcollab-key-v2:synthetic " },
  });
  fireEvent.click(restore);
  await screen.findByText("Encryption Key restored on this device.");
  expect(ipc).toHaveBeenCalledWith("save_folder_privacy_key", {
    invitation: null,
    key: "mcollab-key-v2:synthetic",
  });
  expect(screen.getByLabelText("Encryption Key")).toHaveValue("");
});
it("preserves the entered key on failure and blocks duplicate restoration", async () => {
  let reject!: (e: Error) => void;
  ipc.mockImplementation(
    () =>
      new Promise((_, fail) => {
        reject = fail;
      }),
  );
  render(<EncryptionKeyRestore />);
  fireEvent.change(screen.getByLabelText("Encryption Key"), {
    target: { value: "mcollab-key-v2:synthetic" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Restore" }));
  expect(screen.getByRole("button", { name: "Restoring…" })).toBeDisabled();
  expect(screen.getByLabelText("Encryption Key")).toBeDisabled();
  reject(new Error("Could not restore this key"));
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent("Could not restore"),
  );
  expect(screen.getByLabelText("Encryption Key")).toHaveValue(
    "mcollab-key-v2:synthetic",
  );
  expect(screen.getByRole("button", { name: "Restore" })).toBeEnabled();
});

// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  within,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
vi.mock("./bridge", () => ({
  desktop: true,
  call: vi.fn().mockResolvedValue(null),
}));
import { TeamPanel, invitationMessage } from "./TeamPanel";
import { empty } from "./App";
import { call } from "./bridge";
import type { Run, Snapshot } from "./types";
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
const run: Run = async (_, action) => {
  await action();
};
const state = (me: string): Snapshot => ({
  ...empty,
  online: true,
  member: me,
  project: {
    id: "p",
    name: "Fisch",
    storage: { kind: "onedrive", location: "" },
    members: [
      { id: "a", name: "Alex", owner: true },
      { id: "b", name: "Robin", owner: false },
    ],
    locks: [],
    heads: {},
    sequence: 1,
  },
});

it("lets the owner remove creators and hand over ownership after confirming", () => {
  render(<TeamPanel state={state("a")} busy="" run={run} />);
  expect(screen.getByText("Alex (you)")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Remove" }));
  const remove = screen.getByRole("dialog", {
    name: "Remove Robin from Fisch?",
  });
  expect(remove).toHaveTextContent("stop sharing the team folder");
  expect(remove).toHaveTextContent("Encryption Key they already have");
  fireEvent.click(within(remove).getByRole("button", { name: "Remove" }));
  expect(call).toHaveBeenCalledWith("remove_member", { member: "b" });
  fireEvent.click(screen.getByRole("button", { name: "Make owner" }));
  const owner = screen.getByRole("dialog", { name: "Make Robin the owner?" });
  fireEvent.click(within(owner).getByRole("button", { name: "Make owner" }));
  expect(call).toHaveBeenCalledWith("transfer_ownership", { member: "b" });
  fireEvent.click(screen.getByRole("button", { name: "Remove" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(call).toHaveBeenCalledTimes(2);
});
it("keeps member controls without exposing pending invitations or cancellation", () => {
  render(<TeamPanel state={state("b")} busy="" run={run} />);
  expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Make owner" })).toBeNull();
  expect(
    screen.queryByRole("button", { name: /Revoke|Cancel invitation/ }),
  ).toBeNull();
  expect(
    screen.queryByText(/Open invitations|No open invitations|From Robin/),
  ).toBeNull();
});

it("includes the website and Encryption Key in the message while leaving the URL unchanged", () => {
  const link = "https://x.example/invite/1#2",
    key = "mcollab-key-v2:synthetic";
  const text = invitationMessage("Fisch", link, key);
  expect(text).toContain("“Fisch”");
  expect(text).toContain(link);
  expect(text).toContain("https://miliastra.dev");
  expect(text).toContain("Encryption Key: " + key);
  expect(text).not.toContain("separately");
  expect(link).not.toContain(key);
});

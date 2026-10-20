// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import {
  RecoveryNotice,
  ProjectConnectionNotice,
  storageRecovery,
} from "./RecoveryNotice";
import { empty } from "./App";
import type { Snapshot, Run } from "./types";
const native = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ call: native }));
afterEach(() => {
  cleanup();
  native.mockReset();
});
const state: Snapshot = {
  ...empty,
  online: true,
  project: {
    id: "p",
    name: "P",
    storage: { kind: "onedrive", location: "private" },
    members: [{ id: "owner", name: "Alex", owner: true }],
    heads: {},
    locks: [],
    sequence: 1,
  },
  storageAccounts: { onedrive: "connected" },
};
const run: Run = async (_, action) => {
  await action();
};
it.each([
  "access",
  "missing",
  "network",
  "busy",
  "validation",
  "unknown",
] as const)(
  "provides specific recovery for %s independently of coordinator status",
  (issue) => {
    const recovery = storageRecovery({ ...state, storageIssue: issue })!;
    expect(recovery.title).not.toMatch(/offline|project connection/i);
    expect(recovery.message.length).toBeGreaterThan(20);
    expect(
      storageRecovery({ ...state, online: false, storageIssue: issue }),
    ).toEqual(recovery);
    if (issue === "access") expect(recovery.message).toContain("Ask Alex");
    if (issue === "validation")
      expect(recovery.message).toContain("has not been applied");
  },
);
it("signs in for an expired account and acknowledges the successful connection", async () => {
  native.mockResolvedValue(undefined);
  render(
    <RecoveryNotice
      state={{ ...state, storageAccounts: { onedrive: "reconnect" } }}
      busy=""
      run={run}
      onAccounts={vi.fn()}
      onTeam={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Reconnect OneDrive" }));
  await screen.findByRole("dialog", { name: "Connected to OneDrive" });
  expect(native).toHaveBeenCalledWith("sign_in", { provider: "onedrive" });
  expect(native).toHaveBeenCalledWith("refresh");
  fireEvent.click(screen.getByRole("button", { name: "Done" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
it("takes permission issues to the team and lets the creator retry", async () => {
  const team = vi.fn();
  native.mockResolvedValue(undefined);
  render(
    <RecoveryNotice
      state={{ ...state, storageIssue: "access" }}
      busy=""
      run={run}
      onAccounts={vi.fn()}
      onTeam={team}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "View team" }));
  expect(team).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Retry download" }));
  await waitFor(() => expect(native).toHaveBeenCalledWith("refresh"));
});
it("shows metadata waiting separately from failures, and disables retry during another operation", () => {
  const view = render(
    <RecoveryNotice
      state={{ ...state, metadataPending: true }}
      busy="Checking updates"
      run={run}
      onAccounts={vi.fn()}
      onTeam={vi.fn()}
    />,
  );
  expect(screen.getByText("Waiting for update details")).toBeVisible();
  expect(
    screen.getByRole("button", { name: "Check for changes" }),
  ).toBeDisabled();
  view.rerender(
    <RecoveryNotice
      state={state}
      busy=""
      run={run}
      onAccounts={vi.fn()}
      onTeam={vi.fn()}
    />,
  );
  expect(screen.queryByRole("status")).toBeNull();
});

it("gives owners a direct folder check instead of asking them to contact themselves", () => {
  const ownerState = { ...state, member: "owner" };
  expect(
    storageRecovery({ ...ownerState, storageIssue: "missing" })!.message,
  ).toContain("Check the team folder");
  expect(
    storageRecovery({ ...ownerState, storageIssue: "missing" })!.message,
  ).not.toContain("Ask Alex");
  expect(
    storageRecovery({ ...ownerState, storageIssue: "access" })!.message,
  ).toContain("Check that your connected account");
});

it("retries the project connection and hides the warning for online, removed or unopened projects", async () => {
  native.mockResolvedValue(undefined);
  const props = { state: { ...state, online: false }, busy: "", run };
  const view = render(<ProjectConnectionNotice {...props} />);
  expect(
    screen.getByRole("status", { name: "Project connection" }),
  ).toHaveTextContent("Project connection unavailable");
  fireEvent.click(screen.getByRole("button", { name: "Retry connection" }));
  await waitFor(() => expect(native).toHaveBeenCalledWith("refresh"));
  view.rerender(<ProjectConnectionNotice {...props} busy="Reconnecting" />);
  expect(
    screen.getByRole("button", { name: "Retry connection" }),
  ).toBeDisabled();
  for (const next of [
    { ...state, online: true },
    { ...state, online: false, removed: true },
    { ...state, project: null },
  ]) {
    view.rerender(<ProjectConnectionNotice {...props} state={next} />);
    expect(screen.queryByRole("status")).toBeNull();
  }
});

it("distinguishes incompatible versions from an unreachable service and clears after reconnection", async () => {
  native.mockResolvedValue(undefined);
  const current = { ...state, online: false, protocolMismatch: true };
  const view = render(
    <ProjectConnectionNotice state={current} busy="" run={run} />,
  );
  const banner = screen.getByRole("status", { name: "Project connection" });
  expect(banner).toHaveTextContent("Update required");
  expect(banner).toHaveTextContent("Update miliastra/collab");
  expect(banner).toHaveTextContent(
    "ask the project owner to update the service",
  );
  expect(banner).not.toHaveTextContent("cannot reach");
  fireEvent.click(screen.getByRole("button", { name: "Retry connection" }));
  await waitFor(() => expect(native).toHaveBeenCalledWith("refresh"));
  view.rerender(
    <ProjectConnectionNotice
      state={{ ...current, online: true }}
      busy=""
      run={run}
    />,
  );
  expect(screen.getByText("Update required")).toBeVisible();
  view.rerender(
    <ProjectConnectionNotice
      state={{ ...current, online: true, protocolMismatch: false }}
      busy=""
      run={run}
    />,
  );
  expect(screen.queryByRole("status")).toBeNull();
  for (const next of [
    { ...current, project: null },
    { ...current, removed: true },
  ]) {
    view.rerender(<ProjectConnectionNotice state={next} busy="" run={run} />);
    expect(screen.queryByRole("status")).toBeNull();
  }
});

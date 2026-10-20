// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  act,
} from "@testing-library/react";
import { afterEach, it, expect, vi } from "vitest";
import { useState } from "react";
const ipc = vi.hoisted(() => ({
  call: vi.fn(),
  progress: undefined as
    | undefined
    | ((e: { payload: { message: string; percent: number | null } }) => void),
}));
vi.mock("./bridge", () => ({ desktop: true, call: ipc.call }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_: string, fn: typeof ipc.progress) => {
    ipc.progress = fn;
    return () => {};
  }),
}));
import {
  ApplicationUpdates,
  FIRST_UPDATE_CHECK_MS,
  UPDATE_CHECK_EVERY_MS,
  UpdateBanner,
  useAppUpdateCheck,
  type AppUpdate,
} from "./ApplicationUpdates";
import type { Run } from "./types";
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
function Harness() {
  const [busy, setBusy] = useState("");
  const run: Run = async (label, action, options) => {
    setBusy(label);
    try {
      await action();
    } catch (e) {
      options?.onError?.(String(e));
    } finally {
      setBusy("");
    }
  };
  return <ApplicationUpdates busy={busy} run={run} />;
}
async function check(canReplace = true) {
  ipc.call.mockResolvedValue({
    version: "0.2.0",
    notes: "New app",
    canReplace,
  });
  render(<Harness />);
  fireEvent.click(
    screen.getByRole("button", { name: "Check for an app update" }),
  );
  await screen.findByText("Version 0.2.0");
}
it("offers a portable restart, shows download progress and recovers from replacement errors", async () => {
  await check();
  let fail!: (e: Error) => void;
  ipc.call.mockImplementation(
    () =>
      new Promise((_, reject) => {
        fail = reject;
      }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Restart and update" }));
  expect(
    screen.getByRole("button", { name: "Restart and update" }),
  ).toBeDisabled();
  await act(async () =>
    ipc.progress?.({
      payload: { message: "Downloading the new app…", percent: 42 },
    }),
  );
  expect(screen.getByRole("status")).toHaveTextContent("42%");
  await act(async () => fail(new Error("The folder became read-only")));
  expect(await screen.findByRole("alert")).toHaveTextContent("read-only");
  expect(
    screen.getByRole("button", { name: "Download updated app" }),
  ).toBeEnabled();
  expect(ipc.call).toHaveBeenCalledWith("install_application_update");
  expect(screen.queryByText(/installer/i)).toBeNull();
});
it("offers a saved copy for read-only locations and treats a cancelled save as cancellation", async () => {
  await check(false);
  expect(
    screen.queryByRole("button", { name: "Restart and update" }),
  ).toBeNull();
  ipc.call.mockResolvedValueOnce(null);
  fireEvent.click(screen.getByRole("button", { name: "Download updated app" }));
  await act(async () => {});
  expect(
    screen.queryByText("Downloaded. Close this app and open the new copy."),
  ).toBeNull();
  ipc.call.mockResolvedValueOnce("C:/Downloads/miliastra-collab.exe");
  fireEvent.click(screen.getByRole("button", { name: "Download updated app" }));
  expect(
    await screen.findByText(
      "Downloaded. Close this app and open the new copy.",
    ),
  ).toBeVisible();
  expect(ipc.call).toHaveBeenCalledWith("download_application_update");
});
it("shows unconfigured/offline failures without claiming the app is up to date", async () => {
  ipc.call.mockRejectedValue(
    new Error("App updates are not configured for this build."),
  );
  render(<Harness />);
  fireEvent.click(
    screen.getByRole("button", { name: "Check for an app update" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("not configured");
  expect(screen.queryByText("You are up to date.")).toBeNull();
});

function Checker({ mismatch }: { mismatch: boolean }) {
  const update = useAppUpdateCheck(mismatch);
  return <p>{update ? "found " + update.version : "none"}</p>;
}
it("checks for a newer app after start, every few hours and at once on a version mismatch", async () => {
  vi.useFakeTimers();
  try {
    ipc.call.mockResolvedValue(null);
    const view = render(<Checker mismatch={false} />);
    expect(ipc.call).not.toHaveBeenCalled();
    await act(async () => vi.advanceTimersByTimeAsync(FIRST_UPDATE_CHECK_MS));
    expect(ipc.call).toHaveBeenCalledTimes(1);
    expect(ipc.call).toHaveBeenCalledWith("check_application_update");
    await act(async () => vi.advanceTimersByTimeAsync(UPDATE_CHECK_EVERY_MS));
    expect(ipc.call).toHaveBeenCalledTimes(2);
    ipc.call.mockResolvedValue({
      version: "0.2.0",
      notes: "",
      canReplace: true,
    });
    await act(async () => view.rerender(<Checker mismatch={true} />));
    expect(ipc.call).toHaveBeenCalledTimes(3);
    expect(screen.getByText("found 0.2.0")).toBeVisible();
  } finally {
    vi.useRealTimers();
  }
});
it("stays quiet when the check fails, for example without an update feed", async () => {
  vi.useFakeTimers();
  try {
    ipc.call.mockRejectedValue(
      "App updates are not configured for this build.",
    );
    render(<Checker mismatch={true} />);
    await act(async () => vi.advanceTimersByTimeAsync(FIRST_UPDATE_CHECK_MS));
    expect(screen.getByText("none")).toBeVisible();
  } finally {
    vi.useRealTimers();
  }
});
function Banner({
  update,
  required = false,
  onLater = () => {},
}: {
  update: AppUpdate;
  required?: boolean;
  onLater?: () => void;
}) {
  const run: Run = async (_label, action) => {
    await action();
  };
  return (
    <UpdateBanner
      update={update}
      required={required}
      busy=""
      run={run}
      onLater={onLater}
    />
  );
}
it("offers a one-click update that can wait unless the service requires it", async () => {
  const later = vi.fn();
  const update = { version: "0.2.0", notes: "", canReplace: true };
  const view = render(<Banner update={update} onLater={later} />);
  expect(screen.getByText("Update available")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Later" }));
  expect(later).toHaveBeenCalledOnce();
  ipc.call.mockResolvedValue(undefined);
  await act(async () =>
    fireEvent.click(screen.getByRole("button", { name: "Restart and update" })),
  );
  expect(ipc.call).toHaveBeenCalledWith("install_application_update");
  view.rerender(<Banner update={update} required />);
  expect(screen.getByText("Update required")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Later" })).toBeNull();
});
it("saves a copy elsewhere when the app's folder is read-only", async () => {
  ipc.call.mockResolvedValue("C:/Users/me/Downloads/miliastra-collab.exe");
  render(
    <Banner update={{ version: "0.2.0", notes: "", canReplace: false }} />,
  );
  await act(async () =>
    fireEvent.click(
      screen.getByRole("button", { name: "Download updated app" }),
    ),
  );
  expect(ipc.call).toHaveBeenCalledWith("download_application_update");
  expect(
    screen.getByText("Downloaded. Close this app and open the new copy."),
  ).toBeVisible();
});

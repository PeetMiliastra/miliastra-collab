// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
const ipc = vi.hoisted(() => ({ call: vi.fn(async () => {}) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("./bridge", () => ({ desktop: true, call: ipc.call }));
vi.mock("../src-tauri/src/providers/catalog.json", () => ({
  default: [
    {
      id: "test-cloud",
      name: "Test Cloud",
      cloud: true,
      enabled: true,
      setupMessage: "Creating Test Cloud folder…",
    },
  ],
}));
import { StorageConnection } from "./StorageConnection";
import { Setup } from "./Setup";
afterEach(cleanup);

it("connects an additional catalog provider without a component-specific branch", async () => {
  const connected = vi.fn();
  render(
    <StorageConnection
      provider="test-cloud"
      status="notConnected"
      run={async (_, action) => {
        await action();
      }}
      busy=""
      onConnected={connected}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Connect Test Cloud" }));
  await waitFor(() =>
    expect(ipc.call).toHaveBeenCalledWith("sign_in", {
      provider: "test-cloud",
    }),
  );
  expect(connected).toHaveBeenCalledWith("Test Cloud");
});

it("offers additional providers and their connection controls through the ordinary setup flow", async () => {
  ipc.call.mockImplementation(async () => {});
  render(
    <Setup
      providers={["test-cloud"]}
      storageAccounts={{ "test-cloud": "connected" }}
      busy=""
      run={async (_, action) => {
        await action();
      }}
      onClose={() => {}}
      onDone={() => {}}
    />,
  );
  expect(screen.getByRole("option", { name: "Test Cloud" })).toHaveValue(
    "test-cloud",
  );
  fireEvent.change(
    screen.getByRole("combobox", { name: "Keep the team folder in" }),
    {
      target: { value: "test-cloud" },
    },
  );
  expect(
    screen.getByRole("group", { name: "Test Cloud account" }),
  ).toHaveTextContent("Connected");
});

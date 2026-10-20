// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { SliceName } from "./SectionNames";
import { empty } from "./App";
import type { Snapshot, Run } from "./types";
const native = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ desktop: true, call: native }));
afterEach(() => {
  cleanup();
  native.mockReset();
});
const section = {
  id: "5",
  name: "Boats",
  changed: false,
  conflict: false,
  lock: null,
  downloaded: true,
  installed: true,
};
const state: Snapshot = {
  ...empty,
  online: true,
  member: "me",
  project: {
    id: "p",
    name: "P",
    storage: { kind: "folder", location: "" },
    members: [{ id: "me", name: "Owner", owner: true }],
    heads: {},
    locks: [],
    sequence: 1,
    namesVersion: 0,
    sectionNames: { "5": "Boats" },
  },
  sections: [section],
};
const run: Run = async (_, action, options) => {
  try {
    await action();
  } catch (e) {
    options?.onError?.(String(e));
  }
};
const props = { state, section, busy: false, run };
it("renames directly in the row and saves only that name with its version", async () => {
  native.mockResolvedValue(null);
  render(<SliceName {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  const input = screen.getByRole("textbox", { name: "Name for Boats" });
  fireEvent.change(input, { target: { value: "Ships" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("share_section_names", {
      names: { "5": "Ships" },
      parentVersion: 0,
    }),
  );
  expect(native.mock.calls[0][0]).toBe("save_names_draft");
  expect(screen.queryByRole("dialog")).toBeNull();
});
it("keeps failed edits inline with a visible error and retry", async () => {
  native.mockImplementation(async (cmd) => {
    if (cmd === "share_section_names") throw Error("Storage unavailable");
  });
  render(<SliceName {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Ships" } });
  fireEvent.click(screen.getByRole("button", { name: "Save name for Boats" }));
  await screen.findByRole("alert");
  expect(screen.getByRole("textbox")).toHaveValue("Ships");
  expect(
    screen.getByRole("button", { name: "Save name for Boats" }),
  ).toBeEnabled();
});
it("preserves recovered drafts and prevents stale edits overwriting newer project names", () => {
  const restored = {
    ...state,
    nameSync: {
      pending: [],
      draft: {
        edits: { "5": { name: "Ships", baseName: "Boats", parentVersion: 0 } },
      },
    },
  };
  const view = render(<SliceName {...props} state={restored} />);
  expect(screen.getByRole("textbox")).toHaveValue("Ships");
  view.rerender(
    <SliceName
      {...props}
      state={{
        ...restored,
        project: {
          ...state.project!,
          namesVersion: 1,
          sectionNames: { "5": "Harbour" },
        },
      }}
      section={{ ...section, name: "Harbour" }}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Save name for Harbour" }),
  ).toBeDisabled();
  expect(screen.getByRole("alert")).toHaveTextContent("Slice names changed");
  fireEvent.click(screen.getByRole("button", { name: "Use current name" }));
  expect(screen.getByRole("textbox")).toHaveValue("Harbour");
});
it("rejects blank/control-character/oversize names and leaves the shared row read-only", () => {
  const view = render(<SliceName {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  for (const value of [" ", "x".repeat(257)]) {
    fireEvent.change(screen.getByRole("textbox"), { target: { value } });
    expect(
      screen.getByRole("button", { name: "Save name for Boats" }),
    ).toBeDisabled();
  }
  view.rerender(
    <SliceName
      {...props}
      section={{ ...section, id: "shared", name: "Shared level settings" }}
    />,
  );
  expect(screen.queryByRole("button")).toBeNull();
});
it("cancels without publishing and shows no separate names modal", () => {
  native.mockResolvedValue(null);
  render(<SliceName {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Ships" } });
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
  expect(screen.getByRole("button", { name: "Boats" })).toBeVisible();
  expect(native.mock.calls.some(([cmd]) => cmd === "share_section_names")).toBe(
    false,
  );
});
it("lets every member rename a slice, not only the owner", () => {
  const member = {
    ...state,
    project: {
      ...state.project!,
      members: [
        { id: "owner", name: "Owner", owner: true },
        { id: "me", name: "Robin", owner: false },
      ],
    },
  };
  render(<SliceName {...props} state={member} />);
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  expect(screen.getByRole("textbox", { name: "Name for Boats" })).toBeVisible();
});

it("edits and saves a name draft offline, then shares it only when confirmed online", async () => {
  native.mockResolvedValue(null);
  const view = render(
    <SliceName {...props} state={{ ...state, online: false }} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Boats" }));
  fireEvent.change(screen.getByRole("textbox"), { target: { value: "Ships" } });
  fireEvent.click(screen.getByRole("button", { name: "Save name for Boats" }));
  await screen.findByRole("button", { name: "Ships" });
  expect(screen.getByText("Draft")).toBeVisible();
  expect(native).toHaveBeenLastCalledWith("save_names_draft", {
    projectId: "p",
    draft: {
      edits: { "5": { name: "Ships", baseName: "Boats", parentVersion: 0 } },
    },
  });
  expect(native.mock.calls.some(([cmd]) => cmd === "share_section_names")).toBe(
    false,
  );
  view.rerender(<SliceName {...props} state={state} />);
  expect(native.mock.calls.some(([cmd]) => cmd === "share_section_names")).toBe(
    false,
  );
  fireEvent.click(screen.getByRole("button", { name: "Ships" }));
  expect(screen.getByRole("textbox")).toHaveValue("Ships");
  fireEvent.click(screen.getByRole("button", { name: "Save name for Boats" }));
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("share_section_names", {
      names: { "5": "Ships" },
      parentVersion: 0,
    }),
  );
});
it("recovers offline drafts with independent baselines and keeps newer row drafts editable", () => {
  const restored = {
    ...state,
    online: false,
    project: { ...state.project!, namesVersion: 2 },
    nameSync: {
      pending: [],
      draft: {
        edits: {
          "5": { name: "Ships", baseName: "Boats", parentVersion: 1 },
          "6": { name: "Reef", baseName: "Islands", parentVersion: 2 },
        },
      },
    },
  };
  const view = render(<SliceName {...props} state={restored} />);
  expect(screen.getByRole("textbox")).toHaveValue("Ships");
  expect(
    screen.getByRole("button", { name: "Save name for Boats" }),
  ).toBeDisabled();
  view.unmount();
  render(
    <SliceName
      {...props}
      state={restored}
      section={{ ...section, id: "6", name: "Islands" }}
    />,
  );
  expect(screen.getByRole("textbox")).toHaveValue("Reef");
  expect(
    screen.getByRole("button", { name: "Save name for Islands" }),
  ).toBeEnabled();
});

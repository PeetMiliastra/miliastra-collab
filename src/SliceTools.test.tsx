// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { SliceChanges, RevertSlice, MoveAsset } from "./SliceTools";
import { empty } from "./App";
import type { Run, Section, SliceInspection, MoveId } from "./types";
const native = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ desktop: true, call: native }));
afterEach(() => {
  cleanup();
  native.mockReset();
});
const section: Section = {
  id: "5",
  name: "Boats",
  changed: true,
  conflict: false,
  installed: true,
  downloaded: true,
  lock: null,
};
const asset = {
  key: "Entity:1078263809",
  id: 1078263809,
  name: "Boat deck",
  kind: "Entity",
  section: 5,
  canGenerateId: true,
};
const inspection: SliceInspection = {
  section: "5",
  assets: [asset],
  changes: [{ asset, status: "Modified", beforeName: "Old deck" }],
  otherChanges: true,
  baselineAvailable: true,
};
const run: Run = async (_, action, options) => {
  try {
    await action();
  } catch (e) {
    options?.onError?.(String(e));
  }
};
it("shows named asset changes, renames, and an honest remainder for unsupported data", async () => {
  native.mockResolvedValue(inspection);
  render(<SliceChanges section={section} onClose={vi.fn()} />);
  expect(await screen.findByText("Boat deck")).toBeVisible();
  expect(screen.getByText("Modified")).toBeVisible();
  expect(screen.getByText("Previously Old deck")).toBeVisible();
  expect(screen.getByText(/Other unidentified changes/)).toBeVisible();
  expect(native).toHaveBeenCalledWith("inspect_slice", { section: "5" });
});
it("reports missing baselines and read errors instead of invented asset changes", async () => {
  native.mockResolvedValue({
    ...inspection,
    changes: [],
    baselineAvailable: false,
  });
  const view = render(<SliceChanges section={section} onClose={vi.fn()} />);
  expect(
    await screen.findByText(/previous version is not available/),
  ).toBeVisible();
  expect(screen.queryByText("Modified")).toBeNull();
  view.unmount();
  native.mockRejectedValue(Error("Still saving"));
  render(<SliceChanges section={section} onClose={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Still saving");
});
it("waits for explicit Revert confirmation before doing any preparation", () => {
  native.mockResolvedValue({ id: "preview", section: "5", name: "Boats" });
  const close = vi.fn();
  render(
    <RevertSlice
      section={section}
      busy={false}
      run={run}
      onClose={close}
      onReverted={vi.fn()}
    />,
  );
  expect(
    screen.getByText(
      "All edits in this slice will be replaced with the last-shared version.",
    ),
  ).toBeVisible();
  const button = screen.getByRole("button", { name: "Back up and revert" });
  expect(button).toBeDisabled();
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
  expect(native).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("checkbox"));
  expect(button).toBeEnabled();
  expect(native).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(close).toHaveBeenCalledOnce();
  expect(native).not.toHaveBeenCalled();
});
it("reverts only after confirmation and keeps stale-preview errors inside the modal", async () => {
  let attempts = 0;
  let fail = true;
  native.mockImplementation(async (command) => {
    if (command === "preview_revert")
      return { id: `preview-${++attempts}`, section: "5", name: "Boats" };
    if (fail) throw Error("This slice changed after confirmation opened");
  });
  const done = vi.fn();
  render(
    <RevertSlice
      section={section}
      busy={false}
      run={run}
      onClose={vi.fn()}
      onReverted={done}
    />,
  );
  expect(native).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("checkbox"));
  const button = screen.getByRole("button", { name: "Back up and revert" });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  expect(await screen.findByRole("alert")).toHaveTextContent("slice changed");
  expect(done).not.toHaveBeenCalled();
  await waitFor(() => expect(button).toBeEnabled());
  fail = false;
  fireEvent.click(button);
  await waitFor(() => expect(done).toHaveBeenCalledOnce());
  expect(attempts).toBe(2);
  expect(native).toHaveBeenNthCalledWith(2, "revert_slice", {
    previewId: "preview-1",
    editorReady: true,
  });
  expect(native).toHaveBeenNthCalledWith(4, "revert_slice", {
    previewId: "preview-2",
    editorReady: true,
  });
});
it("shows preparation errors only after Revert is pressed and allows retry", async () => {
  native.mockRejectedValueOnce(Error("Restore copy failed validation"));
  const done = vi.fn();
  render(
    <RevertSlice
      section={section}
      busy={false}
      run={run}
      onClose={vi.fn()}
      onReverted={done}
    />,
  );
  expect(native).not.toHaveBeenCalled();
  expect(screen.queryByRole("alert")).toBeNull();
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Back up and revert" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Restore copy failed validation",
  );
  expect(native).toHaveBeenCalledExactlyOnceWith("preview_revert", {
    section: "5",
  });
  expect(done).not.toHaveBeenCalled();
  native
    .mockResolvedValueOnce({ id: "retry", section: "5", name: "Boats" })
    .mockResolvedValueOnce(undefined);
  const button = screen.getByRole("button", { name: "Back up and revert" });
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  await waitFor(() => expect(done).toHaveBeenCalledOnce());
  expect(native).toHaveBeenLastCalledWith("revert_slice", {
    previewId: "retry",
    editorReady: true,
  });
  expect(screen.queryByRole("alert")).toBeNull();
});
it("blocks duplicate Revert and dismissal while the restore copy is being checked", async () => {
  let finish!: (value: unknown) => void;
  native
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValue(undefined);
  const close = vi.fn(),
    done = vi.fn();
  render(
    <RevertSlice
      section={section}
      busy={false}
      run={run}
      onClose={close}
      onReverted={done}
    />,
  );
  const checkbox = screen.getByRole("checkbox");
  fireEvent.click(checkbox);
  const button = screen.getByRole("button", { name: "Back up and revert" });
  fireEvent.click(button);
  expect(screen.getByRole("button", { name: "Reverting…" })).toBeDisabled();
  expect(checkbox).toBeDisabled();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  fireEvent.click(button);
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(native).toHaveBeenCalledExactlyOnceWith("preview_revert", {
    section: "5",
  });
  expect(close).not.toHaveBeenCalled();
  await act(async () => {
    finish({ id: "preview", section: "5", name: "Boats" });
  });
  await waitFor(() => expect(done).toHaveBeenCalledOnce());
  expect(native).toHaveBeenCalledTimes(2);
  expect(native).toHaveBeenLastCalledWith("revert_slice", {
    previewId: "preview",
    editorReady: true,
  });
});
const generated: MoveId = {
  token: "token",
  oldId: asset.id,
  newId: 1078329345,
  name: asset.name,
  source: "5",
  target: "6",
  expires: 9999999999,
};
function renderMove(online = true) {
  return render(
    <MoveAsset
      state={{
        ...empty,
        online,
        sections: [section, { ...section, id: "6", name: "Islands" }],
      }}
      busy={false}
      run={run}
      onClose={vi.fn()}
    />,
  );
}
it("selects real assets and a distinct destination, then revalidates before copying", async () => {
  const copy = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: copy },
  });
  native.mockImplementation(async (command) =>
    command === "inspect_slice"
      ? inspection
      : command === "generate_move_id"
        ? generated
        : String(generated.newId),
  );
  renderMove();
  expect(
    screen.getByText(/Changing the GUID breaks any references/),
  ).toBeVisible();
  const generate = screen.getByRole("button", { name: "Generate GUID" });
  expect(generate).toBeDisabled();
  expect(
    await screen.findByRole("button", { name: /^Boat deck —/ }),
  ).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.click(generate);
  expect(await screen.findByLabelText("New GUID")).toHaveValue(
    String(generated.newId),
  );
  expect(native).toHaveBeenCalledWith("generate_move_id", {
    source: "5",
    assetKey: asset.key,
    target: "6",
  });
  fireEvent.click(screen.getByRole("button", { name: "Copy GUID" }));
  await waitFor(() =>
    expect(copy).toHaveBeenCalledWith(String(generated.newId)),
  );
  expect(native).toHaveBeenLastCalledWith("validate_move_id", {
    token: "token",
  });
});
it("generates locally while offline without reservation or reconnect steps", async () => {
  native.mockImplementation(async (command) =>
    command === "inspect_slice" ? inspection : generated,
  );
  const copy = vi.fn(async () => {});
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: copy },
  });
  native.mockImplementation(async (command) =>
    command === "inspect_slice"
      ? inspection
      : command === "generate_move_id"
        ? generated
        : String(generated.newId),
  );
  renderMove(false);
  expect(
    screen.queryByText(/reconnect|reserved|Incoming updates|expires/i),
  ).toBeNull();
  await screen.findByRole("button", { name: /^Boat deck —/ });
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  const button = screen.getByRole("button", { name: "Generate GUID" });
  expect(button).toBeEnabled();
  fireEvent.click(button);
  await screen.findByLabelText("New GUID");
  expect(native).toHaveBeenCalledWith("generate_move_id", {
    source: "5",
    assetKey: asset.key,
    target: "6",
  });
  fireEvent.click(screen.getByRole("button", { name: "Copy GUID" }));
  await waitFor(() =>
    expect(copy).toHaveBeenCalledWith(String(generated.newId)),
  );
  expect(
    native.mock.calls.every(([cmd]) =>
      ["inspect_slice", "generate_move_id", "validate_move_id"].includes(cmd),
    ),
  ).toBe(true);
});
it("shows local validation failures and allows retry without losing selections", async () => {
  let attempts = 0;
  native.mockImplementation(async (command) => {
    if (command === "inspect_slice") return inspection;
    if (command === "generate_move_id" && ++attempts === 1)
      throw Error("This ID is already in use");
    return generated;
  });
  renderMove(false);
  await screen.findByRole("button", { name: /^Boat deck —/ });
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("already in use");
  expect(screen.getByRole("button", { name: /^Boat deck —/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByLabelText("To slice")).toHaveValue("6");
  expect(screen.queryByLabelText("New GUID")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  await screen.findByLabelText("New GUID");
  expect(screen.queryByRole("alert")).toBeNull();
});
it.each(["Prefab", "Entity", "Camera", "Node graph", "Player Template"])(
  "generates IDs for %s without claiming the editor supports editing it",
  async (kind) => {
    const selected = {
      ...asset,
      key: `${kind}:${asset.id}`,
      kind,
      name: `${kind} asset`,
    };
    native.mockImplementation(async (command) =>
      command === "inspect_slice"
        ? { ...inspection, assets: [selected] }
        : { ...generated, name: selected.name },
    );
    renderMove();
    expect(
      screen.getByText(
        "This generates a new GUID to move your asset to another slice.",
      ),
    ).toBeVisible();
    const option = await screen.findByRole("button", {
      name: new RegExp(`^${kind} asset —`),
    });
    expect(option).toBeEnabled();
    fireEvent.click(option);
    fireEvent.change(screen.getByLabelText("To slice"), {
      target: { value: "6" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
    await screen.findByLabelText("New GUID");
    expect(native).toHaveBeenCalledWith("generate_move_id", {
      source: "5",
      assetKey: selected.key,
      target: "6",
    });
    expect(screen.queryByText(/ID editing unavailable/)).toBeNull();
  },
);
it("does not copy stale IDs and disables unrecognized ID layouts", async () => {
  const copy = vi.fn();
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: copy },
  });
  native.mockImplementation(async (command) => {
    if (command === "inspect_slice")
      return {
        ...inspection,
        assets: [
          asset,
          {
            ...asset,
            key: "Node graph:1610612737",
            id: 1610612737,
            kind: "Node graph",
            name: "Stage graph",
            canGenerateId: false,
          },
        ],
      };
    if (command === "generate_move_id") return generated;
    throw Error("One of these slices changed");
  });
  renderMove();
  expect(
    await screen.findByRole("button", { name: /^Boat deck —/ }),
  ).toBeEnabled();
  expect(
    screen.getByRole("button", { name: /Stage graph.*Unrecognized ID layout/ }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  fireEvent.click(await screen.findByRole("button", { name: "Copy GUID" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("slices changed");
  expect(copy).not.toHaveBeenCalled();
});

it("shows asset changes without movement actions", async () => {
  native.mockResolvedValue(inspection);
  render(<SliceChanges section={section} onClose={vi.fn()} />);
  await screen.findByText("Boat deck");
  expect(screen.queryByRole("button", { name: /Move|GUID/ })).toBeNull();
});
it("explains how to apply the generated GUID without editing the stage", async () => {
  native.mockImplementation(async (command) =>
    command === "inspect_slice" ? inspection : generated,
  );
  renderMove();
  await screen.findByRole("button", { name: /^Boat deck —/ });
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  await screen.findByLabelText("New GUID");
  expect(
    screen.getByText(
      "In Miliastra, change the GUID for Boat deck to this and save the stage.",
    ),
  ).toBeVisible();
  expect(
    native.mock.calls.every(([cmd]) =>
      ["inspect_slice", "generate_move_id"].includes(cmd),
    ),
  ).toBe(true);
});

it("groups assets by type, filters by category, and searches names and GUIDs", async () => {
  const prefab = {
    ...asset,
    key: "Prefab:1078263810",
    id: 1078263810,
    name: "Fishing boat",
    kind: "Prefab",
  };
  const unknown = {
    ...asset,
    key: "Future type:1078263811",
    id: 1078263811,
    name: "New asset type",
    kind: "Future type",
  };
  native.mockResolvedValue({ ...inspection, assets: [asset, prefab, unknown] });
  renderMove();
  await screen.findByRole("button", { name: /^Fishing boat —/ });
  expect(
    screen.getByRole("navigation", { name: "Asset categories" }),
  ).toBeVisible();
  expect(screen.getByRole("region", { name: "Prefabs" })).toBeVisible();
  expect(
    screen.getByRole("region", { name: "In-world objects" }),
  ).toBeVisible();
  expect(
    screen.getByRole("button", { name: /^New asset type —/ }),
  ).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Prefabs" }));
  expect(screen.queryByRole("button", { name: /^Boat deck —/ })).toBeNull();
  expect(screen.getByRole("button", { name: /^Fishing boat —/ })).toBeVisible();
  fireEvent.change(screen.getByRole("searchbox", { name: "Search assets" }), {
    target: { value: "  FISHING  " },
  });
  expect(screen.getByRole("button", { name: /^Fishing boat —/ })).toBeVisible();
  fireEvent.change(screen.getByRole("searchbox", { name: "Search assets" }), {
    target: { value: "missing" },
  });
  expect(screen.getByText("No assets match your search.")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "All assets" }));
  fireEvent.change(screen.getByRole("searchbox", { name: "Search assets" }), {
    target: { value: String(unknown.id) },
  });
  expect(
    screen.getByRole("button", { name: /^New asset type —/ }),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: /^Fishing boat —/ })).toBeNull();
});

it("clears generated IDs when browsing another category or searching", async () => {
  native.mockImplementation(async (command) =>
    command === "inspect_slice" ? inspection : generated,
  );
  renderMove();
  fireEvent.click(await screen.findByRole("button", { name: /^Boat deck —/ }));
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  await screen.findByLabelText("New GUID");
  fireEvent.click(screen.getByRole("button", { name: "In-world objects" }));
  expect(screen.queryByLabelText("New GUID")).toBeNull();
  expect(screen.getByRole("button", { name: "Generate GUID" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: /^Boat deck —/ }));
  fireEvent.click(screen.getByRole("button", { name: "Generate GUID" }));
  await screen.findByLabelText("New GUID");
  fireEvent.change(screen.getByRole("searchbox", { name: "Search assets" }), {
    target: { value: "deck" },
  });
  expect(screen.queryByLabelText("New GUID")).toBeNull();
  expect(screen.getByRole("button", { name: /^Boat deck —/ })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
});

it("discards a late inspection and clears the destination when switching to that slice", async () => {
  let finishFirst!: (value: SliceInspection) => void;
  native.mockImplementation((command, args) => {
    if (command === "inspect_slice" && args.section === "5")
      return new Promise<SliceInspection>((resolve) => {
        finishFirst = resolve;
      });
    return Promise.resolve({
      ...inspection,
      section: "6",
      assets: [{ ...asset, section: 6, name: "Island pier" }],
    });
  });
  renderMove();
  fireEvent.change(screen.getByLabelText("To slice"), {
    target: { value: "6" },
  });
  fireEvent.change(screen.getByLabelText("From slice"), {
    target: { value: "6" },
  });
  await screen.findByRole("button", { name: /^Island pier —/ });
  finishFirst(inspection);
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: /^Boat deck —/ })).toBeNull(),
  );
  expect(screen.getByLabelText("To slice")).toHaveValue("");
  expect(screen.getByRole("searchbox", { name: "Search assets" })).toHaveValue(
    "",
  );
  expect(screen.getByRole("button", { name: "All assets" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByRole("button", { name: "Generate GUID" })).toBeDisabled();
});

it("keeps busy controls disabled and distinguishes an empty saved slice from loading", async () => {
  native.mockResolvedValue({ ...inspection, assets: [] });
  const view = renderMove();
  expect(screen.getByText("Reading saved assets…")).toBeVisible();
  expect(await screen.findByText(/No named assets were found/)).toBeVisible();
  expect(screen.queryByText("Reading saved assets…")).toBeNull();
  view.rerender(
    <MoveAsset
      state={{
        ...empty,
        sections: [section, { ...section, id: "6", name: "Islands" }],
      }}
      busy={true}
      run={run}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByLabelText("From slice")).toBeDisabled();
  expect(screen.getByLabelText("To slice")).toBeDisabled();
  expect(
    screen.getByRole("searchbox", { name: "Search assets" }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: "All assets" })).toBeDisabled();
});

it.each([
  "Creation",
  "Loot",
  "Class",
  "Growth Curve",
  "Inventory Template",
  "Attribute Group",
  "Character Skill",
  "Unit Status",
  "Projectile",
  "Item",
  "VFX Tool",
  "Character Control Skill",
  "Shield",
  "Scan Tag Template",
  "Environment",
])(
  "shows %s assets in their own category with local ID generation available",
  async (kind) => {
    const current = { ...asset, kind, name: "Renamed asset" };
    native.mockResolvedValue({ ...inspection, assets: [current] });
    renderMove(false);
    const label = kind === "Creation" ? "Creations" : kind;
    const category = await screen.findByRole("button", { name: label });
    fireEvent.click(category);
    expect(screen.getByRole("region", { name: label })).toBeVisible();
    fireEvent.change(screen.getByRole("searchbox", { name: "Search assets" }), {
      target: { value: label },
    });
    fireEvent.click(screen.getByRole("button", { name: /^Renamed asset —/ }));
    fireEvent.change(screen.getByLabelText("To slice"), {
      target: { value: "6" },
    });
    expect(screen.getByRole("button", { name: "Generate GUID" })).toBeEnabled();
  },
);
it("places unclassified configurations in Other", async () => {
  native.mockResolvedValue({
    ...inspection,
    assets: [
      { ...asset, kind: "Configuration", name: "Unknown configuration" },
    ],
  });
  renderMove();
  fireEvent.click(await screen.findByRole("button", { name: "Other" }));
  expect(screen.getByRole("region", { name: "Other" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Configurations" })).toBeNull();
  expect(
    screen.getByRole("button", { name: /^Unknown configuration —/ }),
  ).toBeEnabled();
});

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
import { ConflictPanel } from "./ConflictPanel";
import type { ConflictPreview, Revision } from "./types";
const native = vi.hoisted(() =>
  vi.fn(async (_cmd: string, _args?: Record<string, unknown>) => undefined),
);
vi.mock("./bridge", () => ({ desktop: true, call: native }));
afterEach(() => {
  cleanup();
  native.mockClear();
});
const revision: Revision = {
  section: "8",
  name: "Quests",
  hash: "a".repeat(64),
  contentHash: "b".repeat(64),
  parent: null,
  artifact: "History/11111111-1111-4111-8111-111111111111/6.gis",
  size: 1024,
};
const preview: ConflictPreview = {
  id: "review",
  project: "p",
  section: "8",
  name: "Quests",
  mine: revision,
  theirs: { ...revision, hash: "c".repeat(64) },
  baseline: revision,
  at: 1,
  folder: "private/conflicts/review",
  latestUpdate: null,
  canKeepMine: true,
  canUseTheirs: true,
  reason: "Both versions preserved",
  draft: { summary: "", details: "" },
};
const run = async (_: string, f: () => Promise<unknown>) => {
  await f();
};
it("requires an explicit choice, readiness and changelog before publishing the chosen local version", async () => {
  const close = vi.fn();
  render(
    <ConflictPanel
      online={true}
      preview={preview}
      busy=""
      run={run}
      onClose={close}
    />,
  );
  expect(screen.queryByText("Preserved copy details")).toBeNull();
  expect(screen.queryByText(preview.folder)).toBeNull();
  expect(screen.queryByText(preview.mine.hash)).toBeNull();
  expect(
    screen.getByRole("button", { name: "Choose a version" }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("radio", { name: /Keep my version/ }));
  const button = screen.getByRole("button", { name: "Share my version" });
  expect(button).toBeDisabled();
  fireEvent.click(screen.getByRole("checkbox"));
  expect(button).toBeDisabled();
  fireEvent.change(
    screen.getByRole("textbox", { name: "Conflict changelog" }),
    { target: { value: "Kept the reviewed quest route" } },
  );
  fireEvent.click(button);
  await waitFor(() => expect(close).toHaveBeenCalledOnce());
  expect(native).toHaveBeenLastCalledWith("resolve_conflict", {
    previewId: "review",
    choice: "mine",
    draft: { summary: "Kept the reviewed quest route", details: "" },
    editorReady: true,
  });
  expect(native.mock.calls[0][0]).toBe("save_conflict_draft");
});
it("keeps unsupported installation disabled and can retain both copies without choosing", () => {
  const close = vi.fn();
  render(
    <ConflictPanel
      online={true}
      preview={{ ...preview, canUseTheirs: false }}
      busy=""
      run={run}
      onClose={close}
    />,
  );
  expect(
    screen.getByRole("radio", { name: /Use teammate’s version/ }),
  ).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Keep both for now" }));
  expect(close).toHaveBeenCalledOnce();
  expect(native).not.toHaveBeenCalled();
});
it("uses the normal installation command and the normal readiness confirmation", async () => {
  render(
    <ConflictPanel
      online={true}
      preview={preview}
      busy=""
      run={run}
      onClose={() => {}}
    />,
  );
  fireEvent.click(
    screen.getByRole("radio", { name: /Use teammate’s version/ }),
  );
  const button = screen.getByRole("button", {
    name: "Back up and use teammate’s version",
  });
  expect(button).toBeDisabled();
  fireEvent.click(
    screen.getByRole("checkbox", { name: /unloaded this stage/ }),
  );
  fireEvent.click(button);
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("resolve_conflict", {
      previewId: "review",
      choice: "theirs",
      draft: { summary: "", details: "" },
      editorReady: true,
    }),
  );
});

it("keeps failed resolution and the creator's changelog inside the open modal", async () => {
  native.mockImplementation(async (cmd) => {
    if (cmd === "resolve_conflict")
      throw Error("Your reservation expired; review again");
  });
  const close = vi.fn();
  const guarded: import("./types").Run = async (_, action, options) => {
    try {
      await action();
    } catch (e) {
      options?.onError?.(String(e));
    }
  };
  render(
    <ConflictPanel
      online={true}
      preview={{
        ...preview,
        draft: { summary: "Keep my route", details: "Private details" },
      }}
      busy=""
      run={guarded}
      onClose={close}
    />,
  );
  fireEvent.click(screen.getByRole("radio", { name: /Keep my version/ }));
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Share my version" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "reservation expired",
  );
  expect(screen.getByLabelText("Conflict changelog")).toHaveValue(
    "Keep my route",
  );
  expect(close).not.toHaveBeenCalled();
});

it("allows offline comparison, private drafts and installing the downloaded version, while Share stays disabled", async () => {
  native.mockResolvedValue(undefined);
  render(
    <ConflictPanel
      online={false}
      preview={preview}
      busy=""
      run={run}
      onClose={() => {}}
    />,
  );
  fireEvent.click(screen.getByRole("radio", { name: /Keep my version/ }));
  fireEvent.change(screen.getByLabelText("Conflict changelog"), {
    target: { value: "Private offline explanation" },
  });
  fireEvent.click(screen.getByRole("checkbox"));
  expect(
    screen.getByRole("button", { name: "Share my version" }),
  ).toBeDisabled();
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("save_conflict_draft", {
      previewId: "review",
      draft: { summary: "Private offline explanation", details: "" },
    }),
  );
  fireEvent.click(
    screen.getByRole("radio", { name: /Use teammate’s version/ }),
  );
  const use = screen.getByRole("button", {
    name: "Back up and use teammate’s version",
  });
  expect(use).toBeEnabled();
  fireEvent.click(use);
  await waitFor(() =>
    expect(native).toHaveBeenCalledWith("resolve_conflict", {
      previewId: "review",
      choice: "theirs",
      draft: { summary: "Private offline explanation", details: "" },
      editorReady: true,
    }),
  );
});

it("shows the teammate changelog after the author with a colon", () => {
  render(
    <ConflictPanel
      online={true}
      preview={{
        ...preview,
        latestUpdate: {
          id: "update",
          author: "Alex",
          authorId: "alex",
          summary: "Fixed quest routes",
          details: "",
          at: 1,
          sequence: 1,
          revisions: [revision],
          restores: null,
        },
      }}
      busy=""
      run={run}
      onClose={vi.fn()}
    />,
  );
  expect(screen.getByText("Alex: Fixed quest routes")).toBeVisible();
});

// @vitest-environment jsdom
import { beforeEach, afterEach, expect, it, vi } from "vitest";
const native = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: native }));
import { beginDemo, endDemo, demoCall, isDemo, seedDemo } from "./demo";
import { call } from "./bridge";
import type { Snapshot, HistoryPage, Update } from "./types";
beforeEach(() => {
  localStorage.clear();
  native.mockClear();
});
afterEach(() => localStorage.clear());
it("seeds realistic collaborators, reservations, download states and a long project-wide history", () => {
  const demo = seedDemo();
  expect(demo.history).toHaveLength(80);
  expect(demo.snapshot.project!.members).toHaveLength(4);
  expect(demo.snapshot.sections.some((s) => s.conflict)).toBe(true);
  expect(demo.snapshot.sections.some((s) => s.downloadError)).toBe(true);
  expect(demo.snapshot.sections.some((s) => !s.installed && s.downloaded)).toBe(
    true,
  );
  expect(demo.snapshot.sections.find((s) => s.id === "5")!.lock!.member).toBe(
    "you",
  );
});
it("paginates the sample project and filters by slice", async () => {
  beginDemo();
  const first = await demoCall<HistoryPage>("history_page", { filter: {} });
  expect(first.entries).toHaveLength(20);
  expect(first.next).toBe(61);
  const second = await demoCall<HistoryPage>("history_page", {
    filter: { before: first.next },
  });
  expect(second.entries[0].sequence).toBe(60);
  const filtered = await demoCall<HistoryPage>("history_page", {
    filter: { section: "7" },
  });
  expect(filtered.entries.length).toBeGreaterThan(0);
  expect(
    filtered.entries.every((u) => u.revisions.some((r) => r.section === "7")),
  ).toBe(true);
});
it("preserves a required draft, rejects an entire conflicting share, then publishes locally", async () => {
  beginDemo();
  const draft = { summary: "Fixed the demo boat", details: "Only sample data" };
  await demoCall("save_draft", { projectId: "demo-project", draft });
  await expect(
    demoCall("share_update", { sections: ["5", "8"], draft }),
  ).rejects.toThrow("conflict");
  expect((await demoCall<Snapshot>("snapshot")).project!.sequence).toBe(80);
  await expect(
    demoCall("share_update", {
      sections: ["5"],
      draft: { summary: " ", details: "" },
    }),
  ).rejects.toThrow("Write what changed");
  const update = await demoCall<Update>("share_update", {
    sections: ["5"],
    draft,
  });
  expect(update.sequence).toBe(81);
  expect((await demoCall<Snapshot>("snapshot")).draft.summary).toBe("");
  expect(
    (
      await demoCall<HistoryPage>("history_page", {
        filter: { section: "5" },
      })
    ).entries[0].author,
  ).toBe("You");
});
it("intercepts demo operations before native IPC and blocks real storage or installation commands", async () => {
  beginDemo();
  expect(isDemo()).toBe(true);
  expect((await call("snapshot")).project!.id).toBe("demo-project");
  await expect(
    call("install_updates", { planId: "test-plan", editorReady: true }),
  ).rejects.toThrow("demo");
  await expect(call("sign_in", { provider: "google" })).rejects.toThrow("demo");
  expect(native).not.toHaveBeenCalled();
  endDemo();
  expect(isDemo()).toBe(false);
  expect(localStorage.getItem("miliastra-collab-preview-v1")).toBeTruthy();
});

it("shares name metadata into demo History and applies them to the simulated editor without changing slice heads", async () => {
  beginDemo();
  const before = await demoCall<Snapshot>("snapshot");
  const update = await demoCall<Update>("share_section_names", {
    names: { "5": "Ships" },
    parentVersion: 0,
    draft: { summary: "Rename the area", details: "" },
  });
  const after = await demoCall<Snapshot>("snapshot");
  expect(after.project!.heads).toEqual(before.project!.heads);
  expect(after.stage!.sections.find((s) => s.id === "5")!.name).toBe("Ships");
  expect(after.stage!.lastSavedSection).toBe(before.stage!.lastSavedSection);
  expect(after.sections.find((s) => s.id === "5")!.name).toBe("Ships");
  expect(update.revisions).toEqual([]);
  expect(update.nameChanges![0].before).toBe("Boats");
  const history = await demoCall<HistoryPage>("history_page", {
    filter: { section: "5" },
  });
  expect(history.entries[0].id).toBe(update.id);
});

it("keeps finish-request confirmations silent in the browser preview", async () => {
  beginDemo();
  const notice = vi.fn();
  window.addEventListener("preview-notice", notice);
  const before = (await demoCall<Snapshot>("snapshot")).project!.locks;
  await expect(
    call("request_finish", { section: "6" }),
  ).resolves.toBeUndefined();
  expect(notice).not.toHaveBeenCalled();
  expect((await demoCall<Snapshot>("snapshot")).project!.locks).toEqual(before);
  expect(native).not.toHaveBeenCalled();
  window.removeEventListener("preview-notice", notice);
});

it("does not imply that browser preview has connected real storage accounts", async () => {
  beginDemo();
  expect((await demoCall<Snapshot>("snapshot")).storageAccounts).toEqual({
    google: "notConnected",
    onedrive: "notConnected",
  });
  expect(native).not.toHaveBeenCalled();
});

it("keeps multiple reservations, preserves them on conflicts, and finishes one or all", async () => {
  beginDemo();
  await demoCall("start_work", { section: "0" });
  await demoCall("start_work", { section: "1" });
  const owned = async () =>
    (await demoCall<Snapshot>("snapshot"))
      .project!.locks.filter((l) => l.member === "you")
      .map((l) => l.section)
      .sort();
  expect(await owned()).toEqual(["0", "1", "5"]);
  await expect(demoCall("start_work", { section: "6" })).rejects.toThrow(
    "reserved",
  );
  expect(await owned()).toEqual(["0", "1", "5"]);
  await demoCall("finish_work", { section: "0" });
  expect(await owned()).toEqual(["1", "5"]);
  await demoCall("finish_work");
  expect(await owned()).toEqual(["5"]);
  await expect(demoCall("finish_work", { section: "5" })).rejects.toThrow(
    "Share your saved changes",
  );
  expect(await owned()).toEqual(["5"]);
  await demoCall("share_update", {
    sections: ["5"],
    draft: { summary: "Finished boats", details: "" },
  });
  expect(await owned()).toEqual([]);
  expect(
    (await demoCall<Snapshot>("snapshot")).project!.locks.some(
      (l) => l.member !== "you",
    ),
  ).toBe(true);
});

it("rejects removed History filters", async () => {
  beginDemo();
  for (const field of ["q", "author", "from", "to"]) {
    await expect(
      demoCall("history_page", { filter: { [field]: "removed" } }),
    ).rejects.toThrow("only slice filtering and pagination");
  }
});

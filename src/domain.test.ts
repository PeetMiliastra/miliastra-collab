import { describe, it, expect } from "vitest";
import {
  canShare,
  updateStatus,
  installationBlocker,
  nextStep,
  people,
  sliceStatus,
} from "./domain";
import { empty } from "./App";
import type { Snapshot, Update, Revision } from "./types";
const r: Revision = {
  section: "5",
  name: "Boats",
  hash: "a".repeat(64),
  contentHash: "b".repeat(64),
  parent: null,
  artifact: "History/11111111-1111-4111-8111-111111111111/6.gis",
  size: 100,
};
const u: Update = {
  id: "update",
  author: "Alex",
  authorId: "alex",
  summary: "Fixed the boat deck",
  details: "",
  at: 1,
  sequence: 1,
  revisions: [r],
  restores: null,
};
function snapshot(): Snapshot {
  return {
    ...empty,
    project: {
      id: "p",
      name: "P",
      storage: { kind: "folder", location: "" },
      members: [],
      locks: [],
      heads: { "5": r },
      sequence: 1,
    },
    sections: [
      {
        id: "5",
        name: "Boats",
        changed: false,
        conflict: false,
        lock: null,
        downloaded: false,
        installed: false,
      },
    ],
  };
}
describe("Sharing", () => {
  it("requires a meaningful changelog, selected changes, and a connection", () => {
    expect(canShare("  ", ["5"], true)).toBe(false);
    expect(canShare("Fix boats", [], true)).toBe(false);
    expect(canShare("Fix boats", ["5"], false)).toBe(false);
    expect(canShare("Fix boats", ["5"], true)).toBe(true);
  });
});
describe("History status", () => {
  it("distinguishes download, installation, conflict, and supersession", () => {
    const s = snapshot();
    expect(updateStatus(u, s)).toBe("Downloading");
    s.sections[0].installationPending = true;
    expect(updateStatus(u, s)).toBe("Checking files");
    s.sections[0].installationPending = false;
    s.sections[0].downloaded = true;
    expect(updateStatus(u, s)).toBe("Ready to apply");
    s.sections[0].installed = true;
    expect(updateStatus(u, s)).toBe("Applied");
    s.sections[0].conflict = true;
    expect(updateStatus(u, s)).toBe("Both changed");
    s.project!.heads["5"] = { ...r, hash: "c".repeat(64) };
    expect(updateStatus(u, s)).toBe("Superseded");
  });
});

it("distinguishes synced name metadata from editor installation and supersession", () => {
  const s = snapshot();
  s.project!.sectionNames = { "5": "Ships" };
  const names = {
    ...u,
    revisions: [],
    nameChanges: [{ section: "5", before: "Boats", after: "Ships" }],
  };
  expect(updateStatus(names, s)).toBe("Names synced");
  s.stage = {
    path: "fixture",
    level: "42",
    account: "recipient",
    name: "Stage",
    sections: [{ id: "5", name: "Ships" }],
    lastSavedSection: "5",
    settingsHash: "hash",
  };
  expect(updateStatus(names, s)).toBe("Names applied");
  s.project!.sectionNames["5"] = "Fleet";
  expect(updateStatus(names, s)).toBe("Superseded");
});

it("keeps an entire update batch blocked by a conflicting, incomplete or dirty section", () => {
  const s = snapshot();
  s.compatibility = { ...s.compatibility, directInstall: true };
  s.sections[0].downloaded = true;
  expect(installationBlocker(s)).toBeNull();
  s.sections.push({
    ...s.sections[0],
    id: "8",
    name: "Quests",
    conflict: true,
  });
  expect(installationBlocker(s)).toContain("Review Quests");
  s.sections[1].conflict = false;
  s.sections[1].changed = true;
  expect(installationBlocker(s)).toContain("preserve your changes to Quests");
  s.sections[1].changed = false;
  s.sections[1].downloaded = false;
  expect(installationBlocker(s)).toContain("finish downloading");
  s.sections[1].downloadError = "Revoked access";
  expect(installationBlocker(s)).toContain("storage connection");
  s.sections[1].downloadError = null;
  s.sections[1].downloaded = true;
  s.online = false;
  expect(installationBlocker(s)).toBeNull();
  s.sections.push({
    ...s.sections[0],
    id: "shared",
    name: "Shared level settings",
  });
  // A teammate's shared level settings install with the batch like any slice.
  expect(installationBlocker(s)).toBeNull();
  s.sections[s.sections.length - 1].changed = true;
  expect(installationBlocker(s)).toContain(
    "preserve your changes to Shared level settings",
  );
  s.sections[s.sections.length - 1].changed = false;
  s.installation = {
    id: "install",
    phase: "awaitingReload",
    journal: "private",
    revisions: [],
    error: null,
  };
  expect(installationBlocker(s)).toContain("current update");
});

describe("Slices page guidance", () => {
  it("labels slices in plain language", () => {
    const base = snapshot().sections[0];
    expect(sliceStatus(base).label).toBe("Downloading");
    expect(sliceStatus({ ...base, downloaded: true }).label).toBe(
      "Ready to apply",
    );
    expect(sliceStatus({ ...base, downloadError: "offline" }).label).toBe(
      "Download paused",
    );
    expect(sliceStatus({ ...base, changed: true, installed: true }).label).toBe(
      "Your changes",
    );
    expect(sliceStatus({ ...base, conflict: true, changed: true }).label).toBe(
      "Both changed",
    );
    expect(sliceStatus({ ...base, installationPending: true }).label).toBe(
      "Checking files",
    );
    expect(sliceStatus({ ...base, installed: true })).toEqual({
      label: "Up to date",
      tone: "clean",
    });
  });
  it("suggests one next step: finish, conflict, apply, then share", () => {
    const s = snapshot();
    s.compatibility.directInstall = true;
    expect(nextStep(s)).toBe(null);
    s.sections[0].downloaded = true;
    s.sections.push({
      id: "shared",
      name: "Shared level settings",
      changed: false,
      conflict: false,
      lock: null,
      downloaded: true,
      installed: true,
    });
    expect(nextStep(s)).toBe("apply");
    s.sections.push({
      id: "6",
      name: "Islands",
      changed: true,
      conflict: false,
      lock: null,
      downloaded: true,
      installed: true,
    });
    // Changes to another slice don't block applying updates.
    expect(nextStep(s)).toBe("apply");
    s.sections[0].downloaded = false;
    expect(nextStep(s)).toBe("share");
    s.sections[2].conflict = true;
    expect(nextStep(s)).toBe("conflict");
    s.installation = {
      id: "i",
      phase: "awaitingReload",
      journal: "",
      revisions: [],
      error: null,
    };
    expect(nextStep(s)).toBe("install");
  });
  it("names people briefly", () => {
    expect(people([])).toBe("a teammate");
    expect(people(["Alex", "Alex"])).toBe("Alex");
    expect(people(["Alex", "Mia"])).toBe("Alex and Mia");
    expect(people(["Alex", "Mia", "Sam"])).toBe("Alex, Mia and Sam");
    expect(people(["Alex", "Mia", "Sam", "Kai"])).toBe(
      "Alex, Mia and 2 others",
    );
  });
});

it("blocks installation until committed update details are available", () => {
  expect(installationBlocker({ ...empty, metadataPending: true })).toMatch(
    /Waiting for update details/,
  );
});

it("keeps unknown baseline comparisons out of conflict and apply guidance", () => {
  const state = snapshot();
  state.sections[0] = { ...state.sections[0], comparisonPending: true };
  expect(sliceStatus(state.sections[0])).toEqual({
    label: "Checking",
    tone: "blue",
  });
  expect(installationBlocker(state)).toBe(
    "Wait for saved slices to finish checking.",
  );
  expect(nextStep(state)).toBeNull();
  expect(
    sliceStatus({ ...state.sections[0], downloadError: "offline" }).label,
  ).toBe("Check paused");
  state.sections[0] = {
    ...state.sections[0],
    comparisonPending: false,
    downloaded: true,
    installed: true,
  };
  expect(sliceStatus(state.sections[0]).label).toBe("Up to date");
  expect(nextStep(state)).toBeNull();
});

import { storageProviders, defaultCloudProviders } from "./providers";
import type {
  Draft,
  Snapshot,
  Update,
  Revision,
  HistoryPage,
  NamesDraft,
} from "./types";
const KEY = "miliastra-collab-preview-v1";
const ACTIVE = "miliastra-collab-preview-active";
type Demo = { snapshot: Snapshot; history: Update[] };
const hex = (n: number) => n.toString(16).padStart(64, "0");
export function isDemo() {
  return (
    typeof localStorage !== "undefined" &&
    localStorage.getItem(ACTIVE) === "yes"
  );
}
export function seedDemo(): Demo {
  const now = Math.floor(Date.now() / 1000);
  const names = [
    "Common",
    "Save",
    "UI - Common",
    "UI - Items",
    "Fishing Rods",
    "Boats",
    "Islands",
    "NPCs",
    "Quests",
    "Totems",
    "Shops",
  ];
  const members = [
    { id: "you", name: "You", owner: true, joined: now - 40 * 86400 },
    { id: "alex", name: "Alex", owner: false, joined: now - 38 * 86400 },
    { id: "mia", name: "Mia", owner: false, joined: now - 30 * 86400 },
    { id: "sam", name: "Sam", owner: false, joined: now - 2 * 86400 },
  ];
  const stories = [
    [
      "Built a sheltered fishing cove",
      "Added rocks, a quiet beach and a dock for first-time visitors.",
    ],
    [
      "Fixed passengers falling through the boat deck",
      "Repositioned the deck collision and tested boarding at both docks.",
    ],
    [
      "Added a new quest at the lighthouse",
      "Players can help the keeper find three missing lanterns.",
    ],
    [
      "Made shop prices easier to read",
      "Updated the item cards and the confirmation message.",
    ],
    [
      "Polished the island arrival",
      "Adjusted the spawn point and added signs toward the village.",
    ],
    [
      "Added dialogue for the harbour master",
      "The harbour master now explains how to borrow a boat.",
    ],
    [
      "Balanced the beginner fishing rods",
      "Improved the starter rod and clarified its description.",
    ],
    [
      "Connected the northern totem route",
      "Added clear landmarks between the three totems.",
    ],
  ];
  const baselineId = crypto.randomUUID();
  const heads: Record<string, Revision> = {};
  for (let i = 0; i < names.length; i++)
    heads[String(i)] = {
      section: String(i),
      name: names[i],
      hash: hex(i + 1),
      contentHash: hex(i + 101),
      parent: null,
      artifact: `History/${baselineId}/${i + 1}.gis`,
      size: 12000 + i * 531,
    };
  const history: Update[] = [];
  for (let i = 1; i <= 80; i++) {
    const updateId = crypto.randomUUID();
    const ids = [String([5, 6, 8, 10, 6, 7, 4, 9][(i - 1) % 8])];
    if (i % 9 === 0) ids.push("3");
    const revisions = ids.map((id, j) => {
      const r = {
        ...heads[id],
        hash: hex(i * 100 + j),
        contentHash: hex(i * 100 + j + 9000),
        parent: heads[id].hash,
        artifact: `History/${updateId}/${Number(id) + 1}.gis`,
      };
      heads[id] = r;
      return r;
    });
    const author = members[1 + (i % 3)],
      story = stories[(i - 1) % stories.length];
    history.push({
      id: updateId,
      author: author.name,
      authorId: author.id,
      summary: story[0],
      details: story[1],
      at: now - (81 - i) * 11 * 3600,
      sequence: i,
      revisions,
      restores: null,
    });
  }
  const locks = [
    {
      section: "5",
      member: "you",
      name: "You",
      fence: 1,
      expires: now + 86400,
    },
    {
      section: "6",
      member: "alex",
      name: "Alex",
      fence: 2,
      expires: now + 86400,
    },
    {
      section: "7",
      member: "mia",
      name: "Mia",
      fence: 3,
      expires: now + 86400,
    },
  ];
  const snapshot: Snapshot = {
    settings: {
      endpoint: "https://collabs.miliastra.dev",
      saveRoot: "",
      oauth: { microsoftClientId: "", googleClientId: "" },
      autostart: false,
      autoApply: true,
      screenRecognition: false,
    },
    stage: {
      path: "",
      account: "Demo",
      level: "Demo",
      name: "Fisch — demo",
      sections: names.map((name, i) => ({ id: String(i), name })),
      lastSavedSection: "5",
      settingsHash: "demo",
    },
    project: {
      id: "demo-project",
      name: "Fisch — demo",
      storage: { kind: "folder", location: "" },
      members,
      locks,
      heads,
      sequence: 80,
      sectionNames: Object.fromEntries(names.map((n, i) => [String(i), n])),
      namesVersion: 0,
    },
    member: "you",
    online: true,
    protocolMismatch: false,
    message: "Sample project. No game saves or team storage are connected.",
    draft: { summary: "", details: "" },
    compatibility: {
      directInstall: false,
      liveDetection: false,
      restoreVersions: false,
      reason: "Sample data only. Demo actions never modify game saves.",
    },
    game: { running: false, foreground: false },
    sections: names.map((name, i) => ({
      id: String(i),
      name,
      changed: i === 5 || i === 8,
      conflict: i === 8,
      lock: locks.find((l) => l.section === String(i)) ?? null,
      downloaded: i !== 9 && i !== 10,
      installed: ![6, 7, 8, 9, 10].includes(i),
      incoming: [6, 7, 8, 9, 10].includes(i)
        ? (() => {
            const u = history.find((x) =>
              x.revisions.some((r) => r.hash === heads[String(i)].hash),
            );
            return u
              ? { author: u.author, summary: u.summary, at: u.at }
              : null;
          })()
        : null,
      downloadError:
        i === 10
          ? "Demo: storage temporarily unavailable. Automatic retry is pending."
          : null,
    })),
  };
  return { snapshot, history: history.reverse() };
}
function read(): Demo {
  const stored = localStorage.getItem(KEY);
  if (stored) {
    try {
      return JSON.parse(stored);
    } catch {}
  }
  return seedDemo();
}
function save(demo: Demo) {
  localStorage.setItem(KEY, JSON.stringify(demo));
  window.dispatchEvent(new Event("demo-changed"));
}
export function beginDemo() {
  localStorage.setItem(ACTIVE, "yes");
  if (!localStorage.getItem(KEY)) save(seedDemo());
  window.dispatchEvent(new Event("demo-changed"));
}
export function endDemo() {
  localStorage.removeItem(ACTIVE);
  window.dispatchEvent(new Event("demo-changed"));
}
export async function demoCall<T>(
  command: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  const data = read(),
    s = data.snapshot,
    p = s.project!;
  let result: unknown;
  switch (command) {
    case "request_finish": {
      const section = s.sections.find((x) => x.id === args.section);
      if (!section?.lock || section.lock.member === "you")
        throw Error("Nobody is working on this slice now");
      return undefined as T;
    }
    case "snapshot":
      return structuredClone({
        ...s,
        cloudProviders: defaultCloudProviders,
        storageAccounts: Object.fromEntries(
          storageProviders
            .filter((provider) => provider.cloud)
            .map((provider) => [provider.id, "notConnected"]),
        ),
      }) as T;
    case "pending_invitation":
      return null as T;
    case "connections":
      return [] as T;
    case "discover":
      return [s.stage] as T;
    case "history_page": {
      const f = (args.filter ?? {}) as {
        section?: string;
        before?: number;
      };
      if (Object.keys(f).some((key) => !["section", "before"].includes(key)))
        throw Error("History supports only slice filtering and pagination.");
      const entries = data.history.filter(
        (u) =>
          (!f.before || u.sequence < f.before) &&
          (!f.section ||
            u.revisions.some((r) => r.section === f.section) ||
            u.nameChanges?.some((c) => c.section === f.section)),
      );
      const page: HistoryPage = {
        entries: entries.slice(0, 20),
        next: entries.length > 20 ? entries[19].sequence : null,
      };
      return structuredClone(page) as T;
    }
    case "save_names_draft":
      if (args.projectId !== p.id) throw Error("The project changed.");
      s.nameSync ??= {
        pending: [],
        draft: { edits: {} },
      };
      for (const [id, edit] of Object.entries(
        (args.draft as NamesDraft).edits,
      )) {
        if (edit.name === edit.baseName) delete s.nameSync.draft.edits[id];
        else s.nameSync.draft.edits[id] = edit;
      }
      break;
    case "share_section_names": {
      const names = args.names as Record<string, string>;
      const draft = { summary: "Updated slice names", details: "" };
      if (args.parentVersion !== (p.namesVersion ?? 0))
        throw Error("Project names changed; review your draft.");
      p.sectionNames ??= Object.fromEntries(
        s.sections.filter((x) => x.id !== "shared").map((x) => [x.id, x.name]),
      );
      const changes = Object.entries(names)
        .filter(([id, n]) => p.sectionNames![id] !== n)
        .map(([section, after]) => ({
          section,
          before: p.sectionNames![section],
          after,
        }));
      if (
        !changes.length ||
        changes.some(
          (c) =>
            !c.before ||
            !c.after.trim() ||
            new TextEncoder().encode(c.after).length > 256 ||
            /[\u0000-\u001f\u007f]/.test(c.after),
        )
      )
        throw Error("Choose valid changed slice names.");
      const update: Update = {
        id: crypto.randomUUID(),
        author: "You",
        authorId: "you",
        summary: draft.summary.trim(),
        details: draft.details,
        at: Math.floor(Date.now() / 1000),
        sequence: ++p.sequence,
        revisions: [],
        restores: null,
        nameChanges: changes,
        namesParent: p.namesVersion ?? 0,
      };
      p.namesVersion = (p.namesVersion ?? 0) + 1;
      for (const c of changes) {
        p.sectionNames[c.section] = c.after;
        s.sections.find((x) => x.id === c.section)!.name = c.after;
      }
      s.stage!.sections = s.stage!.sections.map((section) => ({
        ...section,
        name: p.sectionNames![section.id] ?? section.name,
      }));
      if (s.nameSync)
        for (const change of changes) {
          if (
            s.nameSync.draft.edits[change.section]?.name.trim() === change.after
          )
            delete s.nameSync.draft.edits[change.section];
        }
      data.history.unshift(update);
      result = update;
      break;
    }
    case "save_task":
      if (args.projectId !== p.id) throw Error("The project changed.");
      s.task = args.task as Snapshot["task"];
      break;
    case "refresh_saved_changes":
      break;
    case "save_draft":
      if (args.projectId !== p.id) throw Error("The project changed.");
      s.draft = args.draft as Draft;
      break;
    case "start_work": {
      const section = s.sections.find((x) => x.id === args.section);
      if (!section) throw Error("Unknown section");
      if (section.lock && section.lock.member !== "you" && !args.takeover)
        throw Error(
          `${section.name} is reserved by ${section.lock.name}. Your changes stay local.`,
        );
      p.locks = p.locks.filter((l) => l.section !== section.id);
      p.locks.push({
        section: section.id,
        member: "you",
        name: "You",
        fence: Date.now(),
        expires: Math.floor(Date.now() / 1000) + 86400,
      });
      s.sections.forEach(
        (x) => (x.lock = p.locks.find((l) => l.section === x.id) ?? null),
      );
      break;
    }
    case "finish_work":
      if (
        args.section != null &&
        s.sections.some(
          (x) => x.id === args.section && x.changed && x.lock?.member === "you",
        )
      )
        throw Error(
          "Share your saved changes before finishing this slice. It stays reserved.",
        );
      p.locks = p.locks.filter(
        (l) =>
          l.member !== "you" ||
          s.sections.some((x) => x.id === l.section && x.changed) ||
          (args.section != null && l.section !== args.section),
      );
      s.sections.forEach(
        (x) => (x.lock = p.locks.find((l) => l.section === x.id) ?? null),
      );
      break;
    case "share_update": {
      const draft = args.draft as Draft,
        ids = args.sections as string[];
      if (!draft.summary.trim() || !ids.length)
        throw Error("Write what changed and choose a section.");
      const sections = ids.map((id) => s.sections.find((x) => x.id === id));
      if (
        sections.some(
          (x) =>
            !x ||
            !x.changed ||
            x.conflict ||
            (x.lock && x.lock.member !== "you"),
        )
      )
        throw Error(
          "Resolve the slice reservation or local conflict before sharing.",
        );
      const updateId = crypto.randomUUID();
      const revisions = sections.map((x) => {
        const r = {
          ...p.heads[x!.id],
          hash: hex(Date.now() + Number(x!.id)),
          parent: p.heads[x!.id].hash,
        };
        r.artifact = `History/${updateId}/${Number(x!.id) + 1}.gis`;
        p.heads[x!.id] = r;
        x!.changed = false;
        x!.installed = true;
        x!.downloaded = true;
        return r;
      });
      const update: Update = {
        id: updateId,
        author: "You",
        authorId: "you",
        summary: draft.summary.trim(),
        details: draft.details,
        sequence: ++p.sequence,
        at: Math.floor(Date.now() / 1000),
        revisions,
        restores: null,
      };
      data.history.unshift(update);
      s.draft = { summary: "", details: "" };
      if (!args.keepReserved) {
        p.locks = p.locks.filter(
          (l) => !(l.member === "you" && ids.includes(l.section)),
        );
        s.sections.forEach(
          (x) => (x.lock = p.locks.find((l) => l.section === x.id) ?? null),
        );
      }
      result = update;
      break;
    }
    case "refresh":
      s.sections.forEach((x) => {
        x.downloaded = true;
        x.downloadError = null;
      });
      break;
    case "save_settings":
      s.settings = args.settings as Snapshot["settings"];
      break;
    case "leave_project":
      endDemo();
      return undefined as T;
    case "dismiss_invitation":
      return undefined as T;
    case "preview_conflict": {
      const section = s.sections.find((x) => x.id === args.section)!;
      const theirs = p.heads[section.id];
      const latest =
        data.history.find((u) =>
          u.revisions.some((r) => r.section === section.id),
        ) ?? null;
      return {
        id: `demo-${section.id}`,
        project: p.id,
        section: section.id,
        name: section.name,
        mine: { ...theirs, hash: hex(Number(section.id) + 7) },
        theirs,
        baseline: null,
        folder: "",
        at: Math.floor(Date.now() / 1000),
        latestUpdate: latest,
        canKeepMine: true,
        canUseTheirs: true,
        reason: "",
        draft: { summary: "", details: "" },
      } as T;
    }
    case "save_conflict_draft":
      return undefined as T;
    default:
      throw Error(
        "This is a demo project. Leave the demo to connect real storage, invite creators or apply updates.",
      );
  }
  save(data);
  return structuredClone(result) as T;
}

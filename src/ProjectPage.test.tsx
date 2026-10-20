// @vitest-environment jsdom
import { afterEach, describe, it, expect, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  within,
  waitFor,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
import { SharePanel, ProjectPage } from "./ProjectPage";
import { empty } from "./App";
import { call } from "./bridge";
vi.mock("./bridge", () => ({
  desktop: true,
  call: vi.fn().mockResolvedValue([]),
}));
afterEach(cleanup);
describe("Changelog panel", () => {
  it("keeps sharing disabled until a changelog exists", () => {
    const props = {
      state: {
        ...empty,
        online: true,
        sections: [
          {
            id: "5",
            name: "Boats",
            changed: true,
            conflict: false,
            lock: null,
            downloaded: true,
            installed: true,
          },
        ],
      },
      busy: "",
      draft: { summary: "", details: "" },
      setDraft: vi.fn(),
      run: vi.fn(),
      onClose: vi.fn(),
    };
    const view = render(<SharePanel {...props} />);
    expect(screen.getByRole("button", { name: "Share update" })).toBeDisabled();
    view.rerender(
      <SharePanel
        {...props}
        draft={{ summary: "Fixed the boat deck", details: "" }}
      />,
    );
    expect(screen.getByRole("button", { name: "Share update" })).toBeEnabled();
  });
  it("does not discard the draft when dismissed", () => {
    const close = vi.fn(),
      set = vi.fn();
    render(
      <SharePanel
        state={empty}
        busy=""
        draft={{ summary: "Draft", details: "Keep me" }}
        setDraft={set}
        run={vi.fn()}
        onClose={close}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(close).toHaveBeenCalledOnce();
    expect(set).not.toHaveBeenCalled();
  });
});

it("keeps explicit owner takeover available beside conflict resolution", () => {
  const state = {
    ...empty,
    online: true,
    member: "me",
    project: {
      id: "p",
      name: "P",
      storage: { kind: "folder" as const, location: "" },
      members: [{ id: "me", name: "Owner", owner: true }],
      locks: [],
      heads: {},
      sequence: 1,
    },
    sections: [
      {
        id: "8",
        name: "Quests",
        changed: true,
        conflict: true,
        lock: {
          section: "8",
          member: "alex",
          name: "Alex",
          fence: 2,
          expires: 9999999999,
        },
        downloaded: true,
        installed: false,
      },
    ],
  };
  render(
    <ProjectPage
      state={state}
      busy=""
      run={vi.fn()}
      onSetup={() => {}}
      onShare={() => {}}
      onHistory={() => {}}
    />,
  );
  expect(
    screen.getAllByRole("button", { name: "Review conflict" })[0],
  ).toBeEnabled();
  expect(screen.getByRole("button", { name: "Take over" })).toBeEnabled();
});

it("keeps onboarding actions without welcome art or feature slogans", async () => {
  const onSetup = vi.fn(),
    onDemo = vi.fn();
  render(
    <ProjectPage
      state={empty}
      busy=""
      run={vi.fn()}
      onSetup={onSetup}
      onDemo={onDemo}
      onShare={() => {}}
      onHistory={() => {}}
    />,
  );
  expect(screen.getByRole("heading", { name: "Your projects" })).toBeVisible();
  expect(
    document.querySelector(
      ".welcome-card, .welcome-art, .feature-grid, .eyebrow",
    ),
  ).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Create project" }));
  expect(onSetup).toHaveBeenCalledOnce();
  expect(onSetup).toHaveBeenCalledWith();
  expect(screen.queryByRole("button", { name: "Join project" })).toBeNull();
  fireEvent.click(
    screen.getByRole("button", { name: "Explore a demo project" }),
  );
  expect(onDemo).toHaveBeenCalledOnce();
});

it("shows personal changed-slice badges while keeping reservation ownership and stable slice numbers", () => {
  const state = {
    ...empty,
    online: true,
    member: "me",
    stage: {
      path: "stage",
      account: "A",
      level: "10",
      name: "Fisch",
      sections: [],
      settingsHash: "",
      lastSavedSection: "8",
    },
    project: {
      id: "p",
      name: "Fisch",
      storage: { kind: "folder" as const, location: "" },
      members: [],
      heads: {},
      locks: [],
      sequence: 1,
    },
    sections: [
      {
        id: "0",
        name: "Common",
        changed: false,
        conflict: false,
        downloaded: true,
        installed: true,
        lock: null,
      },
      {
        id: "5",
        name: "Boats",
        changed: true,
        conflict: false,
        downloaded: true,
        installed: true,
        lock: {
          section: "5",
          member: "me",
          name: "Robin",
          fence: 1,
          expires: 9999999999,
        },
      },
      {
        id: "8",
        name: "Quests",
        changed: true,
        conflict: true,
        downloaded: true,
        installed: false,
        lock: null,
      },
    ],
  };
  render(
    <ProjectPage
      state={state}
      busy=""
      run={vi.fn()}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
    />,
  );
  const work = screen.getByRole("article", { name: "Your work" });
  expect(within(work).getByText("Boats")).toBeVisible();
  expect(within(work).getByText("Quests")).toBeVisible();
  expect(within(work).getByText("Needs review")).toBeVisible();
  expect(screen.getAllByRole("button", { name: "View changes" })).toHaveLength(
    2,
  );
  expect(screen.getAllByRole("button", { name: "Revert" })).toHaveLength(2);
  expect(screen.queryByRole("button", { name: "Move asset" })).toBeNull();
  expect(
    screen.getByRole("button", { name: "Move asset between slices" }),
  ).toBeVisible();
  expect(within(work).queryByText("Common")).toBeNull();
  expect(document.querySelector(".saved-note")).toBeNull();
  expect(within(screen.getByRole("table")).getByText("You")).toBeVisible();
  expect(screen.queryByRole("button", { name: "All sections" })).toBeNull();
  expect(screen.getByText("Common")).toBeVisible();
  expect(screen.getByLabelText("Slice 1")).toHaveTextContent("1");
  expect(screen.getByLabelText("Slice 6")).toHaveTextContent("6");
  expect(screen.queryByText("More sections and details")).toBeNull();
  expect(
    screen.getByRole("button", { name: "Review and apply" }),
  ).toBeEnabled();
  expect(
    screen.getByText("You and a teammate both changed Quests"),
  ).toBeVisible();
});
it("does not share a selected section that becomes conflicted while the draft stays open", () => {
  const section = {
    id: "5",
    name: "Boats",
    changed: true,
    conflict: false,
    downloaded: true,
    installed: true,
    lock: null,
  };
  const props = {
    state: { ...empty, online: true, sections: [section] },
    busy: "",
    draft: { summary: "Fixed boats", details: "Keep" },
    setDraft: vi.fn(),
    run: vi.fn(),
    onClose: vi.fn(),
  };
  const view = render(<SharePanel {...props} />);
  expect(screen.getByRole("button", { name: "Share update" })).toBeEnabled();
  view.rerender(
    <SharePanel
      {...props}
      state={{ ...props.state, sections: [{ ...section, conflict: true }] }}
    />,
  );
  expect(screen.getByRole("button", { name: "Share update" })).toBeDisabled();
  expect(screen.getByDisplayValue("Fixed boats")).toBeVisible();
  expect(screen.getByText(/Review Boats before sharing it/)).toBeVisible();
});
it("leaves conflicted slices unticked so the rest can be shared", () => {
  const slice = {
    changed: true,
    downloaded: true,
    installed: true,
    lock: null,
  };
  render(
    <SharePanel
      state={{
        ...empty,
        online: true,
        sections: [
          { ...slice, id: "5", name: "Boats", conflict: false },
          { ...slice, id: "8", name: "Quests", conflict: true },
        ],
      }}
      busy=""
      draft={{ summary: "Rebuilt the boat deck", details: "" }}
      setDraft={vi.fn()}
      run={vi.fn()}
      onClose={vi.fn()}
      task={{ kind: "share", selected: [], manualSelection: false }}
    />,
  );
  expect(screen.getByRole("checkbox", { name: /Boats/ })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: /Quests/ })).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Share update" })).toBeEnabled();
});

it("shows available sections immediately in a clean project and keeps all slices visible during live updates", () => {
  const state = {
    ...empty,
    online: true,
    project: {
      id: "new",
      name: "Clean stage",
      storage: { kind: "folder" as const, location: "" },
      members: [],
      heads: {},
      locks: [],
      sequence: 1,
    },
    sections: [
      {
        id: "0",
        name: "Common",
        changed: false,
        conflict: false,
        downloaded: true,
        installed: true,
        lock: null,
      },
    ],
  };
  const props = {
    state,
    busy: "",
    run: vi.fn(),
    onSetup: vi.fn(),
    onShare: vi.fn(),
    onHistory: vi.fn(),
  };
  const view = render(<ProjectPage {...props} />);
  expect(screen.queryByRole("button", { name: "All sections" })).toBeNull();
  expect(screen.getByText("Common")).toBeVisible();
  view.rerender(
    <ProjectPage
      {...props}
      state={{ ...state, sections: [{ ...state.sections[0], changed: true }] }}
    />,
  );
  expect(screen.queryByRole("button", { name: "All sections" })).toBeNull();
  expect(document.querySelector(".section-list")).toBeVisible();
});

it("releases unchanged owned slices without sharing and keeps takeover confirmation", async () => {
  const sections = ["Boats", "Islands"].map((name, i) => ({
    id: String(i),
    name,
    changed: false,
    conflict: false,
    downloaded: true,
    installed: true,
    lock: {
      section: String(i),
      member: i === 0 ? "me" : "alex",
      name: i === 0 ? "Robin" : "Alex",
      fence: 1,
      expires: 9999999999,
    },
  }));
  const state = {
    ...empty,
    online: true,
    member: "me",
    project: {
      id: "p",
      name: "Fisch",
      storage: { kind: "folder" as const, location: "" },
      members: [],
      heads: {},
      locks: sections.map((s) => s.lock),
      sequence: 1,
    },
    sections,
  };
  const run = vi.fn(async (_: string, action: () => Promise<unknown>) => {
    await action();
  });
  render(
    <ProjectPage
      state={state}
      busy=""
      run={run}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
    />,
  );
  expect(screen.queryByRole("button", { name: /Finish/ })).toBeNull();
  const table = screen.getByRole("table");
  const own = within(table).getByText("Boats", { exact: true }).closest("tr")!;
  fireEvent.click(within(own).getByRole("button", { name: "Release" }));
  await waitFor(() =>
    expect(call).toHaveBeenCalledWith("finish_work", { section: "0" }),
  );
  expect(call).not.toHaveBeenCalledWith("share_update", expect.anything());
  const theirs = within(table)
    .getByText("Islands", { exact: true })
    .closest("tr")!;
  // Not the owner (members is empty), yet taking over is offered.
  fireEvent.click(within(theirs).getByRole("button", { name: "Take over" }));
  const dialog = screen.getByRole("dialog", { name: "Take over Islands?" });
  expect(dialog).toHaveTextContent("may still be working on it");
  expect(dialog).toHaveTextContent("choose which version to keep");
  expect(call).not.toHaveBeenCalledWith("start_work", expect.anything());
  fireEvent.click(within(dialog).getByRole("button", { name: "Take over" }));
  await waitFor(() =>
    expect(call).toHaveBeenCalledWith("start_work", {
      section: "1",
      takeover: true,
    }),
  );
});

it("shows one activity card per collaborator, merges personal changes, and keeps changed slices reserved", () => {
  const lease = (section: string, member: string) => ({
    section,
    member,
    name: member,
    fence: 1,
    expires: 9999999999,
  });
  const sections = [
    { id: "0", name: "Boats", changed: true, lock: lease("0", "me") },
    { id: "1", name: "Islands", changed: false, lock: lease("1", "alex") },
    {
      id: "2",
      name: "Quests",
      changed: true,
      conflict: true,
      lock: lease("2", "alex"),
    },
    { id: "3", name: "Clean", changed: false, lock: lease("3", "me") },
    { id: "shared", name: "Shared level settings", changed: true, lock: null },
  ].map((s) => ({ conflict: false, downloaded: true, installed: true, ...s }));
  const state = {
    ...empty,
    online: true,
    member: "me",
    sections,
    project: {
      id: "p",
      name: "P",
      storage: { kind: "folder" as const, location: "" },
      members: [
        { id: "alex", name: "Alex", owner: false },
        { id: "me", name: "Robin", owner: true },
        { id: "idle", name: "Sam", owner: false },
      ],
      heads: {},
      locks: [],
      sequence: 1,
    },
  };
  const run = vi.fn();
  render(
    <ProjectPage
      state={state}
      busy=""
      run={run}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
    />,
  );
  const cards = screen.getByRole("region", { name: "Collaborator activity" });
  expect(within(cards).getAllByRole("article")).toHaveLength(3);
  const own = within(cards).getByRole("article", { name: "Your work" });
  expect(within(own).getAllByRole("listitem")).toHaveLength(4);
  expect(within(own).getByText("Boats")).toBeVisible();
  expect(within(own).getByText("Quests")).toBeVisible();
  expect(within(own).queryByText("Islands")).toBeNull();
  expect(
    within(cards).getByRole("article", { name: "Alex’s work" }).textContent,
  ).toContain("IslandsQuests");
  expect(
    within(cards).getByRole("article", { name: "Sam’s work" }),
  ).toHaveTextContent("No reserved slices");
  const table = screen.getByRole("table");
  const changedRow = within(table)
    .getByText("Boats", { exact: true })
    .closest("tr")!;
  expect(changedRow).toHaveTextContent("Reserved by you");
  expect(
    within(changedRow).queryByRole("button", { name: "Release" }),
  ).toBeNull();
  const cleanRow = within(table)
    .getByText("Clean", { exact: true })
    .closest("tr")!;
  expect(
    within(cleanRow).getByRole("button", { name: "Release" }),
  ).toBeEnabled();
  const otherRow = within(table)
    .getByText("Islands", { exact: true })
    .closest("tr")!;
  expect(
    within(otherRow).queryByRole("button", { name: "Release" }),
  ).toBeNull();
  expect(run).not.toHaveBeenCalled();
});
it("shows who is online or in Miliastra on the collaborator cards", () => {
  const state = {
    ...empty,
    online: true,
    member: "me",
    game: { running: false, foreground: false },
    project: {
      id: "p",
      name: "Fisch",
      storage: { kind: "folder" as const, location: "" },
      members: [
        { id: "me", name: "Robin", owner: true },
        { id: "alex", name: "Alex", owner: false },
        { id: "mia", name: "Mia", owner: false },
        { id: "sam", name: "Sam", owner: false },
      ],
      heads: {},
      locks: [],
      sequence: 1,
      presence: [
        { member: "alex", status: { game: true } },
        { member: "mia", status: null },
      ],
    },
    sections: [],
  };
  render(
    <ProjectPage
      state={state}
      busy=""
      run={vi.fn()}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
    />,
  );
  const card = (name: string) => screen.getByRole("article", { name });
  expect(card("Alex’s work")).toHaveTextContent("In Miliastra");
  expect(card("Mia’s work")).toHaveTextContent("Online");
  expect(card("Sam’s work")).not.toHaveTextContent(/Online|In Miliastra/);
  expect(card("Your work")).toHaveTextContent("Online");
});
it("tells a removed creator and offers to leave the workspace", async () => {
  render(
    <ProjectPage
      state={{
        ...empty,
        removed: true,
        member: "me",
        project: {
          id: "p",
          name: "Fisch",
          storage: { kind: "folder" as const, location: "" },
          members: [],
          heads: {},
          locks: [],
          sequence: 1,
        },
      }}
      busy=""
      run={async (_, action) => {
        await action();
      }}
      onSetup={() => {}}
      onShare={() => {}}
      onHistory={() => {}}
    />,
  );
  expect(screen.getByText("You were removed from this project")).toBeVisible();
  expect(screen.queryByText("Project connection unavailable")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Leave this project" }));
  await waitFor(() => expect(call).toHaveBeenCalledWith("forget_project"));
});

it.each(["busy", "offline", "reload"] as const)(
  "disables unchanged-slice release while %s",
  (reason) => {
    const section = {
      id: "0",
      name: "Boats",
      changed: false,
      conflict: false,
      installed: true,
      downloaded: true,
      installationPending: reason === "reload",
      lock: {
        section: "0",
        member: "me",
        name: "Robin",
        fence: 1,
        expires: 9999999999,
      },
    };
    render(
      <ProjectPage
        state={{
          ...empty,
          member: "me",
          online: reason !== "offline",
          project: {
            id: "p",
            name: "Fisch",
            storage: { kind: "folder", location: "" },
            members: [],
            heads: {},
            locks: [section.lock],
            sequence: 1,
          },
          sections: [section],
        }}
        busy={reason === "busy" ? "Sharing update" : ""}
        run={vi.fn()}
        onSetup={vi.fn()}
        onShare={vi.fn()}
        onHistory={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Release" })).toBeDisabled();
  },
);

it("disables the header Share action for clean slices and enables it when saved work exists", () => {
  const section = {
    id: "5",
    name: "Boats",
    changed: false,
    conflict: false,
    lock: null,
    downloaded: true,
    installed: true,
  };
  const state = {
    ...empty,
    online: true,
    project: {
      id: "p",
      name: "P",
      storage: { kind: "folder" as const, location: "" },
      members: [],
      heads: {},
      locks: [],
      sequence: 1,
    },
    sections: [section],
  };
  const share = vi.fn(),
    props = {
      state,
      busy: "",
      run: vi.fn(),
      onSetup: vi.fn(),
      onShare: share,
      onHistory: vi.fn(),
    };
  const view = render(<ProjectPage {...props} />);
  expect(screen.queryByText(/open invitations?|expires in/i)).toBeNull();
  const button = screen.getByRole("button", {
    name: "Share update (0 slices)",
  });
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(share).not.toHaveBeenCalled();
  view.rerender(
    <ProjectPage
      {...props}
      state={{ ...state, sections: [{ ...section, changed: true }] }}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Share update (1 slices)" }),
  );
  expect(share).toHaveBeenCalledOnce();
});

it("keeps creator-written Share text while showing an optional asset review", async () => {
  vi.mocked(call).mockResolvedValue({
    section: "5",
    assets: [],
    changes: [],
    otherChanges: true,
    baselineAvailable: true,
  });
  const draft = { summary: "My explanation", details: "Keep these details" },
    set = vi.fn();
  render(
    <SharePanel
      state={{
        ...empty,
        online: true,
        sections: [
          {
            id: "5",
            name: "Boats",
            changed: true,
            conflict: false,
            lock: null,
            downloaded: true,
            installed: true,
          },
        ],
      }}
      busy=""
      draft={draft}
      setDraft={set}
      run={vi.fn()}
      onClose={vi.fn()}
    />,
  );
  const details = screen.getByText("Changes").closest("details")!;
  details.open = true;
  fireEvent(details, new Event("toggle"));
  expect(
    await screen.findByText(/Cannot identify specific changes/),
  ).toBeVisible();
  expect(screen.getByDisplayValue("My explanation")).toBeVisible();
  expect(set).not.toHaveBeenCalled();
});
it("opens the asset move helper directly from the slice list", async () => {
  vi.mocked(call).mockResolvedValue([]);
  render(
    <ProjectPage
      state={{
        ...empty,
        project: {
          id: "p",
          name: "P",
          storage: { kind: "folder", location: "" },
          members: [],
          locks: [],
          heads: {},
          sequence: 0,
        },
      }}
      busy=""
      run={vi.fn()}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
    />,
  );
  expect(
    screen.queryByText(/Automatic reservations happen after you save/),
  ).toBeNull();
  expect(screen.queryByRole("button", { name: "Got it" })).toBeNull();
  expect(screen.queryByText("Tools")).toBeNull();
  const move = screen.getByRole("button", {
    name: "Move asset between slices",
  });
  expect(move).toBeVisible();
  expect(move).toBeEnabled();
  expect(move.closest("details")).toBeNull();
  fireEvent.click(move);
  expect(
    screen.getByRole("dialog", { name: "Move asset between slices" }),
  ).toBeVisible();
});

it("keeps every cached conflict review entry point available offline", () => {
  const review = vi.fn();
  render(
    <ProjectPage
      state={{
        ...empty,
        online: false,
        member: "me",
        project: {
          id: "p",
          name: "P",
          storage: { kind: "folder", location: "" },
          members: [],
          heads: {},
          locks: [],
          sequence: 1,
        },
        sections: [
          {
            id: "5",
            name: "Boats",
            changed: true,
            conflict: true,
            lock: null,
            downloaded: true,
            installed: false,
          },
        ],
      }}
      busy=""
      run={vi.fn()}
      onSetup={vi.fn()}
      onShare={vi.fn()}
      onHistory={vi.fn()}
      onConflict={review}
    />,
  );
  const buttons = screen.getAllByRole("button", { name: "Review conflict" });
  expect(buttons.length).toBeGreaterThan(0);
  for (const button of buttons) {
    expect(button).toBeEnabled();
    fireEvent.click(button);
  }
  expect(review).toHaveBeenCalledWith("5");
});

it("shows initial comparisons as checking while preserving independently confirmed conflicts", () => {
  const sections = Array.from({ length: 65 }, (_, i) => ({
    id: i === 64 ? "shared" : String(i),
    name: i === 64 ? "Shared level settings" : `Slice ${i + 1}`,
    changed: false,
    conflict: false,
    comparisonPending: true,
    lock: null,
    downloaded: false,
    installed: false,
  }));
  const state = {
    ...empty,
    member: "me",
    online: true,
    sections,
    project: {
      id: "pending-project",
      name: "Project",
      storage: { kind: "folder", location: "" },
      members: [],
      locks: [],
      heads: {},
      sequence: 1,
    },
  };
  const props = {
    busy: "",
    run: vi.fn(),
    onSetup: vi.fn(),
    onShare: vi.fn(),
    onHistory: vi.fn(),
  };
  const view = render(<ProjectPage {...props} state={state} />);
  expect(screen.getByText("Checking saved slices")).toBeVisible();
  expect(screen.getByText(/65 remaining/)).toBeVisible();
  expect(screen.queryByText(/changed by you and a teammate/)).toBeNull();
  expect(screen.queryByRole("button", { name: "Review conflict" })).toBeNull();
  expect(screen.queryByText(/Getting .*updates? from/)).toBeNull();
  const withConflict = {
    ...state,
    sections: sections.map((s, i) =>
      i === 0
        ? {
            ...s,
            comparisonPending: false,
            changed: true,
            conflict: true,
            downloaded: true,
          }
        : s,
    ),
  };
  view.rerender(<ProjectPage {...props} state={withConflict} />);
  expect(
    screen.getByText("You and a teammate both changed Slice 1"),
  ).toBeVisible();
  expect(screen.getByText(/64 remaining/)).toBeVisible();
  const checked = {
    ...state,
    sections: sections.map((s) => ({
      ...s,
      comparisonPending: false,
      downloaded: true,
      installed: true,
    })),
  };
  view.rerender(<ProjectPage {...props} state={checked} />);
  expect(screen.queryByText("Checking saved slices")).toBeNull();
  expect(screen.queryByRole("button", { name: "Review conflict" })).toBeNull();
  expect(screen.getAllByText("Up to date")).toHaveLength(65);
});

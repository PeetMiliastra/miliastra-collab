export type Revision = {
  section: string;
  name: string;
  hash: string;
  contentHash: string;
  parent: string | null;
  artifact: string;
  size: number;
  sharedDependency?: string | null;
};
type Lease = {
  section: string;
  member: string;
  name: string;
  fence: number;
  expires: number;
};
export type Member = {
  id: string;
  name: string;
  owner: boolean;
  joined?: number;
};
export type Project = {
  id: string;
  name: string;
  storage: {
    kind: string;
    location: string;
    driveId?: string | null;
  };
  members: Member[];
  locks: Lease[];
  heads: Record<string, Revision>;
  sequence: number;
  sectionNames?: Record<string, string>;
  namesVersion?: number;
  /** Members with the app open, and whether they're in Miliastra. */
  presence?: { member: string; status?: { game?: boolean } | null }[];
};
export type Stage = {
  path: string;
  level: string;
  account: string;
  name: string;
  sections: { id: string; name: string }[];
  lastSavedSection: string | null;
  settingsHash: string;
  settingsVersion?: string;
  editorVersion?: string;
};
export type Draft = { summary: string; details: string };
export type Update = {
  id: string;
  author: string;
  authorId: string;
  summary: string;
  details: string;
  at: number;
  sequence: number;
  revisions: Revision[];
  restores: string | null;
  nameChanges?: NameChange[];
  namesParent?: number | null;
};
export type Settings = {
  endpoint: string;
  saveRoot: string;
  oauth: {
    microsoftClientId: string;
    googleClientId: string;
    googleClientSecret?: string;
  };
  autostart: boolean;
  screenRecognition: boolean;
  autoApply?: boolean;
};
export type Section = {
  id: string;
  name: string;
  changed: boolean;
  conflict: boolean;
  /** Reference bytes are still needed to compare this saved slice. */
  comparisonPending?: boolean;
  lock: Lease | null;
  downloaded: boolean;
  downloadError?: string | null;
  installed: boolean;
  installationPending?: boolean;
  incoming?: { author: string; summary: string; at: number } | null;
};
export type StorageAccounts = Record<
  string,
  "connected" | "notConnected" | "reconnect" | "unavailable"
>;
export type Snapshot = {
  metadataPending?: boolean;
  /** What's wrong with storage, without URLs or raw errors. */
  storageIssue?:
    | "reconnect"
    | "access"
    | "missing"
    | "network"
    | "busy"
    | "validation"
    | "unknown"
    | null;
  /** Set when this instance runs as a test profile (`--profile <name>`). */
  testProfile?: string | null;
  cloudProviders?: string[];
  task?: WorkflowTask | null;
  storageAccounts?: StorageAccounts;
  /** Set when the owner has removed you from this project. */
  removed?: boolean;
  settings: Settings;
  stage: Stage | null;
  project: Project | null;
  member: string | null;
  sections: Section[];
  draft: Draft;
  online: boolean;
  /** A verified coordinator response reported an incompatible contract. */
  protocolMismatch: boolean;
  message: string;
  compatibility: {
    directInstall: boolean;
    liveDetection: boolean;
    restoreVersions: boolean;
    reason: string;
  };
  game: {
    running: boolean;
    foreground: boolean;
    exclusiveFullscreen?: boolean;
  };
  installation?: Installation | null;
  nameSync?: {
    pending: NameChange[];
    draft: NamesDraft;
    error?: string | null;
  } | null;
};
export type Invitation = {
  version: number;
  endpoint: string;
  project: string;
  invite: string;
};
export type HistoryPage = { entries: Update[]; next: number | null };
export type Run = (
  label: string,
  action: () => Promise<unknown>,
  options?: { surface?: "page" | "modal"; onError?: (message: string) => void },
) => Promise<void>;

export type InstallPlan = { id: string; revisions: Revision[] };
export type Installation = {
  localRevert?: boolean;
  id: string;
  phase:
    | "installing"
    | "awaitingReload"
    | "verified"
    | "interrupted"
    | "restoring"
    | "undoReload"
    | "undone";
  journal: string;
  revisions: Revision[];
  error: string | null;
};

type NameChange = { section: string; before: string; after: string };
export type NamesDraft = {
  edits: Record<
    string,
    { name: string; baseName: string; parentVersion: number }
  >;
};

export type ConflictPreview = {
  id: string;
  project: string;
  section: string;
  name: string;
  mine: Revision;
  theirs: Revision;
  baseline: Revision | null;
  folder: string;
  at: number;
  latestUpdate: Update | null;
  canKeepMine: boolean;
  canUseTheirs: boolean;
  reason: string;
  draft: Draft;
};

export type WorkflowTask = {
  kind: "share" | "apply";
  selected: string[];
  manualSelection: boolean;
  installationId?: string | null;
};

export type Asset = {
  key: string;
  id: number;
  name: string;
  kind: string;
  section: number;
  canGenerateId: boolean;
};
export type SliceInspection = {
  section: string;
  assets: Asset[];
  changes: {
    asset: Asset;
    status: "Added" | "Modified" | "Deleted";
    beforeName: string | null;
  }[];
  otherChanges: boolean;
  baselineAvailable: boolean;
};
export type RevertPreview = { id: string; section: string; name: string };
export type MoveId = {
  token: string;
  oldId: number;
  newId: number;
  name: string;
  source: string;
  target: string;
  expires: number;
};

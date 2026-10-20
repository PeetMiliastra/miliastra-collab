import type { Section, Snapshot, Update } from "./types";
export function canShare(summary: string, sections: string[], online: boolean) {
  return Boolean(summary.trim()) && sections.length > 0 && online;
}
export function updateStatus(u: Update, s: Snapshot): string {
  if (u.nameChanges?.length && !u.revisions.length) {
    const current = u.nameChanges.filter(
      (c) => s.project?.sectionNames?.[c.section] === c.after,
    );
    if (!current.length) return "Superseded";
    return current.every(
      (c) =>
        s.stage?.sections.find((x) => x.id === c.section)?.name === c.after,
    )
      ? "Names applied"
      : "Names synced";
  }
  if (u.revisions.every((r) => s.project?.heads[r.section]?.hash !== r.hash))
    return "Superseded";
  const current = u.revisions.filter(
    (r) => s.project?.heads[r.section]?.hash === r.hash,
  );
  if (
    current.some(
      (r) => s.sections.find((x) => x.id === r.section)?.installationPending,
    )
  )
    return "Checking files";
  if (current.some((r) => s.sections.find((x) => x.id === r.section)?.conflict))
    return "Both changed";
  if (
    current.some(
      (r) => s.sections.find((x) => x.id === r.section)?.downloadError,
    )
  )
    return "Download paused";
  if (
    current.every((r) => s.sections.find((x) => x.id === r.section)?.installed)
  )
    return "Applied";
  if (
    current.every((r) => s.sections.find((x) => x.id === r.section)?.downloaded)
  )
    return "Ready to apply";
  return "Downloading";
}
export function friendlyError(e: unknown) {
  return String(e).replace(/^Error: /, "");
}
export function initials(name: string) {
  return name
    .trim()
    .split(/\s+/)
    .slice(0, 2)
    .map((x) => x[0] ?? "")
    .join("")
    .toUpperCase();
}

// Only for display; the app checks again when it builds the install preview.
export function installationBlocker(state: Snapshot): string | null {
  if (state.sections.some((s) => s.comparisonPending))
    return "Wait for saved slices to finish checking.";
  if (state.metadataPending)
    return "Waiting for update details. Check your connection to the team folder, then choose Check for changes.";
  if (
    state.installation &&
    !["verified", "undone"].includes(state.installation.phase)
  )
    return "Finish applying the current update first.";
  const updates = state.sections.filter((s) => !s.installed);
  if (!updates.length) return "There are no new slice updates.";
  if (!state.compatibility.directInstall) return state.compatibility.reason;
  const conflicts = updates.filter((s) => s.conflict);
  if (conflicts.length)
    return `Review ${conflicts.map((s) => s.name).join(", ")} before applying updates.`;
  const dirty = updates.filter((s) => s.changed);
  if (dirty.length)
    return `Share or preserve your changes to ${dirty.map((s) => s.name).join(", ")} before applying.`;
  if (updates.some((s) => s.downloadError))
    return "A download is paused. Check your storage connection; downloads retry automatically.";
  if (updates.some((s) => !s.downloaded))
    return "Wait for all slices in this update to finish downloading.";
  return null;
}

/** Plain-language row status for the slice list. */
export function sliceStatus(s: Section): {
  label: string;
  tone: "clean" | "amber" | "blue";
} {
  if (s.installationPending) return { label: "Checking files", tone: "blue" };
  if (s.comparisonPending)
    return {
      label: s.downloadError ? "Check paused" : "Checking",
      tone: "blue",
    };
  if (s.conflict) return { label: "Both changed", tone: "amber" };
  if (s.changed) return { label: "Your changes", tone: "amber" };
  if (!s.installed)
    return {
      label: s.downloaded
        ? "Ready to apply"
        : s.downloadError
          ? "Download paused"
          : "Downloading",
      tone: "blue",
    };
  return { label: "Up to date", tone: "clean" };
}

type NextStep = "install" | "conflict" | "apply" | "share" | null;
/** The single most useful action right now, in priority order. */
export function nextStep(state: Snapshot): NextStep {
  if (
    state.installation &&
    !["verified", "undone"].includes(state.installation.phase)
  )
    return "install";
  if (state.sections.some((s) => s.conflict)) return "conflict";
  if (
    state.sections.some((s) => !s.installed) &&
    installationBlocker(state) === null
  )
    return "apply";
  if (state.sections.some((s) => s.changed && !s.conflict)) return "share";
  return null;
}

/** "Alex", "Alex and Mia", "Alex, Mia and 2 others". */
export function people(names: string[]): string {
  const unique = [...new Set(names)];
  if (unique.length <= 1) return unique[0] ?? "a teammate";
  if (unique.length === 2) return `${unique[0]} and ${unique[1]}`;
  if (unique.length === 3) return `${unique[0]}, ${unique[1]} and ${unique[2]}`;
  return `${unique[0]}, ${unique[1]} and ${unique.length - 2} others`;
}

// @vitest-environment jsdom
import { afterEach, it, expect, vi } from "vitest";
import {
  render,
  screen,
  fireEvent,
  cleanup,
  waitFor,
  within,
} from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import {
  SavedAssetReview,
  ConflictAssetReview,
  AssetSummary,
} from "./AssetReview";
import type { SliceInspection } from "./types";
const native = vi.hoisted(() => vi.fn());
vi.mock("./bridge", () => ({ call: native }));
afterEach(() => {
  cleanup();
  native.mockReset();
});
const asset = {
  key: "Entity:1",
  id: 1,
  name: "Boat deck",
  kind: "Entity",
  section: 5,
  canGenerateId: true,
};
const data: SliceInspection = {
  section: "5",
  assets: [asset],
  changes: [{ asset, status: "Modified", beforeName: "Old deck" }],
  otherChanges: true,
  baselineAvailable: true,
};
function expand(text: string) {
  const details = screen.getByText(text).closest("details")!;
  details.open = true;
  fireEvent(details, new Event("toggle"));
}
it("reads only expanded saved reviews and refreshes the snapshot without generating changelog text", async () => {
  native.mockResolvedValue(data);
  render(<SavedAssetReview section="5" disabled={false} />);
  expect(native).not.toHaveBeenCalled();
  expand("Changes");
  expect(await screen.findByText("Boat deck")).toBeVisible();
  expect(screen.getByText("1 modified")).toBeVisible();
  expect(screen.getByText("Other unidentified changes")).toBeVisible();
  expect(screen.queryByText("Cannot identify specific changes")).toBeNull();
  expect(native).toHaveBeenCalledExactlyOnceWith("inspect_slice", {
    section: "5",
  });
  native.mockResolvedValue({ ...data, changes: [] });
  fireEvent.click(screen.getByRole("button", { name: "Refresh asset review" }));
  await waitFor(() => expect(screen.queryByText("Boat deck")).toBeNull());
  expect(screen.getByText("Cannot identify specific changes")).toBeVisible();
  expect(screen.queryByText("Other unidentified changes")).toBeNull();
  expect(native.mock.calls.every(([cmd]) => cmd === "inspect_slice")).toBe(
    true,
  );
});
it("shows both preserved versions separately and explains whole-slice selection", async () => {
  native.mockResolvedValue({
    mine: data,
    theirs: {
      ...data,
      changes: [
        {
          asset: { ...asset, name: "Dock ladder" },
          status: "Added",
          beforeName: null,
        },
      ],
    },
  });
  render(<ConflictAssetReview previewId="review" />);
  expand("Compare saved asset changes");
  expect(await screen.findByText("Boat deck")).toBeVisible();
  expect(
    within(screen.getByRole("region", { name: "My asset changes" })).getByText(
      "Boat deck",
    ),
  ).toBeVisible();
  expect(
    within(
      screen.getByRole("region", { name: "Teammate’s asset changes" }),
    ).getByText("Dock ladder"),
  ).toBeVisible();
  expect(screen.getByText(/assets are not merged/)).toBeVisible();
  expect(native).toHaveBeenCalledExactlyOnceWith("inspect_conflict", {
    previewId: "review",
  });
});
it("allows a failed optional review to retry without mutating versions", async () => {
  native
    .mockRejectedValueOnce(Error("Preserved version failed validation"))
    .mockResolvedValue({ mine: data, theirs: data });
  render(<ConflictAssetReview previewId="review" />);
  expand("Compare saved asset changes");
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Open the preserved copies",
  );
  fireEvent.click(
    screen.getByRole("button", { name: "Retry asset comparison" }),
  );
  await waitFor(() => expect(screen.getAllByText("Boat deck")).toHaveLength(2));
  expect(native.mock.calls.every(([cmd]) => cmd === "inspect_conflict")).toBe(
    true,
  );
});
it("lists changes without relocation actions or invented changes without a baseline", () => {
  const view = render(<AssetSummary data={data} />);
  expect(screen.getByText("Boat deck")).toBeVisible();
  expect(screen.getByText("ID: 1")).toBeVisible();
  expect(view.container.querySelector("details")).toBeNull();
  expect(screen.queryByRole("button")).toBeNull();
  view.rerender(
    <AssetSummary data={{ ...data, changes: [], baselineAvailable: false }} />,
  );
  expect(screen.getByText(/previous version is not available/)).toBeVisible();
  expect(screen.queryByText(/No saved asset changes/)).toBeNull();
});

it("separates change counts with commas", () => {
  render(
    <AssetSummary
      data={{
        ...data,
        changes: (["Added", "Modified", "Deleted"] as const).map(
          (status, index) => ({
            asset: { ...asset, key: `Entity:${index}`, id: index },
            status,
            beforeName: null,
          }),
        ),
      }}
    />,
  );
  expect(screen.getByText("1 added, 1 modified, 1 deleted")).toBeVisible();
});

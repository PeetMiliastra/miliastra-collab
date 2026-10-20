// Slice tools (revert, move IDs) in a throwaway profile.
const { chromium } = require("playwright"),
  assert = require("node:assert/strict"),
  fs = require("node:fs"),
  path = require("node:path");
const { sha } = require("./reliability/fixtures.cjs");
const vi = (value) => {
  const out = [];
  do {
    let b = value % 128;
    value = Math.floor(value / 128);
    out.push(b | (value ? 128 : 0));
  } while (value);
  return Buffer.from(out);
};
const scalar = (n, v) => Buffer.concat([vi(n * 8), vi(v)]),
  field = (n, b) => {
    b = Buffer.from(b);
    return Buffer.concat([vi(n * 8 + 2), vi(b.length), b]);
  };
const frame = (kind, b) => {
  const h = Buffer.alloc(20),
    t = Buffer.alloc(4);
  [b.length + 20, 1, 806, kind, b.length].forEach((n, i) =>
    h.writeUInt32BE(n, i * 4),
  );
  t.writeUInt32BE(1657);
  return Buffer.concat([h, b, t]);
};
const named = (section, id, name, reference) =>
  frame(
    6,
    Buffer.concat([
      scalar(1, section),
      field(
        2,
        Buffer.concat([
          scalar(1, 42),
          field(
            5,
            field(
              1,
              Buffer.concat([
                scalar(1, id),
                Buffer.concat([
                  field(5, scalar(1, 13)),
                  field(
                    5,
                    Buffer.concat([scalar(1, 1), field(11, field(1, name))]),
                  ),
                  field(5, scalar(1, 14)),
                ]),
              ]),
            ),
          ),
          ...(reference ? [scalar(91, reference)] : []),
        ]),
      ),
    ]),
  );
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(
    path.basename(root).startsWith("slice-tools-native-") &&
      path.basename(path.dirname(root)) === ".tools",
  );
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19238");
  try {
    const page = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => !p.url().includes("overlay=1"));
    assert(page);
    page.setDefaultTimeout(15000);
    await page.locator(".brand").waitFor();
    const call = (cmd, args = {}) =>
      page.evaluate(
        ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
        { cmd, args },
      );
    const poll = async (predicate) => {
      for (let n = 0; n < 240; n++) {
        const s = await call("snapshot");
        if (predicate(s)) return s;
        await new Promise((r) => setTimeout(r, 250));
      }
      throw Error("Synthetic project timed out");
    };
    assert.equal(
      path.resolve((await call("snapshot")).testDataDirectory),
      root,
    );
    await call("begin_practice");
    let state = await poll(
      (s) =>
        s.practice &&
        !s.metadataPending &&
        s.project.sequence === 80 &&
        s.sections.every((x) => x.installed || x.downloaded),
    );
    await call("save_settings", {
      settings: { ...state.settings, autoApply: false },
    });
    const local = (p) =>
      p.startsWith(String.fromCharCode(92, 92, 63, 92)) ? p.slice(4) : p;
    const stage = local(state.stage.path);
    assert(stage.startsWith(root + path.sep));
    const file = (section) =>
      path.join(
        stage,
        "beyond_level_slice",
        path.basename(stage) + "_" + (section + 1) + ".gis",
      );
    assert(state.sections.find((s) => s.id === "5").installed);
    const before = fs.readFileSync(
        path.join(root, "objects", state.project.heads["5"].hash),
      ),
      targetBefore = fs.readFileSync(file(6)),
      unrelatedBefore = fs.readFileSync(file(60)),
      settingsFile = path.join(
        stage,
        "beyond_level_setting/local_setting.gils",
      ),
      settingsHash = sha(fs.readFileSync(settingsFile));
    const sequence = state.project.sequence;
    await call("start_work", { section: "5", takeover: false });
    await call("save_draft", {
      projectId: state.project.id,
      draft: {
        summary: "Keep my private draft",
        details: "Revert must not discard it",
      },
    });
    const originalId = 0x40000000 + (1 << 22) + (5 << 16) + 1,
      candidate = 0x40000000 + (1 << 22) + (6 << 16) + 1;
    let edited = named(5, originalId, "Private Boat Deck");
    fs.writeFileSync(file(5), edited);
    await call("refresh_saved_changes");
    const inspection = await call("inspect_slice", { section: "5" });
    assert(inspection.baselineAvailable);
    assert(
      inspection.changes.some(
        (c) =>
          c.status === "Added" &&
          c.asset.name === "Private Boat Deck" &&
          c.asset.id === originalId,
      ),
    );
    const sourceName = state.sections.find((s) => s.id === "5").name;
    await page
      .getByRole("button", { name: "Project", exact: true })
      .click()
      .catch(() => {});
    const row = page.locator("tbody tr").filter({ hasText: sourceName });
    await row
      .getByRole("button", { name: "View changes", exact: true })
      .click();
    await page
      .getByRole("dialog", { name: "Changes in " + sourceName })
      .getByText("Private Boat Deck", { exact: true })
      .waitFor();
    await page.screenshot({ path: path.join(root, "asset-changes.png") });
    const changesDialog = page.getByRole("dialog", {
      name: "Changes in " + sourceName,
    });
    assert.equal(
      await changesDialog.getByRole("button", { name: /Move|GUID/ }).count(),
      0,
    );
    await changesDialog.getByText("Close", { exact: true }).click();
    await page.getByRole("button", { name: /^Share update \(/ }).click();
    const share = page.getByRole("dialog", {
      name: "Share update",
      exact: true,
    });
    await share
      .getByPlaceholder("Fixed passengers falling through the boat deck")
      .fill("Keep my private draft");
    const shareSlice = share
      .locator(".share-slice-review")
      .filter({ hasText: sourceName });
    await shareSlice.getByText("Changes", { exact: true }).click();
    await shareSlice.getByText("Private Boat Deck", { exact: true }).waitFor();
    assert.equal(
      await share
        .getByPlaceholder("Fixed passengers falling through the boat deck")
        .inputValue(),
      "Keep my private draft",
    );
    const shareButtonBounds = await share
      .getByRole("button", { name: "Share update", exact: true })
      .boundingBox();
    assert(
      shareButtonBounds &&
        shareButtonBounds.y >= 0 &&
        shareButtonBounds.y + shareButtonBounds.height <=
          (await page.evaluate(() => innerHeight)),
      "Share action must stay visible while reviewing assets",
    );
    await page.screenshot({ path: path.join(root, "share-asset-review.png") });
    await share.getByRole("button", { name: "Cancel", exact: true }).click();
    await row.getByRole("button", { name: "Revert", exact: true }).click();
    const dialog = page.getByRole("dialog", {
      name: "Revert " + sourceName + "?",
    });
    await dialog.getByRole("button", { name: "Back up and revert" }).waitFor();
    assert(
      await dialog
        .getByRole("button", { name: "Back up and revert" })
        .isDisabled(),
    );
    assert.equal(await dialog.getByRole("status").count(), 0);
    assert.equal(await dialog.getByRole("alert").count(), 0);
    await dialog.getByRole("checkbox").check();
    assert(
      await dialog
        .getByRole("button", { name: "Back up and revert" })
        .isEnabled(),
    );
    await dialog.getByRole("button", { name: "Cancel" }).click();
    assert.deepEqual(fs.readFileSync(file(5)), edited);
    let preview = await call("preview_revert", { section: "5" });
    edited = named(5, originalId, "Private Boat Deck renamed");
    fs.writeFileSync(file(5), edited);
    await assert.rejects(
      call("revert_slice", { previewId: preview.id, editorReady: true }),
      /changed after confirmation/,
    );
    assert.deepEqual(fs.readFileSync(file(5)), edited);
    preview = await call("preview_revert", { section: "5" });
    await assert.rejects(
      call("revert_slice", { previewId: preview.id, editorReady: false }),
      /unload/,
    );
    await call("revert_slice", { previewId: preview.id, editorReady: true });
    assert.deepEqual(fs.readFileSync(file(5)), before);
    state = await call("snapshot");
    assert(
      state.installation.localRevert && state.installation.phase === "verified",
    );
    assert.equal(state.project.sequence, sequence);

    state = await call("snapshot");
    assert(!state.sections.find((s) => s.id === "5").changed);
    assert.equal(state.draft.summary, "Keep my private draft");
    await call("undo_install", { editorReady: true });

    state = await call("snapshot");
    assert(state.sections.find((s) => s.id === "5").changed);
    assert.deepEqual(fs.readFileSync(file(5)), edited);
    assert.equal(state.project.sequence, sequence);
    // An ID used anywhere, even in another slice, can't be handed out again.
    fs.writeFileSync(
      file(60),
      named(
        60,
        0x40000000 + (1 << 22) + (60 << 16) + 1,
        "Unrelated",
        candidate,
      ),
    );
    // Waiting updates don't block the ID calculator.
    state = await call("snapshot");
    assert(
      !state.sections.find((s) => s.id === "6").installed,
      "Fixture must retain an unapplied incoming update",
    );
    const changesBeforeId = state.sections.map(({ id, changed }) => [
      id,
      changed,
    ]);
    const locksBeforeId = state.project.locks
      .map((l) => [l.section, l.member, l.fence])
      .sort();
    const hashes = new Map(
      [5, 6, 60].map((n) => [n, sha(fs.readFileSync(file(n)))]),
    );
    const allSliceHashes = new Map(
      fs
        .readdirSync(path.dirname(file(5)))
        .map((name) => [
          name,
          sha(fs.readFileSync(path.join(path.dirname(file(5)), name))),
        ]),
    );
    const generated = await call("generate_move_id", {
      source: "5",
      assetKey: "Entity:" + originalId,
      target: "6",
    });
    assert.equal(generated.oldId, originalId);
    assert.equal((generated.newId >>> 16) & 63, 6);
    assert.equal(generated.newId >>> 22, originalId >>> 22);
    assert.notEqual(generated.newId, candidate);
    assert.equal(
      await call("validate_move_id", { token: generated.token }),
      String(generated.newId),
    );
    const again = await call("generate_move_id", {
      source: "5",
      assetKey: "Entity:" + originalId,
      target: "6",
    });
    assert.notEqual(again.newId, generated.newId);
    for (const [n, h] of hashes)
      assert.equal(
        sha(fs.readFileSync(file(n))),
        h,
        "ID helper modified a game file",
      );
    for (const [name, hash] of allSliceHashes)
      assert.equal(
        sha(fs.readFileSync(path.join(path.dirname(file(5)), name))),
        hash,
        "GUID generation changed a saved slice or shared settings",
      );
    await call("refresh_saved_changes");
    state = await call("snapshot");
    assert.deepEqual(
      state.sections.map(({ id, changed }) => [id, changed]),
      changesBeforeId,
      "Generating a GUID changed saved-change status",
    );
    assert.deepEqual(
      state.project.locks.map((l) => [l.section, l.member, l.fence]).sort(),
      locksBeforeId,
      "ID helper changed reservations",
    );
    await page
      .getByRole("button", { name: "Move asset between slices", exact: true })
      .click();
    const move = page.getByRole("dialog", {
      name: "Move asset between slices",
    });
    await move
      .getByText("Changing the GUID breaks any references to this asset.", {
        exact: true,
      })
      .waitFor();
    await move.getByLabel("From slice", { exact: true }).selectOption("5");
    await move
      .getByRole("button", {
        name: `Private Boat Deck renamed — Entity (${originalId})`,
        exact: true,
      })
      .click();
    await move.getByLabel("To slice", { exact: true }).selectOption("6");
    await move
      .getByRole("button", { name: "Generate GUID", exact: true })
      .click();
    await move.getByLabel("New GUID", { exact: true }).waitFor();
    await page.screenshot({ path: path.join(root, "move-helper.png") });
    fs.writeFileSync(file(6), named(6, candidate, "Later local edit"));
    await assert.rejects(
      call("validate_move_id", { token: again.token }),
      /slices changed/,
    );
    await move.getByRole("button", { name: "Copy GUID", exact: true }).click();
    await move
      .getByRole("alert")
      .filter({ hasText: "slices changed" })
      .waitFor();
    assert.deepEqual(fs.readFileSync(file(5)), edited);
    assert.equal(sha(fs.readFileSync(settingsFile)), settingsHash);
    assert.equal((await call("snapshot")).project.sequence, sequence);
    // Sharing only the shared settings moves them forward without changing which shared
    // settings this slice's earlier update depended on.
    await move.getByText("Close", { exact: true }).click();
    const sharedPath = path.join(
      stage,
      "beyond_level_slice",
      path.basename(stage) + ".gilh",
    );
    const oldShared = fs.readFileSync(sharedPath);
    const sharedUpdate = frame(
      7,
      Buffer.concat([
        oldShared.subarray(20, -4),
        field(200, "Synthetic shared settings update"),
      ]),
    );
    await call("start_work", { section: "shared", takeover: false });
    fs.writeFileSync(sharedPath, sharedUpdate);
    await call("refresh_saved_changes");
    await call("share_update", {
      sections: ["shared"],
      draft: {
        summary: "Synthetic shared settings fixture update",
        details: "",
      },
      keepReserved: false,
    });
    const afterShared = await call("snapshot");
    assert.equal(afterShared.project.sequence, sequence + 1);
    const sharedHash = sha(sharedUpdate),
      beforeLocalRevert = fs.readFileSync(file(5));
    assert.equal(afterShared.project.heads.shared.hash, sharedHash);
    const sharedContextPreview = await call("preview_revert", { section: "5" });
    await call("revert_slice", {
      previewId: sharedContextPreview.id,
      editorReady: true,
    });

    assert.deepEqual(
      fs.readFileSync(file(5)),
      before,
      "Revert after a shared-only update must restore the applied slice",
    );
    assert.equal(
      sha(fs.readFileSync(sharedPath)),
      sharedHash,
      "Local Revert must not replace shared settings",
    );
    await call("undo_install", { editorReady: true });

    assert.deepEqual(
      fs.readFileSync(file(5)),
      beforeLocalRevert,
      "Undo must restore the unshared slice edits in the updated shared context",
    );
    assert.equal(sha(fs.readFileSync(sharedPath)), sharedHash);
    assert.equal(
      (await call("snapshot")).project.sequence,
      sequence + 1,
      "Revert and Undo must not publish",
    );
    // Compare both versions in the conflict screen, without resolving.
    const conflictId = 0x40000000 + (1 << 22) + (8 << 16) + 1;
    fs.writeFileSync(file(8), named(8, conflictId, "Private Quest Marker"));
    await call("refresh_saved_changes");
    const conflictPreview = await call("preview_conflict", { section: "8" }),
      comparison = await call("inspect_conflict", {
        previewId: conflictPreview.id,
      });
    assert(
      comparison.mine.baselineAvailable && comparison.theirs.baselineAvailable,
    );
    assert(
      comparison.mine.changes.some(
        (c) => c.asset.name === "Private Quest Marker",
      ),
    );
    assert(comparison.theirs.otherChanges);
    state = await call("snapshot");
    const questName = state.sections.find((s) => s.id === "8").name,
      questRow = page.locator("tbody tr").filter({ hasText: questName });
    await questRow
      .getByRole("button", { name: "Review conflict", exact: true })
      .click();
    const conflict = page.getByRole("dialog", {
      name: "Resolve " + questName,
      exact: true,
    });
    await conflict
      .getByText("Compare saved asset changes", { exact: true })
      .click();
    await conflict
      .getByRole("region", { name: "My asset changes", exact: true })
      .getByText("Private Quest Marker", { exact: true })
      .waitFor();
    await conflict
      .getByRole("region", { name: "Teammate’s asset changes", exact: true })
      .getByText(/Cannot identify specific changes/)
      .waitFor();
    await page.screenshot({
      path: path.join(root, "conflict-asset-review.png"),
    });
    await conflict
      .getByRole("button", { name: "Keep both for now", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Continue later", exact: true })
      .click();
    // Delete one file from team storage and the cache so its download fails. Retry must
    // reset the backoff.
    state = await call("snapshot");
    const remote = state.project.heads["8"],
      object = path.join(root, "objects", remote.hash);
    // Find the practice project's team folder (its name is a UUID).
    const findExchange = (dir) => {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        if (e.isDirectory()) {
          const next = path.join(dir, e.name);
          if (
            e.name === "exchange" &&
            fs.existsSync(path.join(next, remote.artifact))
          )
            return next;
          const found = findExchange(next);
          if (found) return found;
        }
      }
      return null;
    };
    const teamFolder = findExchange(path.join(root, "practice"));
    assert(teamFolder && teamFolder.startsWith(root + path.sep));
    const artifact = path.join(teamFolder, remote.artifact);
    assert(artifact.startsWith(root + path.sep));
    fs.renameSync(object, object + ".held");
    fs.renameSync(artifact, artifact + ".held");
    try {
      await call("refresh");
      state = await poll((s) => s.storageIssue === "missing");
      assert.equal(state.online, true);
      await page
        .getByText("An update file is not available", { exact: true })
        .scrollIntoViewIfNeeded();
      await page.screenshot({ path: path.join(root, "storage-recovery.png") });
    } finally {
      fs.renameSync(artifact + ".held", artifact);
    }
    await page
      .getByRole("button", { name: "Retry download", exact: true })
      .click();
    await poll(
      (s) => !s.storageIssue && s.sections.find((x) => x.id === "8").downloaded,
    );
    assert.equal(sha(fs.readFileSync(object)), remote.hash);
    assert.equal(
      (await call("snapshot")).project.sequence,
      sequence + 1,
      "Review and retries must not publish",
    );
    await call("practice_conflict");
    state = await poll(
      (s) =>
        !s.metadataPending && s.sections.find((x) => x.id === "8").downloaded,
    );
    assert.equal(
      state.project.sequence,
      sequence + 2,
      "Only explicit fixture publications advance History",
    );
    fs.writeFileSync(file(8), named(8, conflictId, "Offline Quest Marker"));
    // Point the project at an unreachable coordinator to show the ID tools work offline.
    const { execFileSync } = require("node:child_process");
    const rewriteEndpoint = (endpoint) =>
      JSON.parse(
        execFileSync(
          "python",
          [
            "-c",
            `
import pathlib,sys,sqlite3,json
root=pathlib.Path(sys.argv[1]).resolve()
assert root.name.startswith('slice-tools-native-') and root.parent.name=='.tools'
c=sqlite3.connect(root/'collab.db',timeout=15)
with c:
 rows=json.loads(c.execute("select value from kv where key='connections'").fetchone()[0])
 matches=[p for p in rows if p['project']['id']==sys.argv[2]]
 assert len(matches)==1
 old=matches[0]['endpoint'];matches[0]['endpoint']=sys.argv[3]
 c.execute("update kv set value=? where key='connections'",(json.dumps(rows),))
 owned=json.loads(c.execute("select value from kv where key='practice-endpoint'").fetchone()[0])
 assert owned==old
 c.execute("update kv set value=? where key='practice-endpoint'",(json.dumps(sys.argv[3]),))
print(json.dumps(old))
`,
            root,
            state.project.id,
            endpoint,
          ],
          { encoding: "utf8", windowsHide: true },
        ).trim(),
      );
    await call("leave_project"); // stop background sync before editing this synthetic connection
    const originalEndpoint = rewriteEndpoint("http://127.0.0.1:1");
    try {
      await call("reopen_project", { projectId: state.project.id });
      const offline = await call("snapshot");
      assert.equal(offline.online, false);
      const offlineLocks = offline.project.locks
        .map((l) => [l.section, l.member, l.fence])
        .sort();
      const offlineHashes = new Map(
        [5, 6, 60].map((n) => [n, sha(fs.readFileSync(file(n)))]),
      );
      const offlineId = await call("generate_move_id", {
        source: "5",
        assetKey: "Entity:" + originalId,
        target: "6",
      });
      assert.equal(
        await call("validate_move_id", { token: offlineId.token }),
        String(offlineId.newId),
      );
      const after = await call("snapshot");
      assert.equal(after.online, false);
      assert.deepEqual(
        after.project.locks.map((l) => [l.section, l.member, l.fence]).sort(),
        offlineLocks,
      );
      for (const [n, h] of offlineHashes)
        assert.equal(sha(fs.readFileSync(file(n))), h);
      // History, including the first entry, still loads from the cache.
      const history = [];
      let historyBefore = null;
      do {
        const page = await call("history_page", {
          filter: { before: historyBefore, section: null },
        });
        history.push(...page.entries);
        historyBefore = page.next;
      } while (historyBefore !== null);
      assert.equal(history.length, sequence + 2);
      assert(history.some((u) => u.sequence === 1));
      const edits = {
        5: {
          name: "Offline boats",
          baseName: offline.sections.find((s) => s.id === "5").name,
          parentVersion: offline.project.namesVersion,
        },
        6: {
          name: "Offline islands",
          baseName: offline.sections.find((s) => s.id === "6").name,
          parentVersion: offline.project.namesVersion,
        },
      };
      await Promise.all(
        Object.entries(edits).map(([id, edit]) =>
          call("save_names_draft", {
            projectId: offline.project.id,
            draft: { edits: { [id]: edit } },
          }),
        ),
      );
      await call("reopen_project", { projectId: offline.project.id });
      assert.deepEqual(
        (await call("snapshot")).nameSync.draft.edits,
        edits,
        "Offline drafts must survive reopen without overwriting each other",
      );
      for (const [n, h] of offlineHashes)
        assert.equal(
          sha(fs.readFileSync(file(n))),
          h,
          "Drafts changed stage bytes",
        );
      const localQuest = fs.readFileSync(file(8));
      const offlinePreview = await call("preview_conflict", { section: "8" });
      await call("inspect_conflict", { previewId: offlinePreview.id });
      await call("save_conflict_draft", {
        previewId: offlinePreview.id,
        draft: {
          summary: "Offline decision draft",
          details: "Keep this private",
        },
      });
      const recovered = await call("preview_conflict", { section: "8" });
      assert.equal(recovered.draft.summary, "Offline decision draft");
      await assert.rejects(
        call("resolve_conflict", {
          previewId: recovered.id,
          choice: "mine",
          draft: recovered.draft,
          editorReady: true,
        }),
      );
      assert.deepEqual(
        fs.readFileSync(file(8)),
        localQuest,
        "Failed offline Share changed local files",
      );
      // Even offline, local changes still block installing a kept version.
      fs.writeFileSync(file(8), named(8, conflictId, "Later offline quest"));
      await assert.rejects(
        call("resolve_conflict", {
          previewId: recovered.id,
          choice: "theirs",
          draft: recovered.draft,
          editorReady: true,
        }),
        /changed after review/,
      );
      fs.writeFileSync(file(8), localQuest);
      const chosen = await call("preview_conflict", { section: "8" });
      await call("resolve_conflict", {
        previewId: chosen.id,
        choice: "theirs",
        draft: chosen.draft,
        editorReady: true,
      });
      assert.equal(sha(fs.readFileSync(file(8))), chosen.theirs.hash);

      await call("undo_install", { editorReady: true });

      assert.deepEqual(
        fs.readFileSync(file(8)),
        localQuest,
        "Offline Undo did not restore private changes",
      );
      assert.equal(
        (await call("snapshot")).project.sequence,
        sequence + 2,
        "Offline local actions must not publish",
      );
      assert.equal((await call("snapshot")).online, false);
    } finally {
      rewriteEndpoint(originalEndpoint);
      await call("reopen_project", { projectId: state.project.id });
    }
    // The test folder is left in place for inspection.
    fs.writeFileSync(
      path.join(root, "slice-tools-report.json"),
      JSON.stringify(
        {
          status: "passed",
          assetNames: true,
          shareAssetReview: true,
          conflictAssetReview: true,
          assetReviewWithoutMoveActions: true,
          storageRecovery: true,
          retryDownload: true,
          revertConfirmation: true,
          staleRevertRejected: true,
          undoRestoredUnsharedEdits: true,
          draftPreserved: true,
          noPublicationsFromSliceTools: true,
          fixtureSharedSettingsPublication: true,
          revertAfterSharedOnlyUpdate: true,
          collisionAcrossSlices: true,
          uniqueLocalProposals: true,
          offlineIdGenerationAndCopy: true,
          offlineHistoryPagination: true,
          offlineNameDraftReopen: true,
          offlineConflictReviewAndDraft: true,
          offlineTheirsAndUndo: true,
          offlineShareRejected: true,
          noIdReservationChanges: true,
          pendingUpdatesDoNotBlockIds: true,
          helperDoesNotWriteGameFiles: true,
          helperDoesNotChangeSavedStatus: true,
          staleCopyRejected: true,
          uiVerified: true,
          limits: [
            "Synthetic records; no game input or live provider used",
            "Editor relocation is confirmed by the creator; this test does not automate it",
          ],
        },
        null,
        2,
      ),
    );
    console.log(
      "Native slice tools passed: Share/conflict asset reviews, Move asset between slices without file or saved-change mutations, a missing-file recovery and Retry with the coordinator online, Revert/Undo/draft preservation, stale-file rejection, and GUID validation.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});

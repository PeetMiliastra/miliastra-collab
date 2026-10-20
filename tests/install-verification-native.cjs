// Focused install/verification/Undo proof, confined to a disposable practice profile.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { sha, slice } = require("./reliability/fixtures.cjs");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(path.basename(root).startsWith("install-verification-native-"));
  assert.equal(path.basename(path.dirname(root)), ".tools");
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19253");
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
    const watchNotices = () =>
      page.evaluate(async () => {
        window.__notificationProof = [];
        await window.__TAURI_INTERNALS__.invoke("plugin:event|listen", {
          event: "notice",
          target: { kind: "Any" },
          handler: window.__TAURI_INTERNALS__.transformCallback((event) => {
            window.__notificationProof.push(event.payload);
          }),
        });
      });
    const takeNotices = () =>
      page.evaluate(() => window.__notificationProof.splice(0));
    const noOperationPopups = async () => {
      const notices = await takeNotices();
      // A completed background download is still an approved event during these operations.
      assert(
        notices.every((text) =>
          /^.+ shared update: \d+ slice updates downloaded\.$/.test(text),
        ),
        `Unexpected operation popup: ${JSON.stringify(notices)}`,
      );
    };
    const wait = async (predicate) => {
      for (let i = 0; i < 1200; i++) {
        const state = await call("snapshot");
        if (predicate(state)) return state;
        await new Promise((resolve) => setTimeout(resolve, 25));
      }
      throw Error("Installation verification checkpoint timed out");
    };
    assert.equal(
      path.resolve((await call("snapshot")).testDataDirectory),
      root,
    );
    await call("begin_practice");
    let state = await wait(
      (s) =>
        s.practice &&
        !s.metadataPending &&
        s.sections.every((x) => x.installed || x.downloaded),
    );
    await call("save_settings", {
      settings: { ...state.settings, autoApply: false },
    });
    const prefix = String.fromCharCode(92, 92, 63, 92);
    const stage = state.stage.path.startsWith(prefix)
      ? state.stage.path.slice(4)
      : state.stage.path;
    assert(path.resolve(stage).startsWith(root + path.sep));
    const file = (id) =>
      path.join(
        stage,
        "beyond_level_slice",
        id === "shared" ? "42.gilh" : `42_${Number(id) + 1}.gis`,
      );
    const settingsFile = path.join(
      stage,
      "beyond_level_setting",
      "local_setting.gils",
    );
    const settingsBefore = sha(fs.readFileSync(settingsFile));
    const unrelatedBefore = sha(fs.readFileSync(file("60")));
    // Restore the synthetic Quests baseline; its seeded edit is a deliberate conflict.
    fs.writeFileSync(file("8"), slice(8, "Sample Quests baseline"));
    await call("refresh_saved_changes");
    const draft = {
      summary: "Keep my private changelog",
      details: "Automatic verification must not discard it",
    };
    await call("save_draft", { projectId: state.project.id, draft });
    await watchNotices();
    const reserved = state.sections.find((section) => section.id === "60").name;
    await call("start_work", { section: "60", takeover: false });
    await page.waitForFunction(
      (expected) => window.__notificationProof.includes(expected),
      `Reserved ${reserved}`,
    );
    const reservationNotices = await takeNotices();
    assert.equal(
      reservationNotices.filter((text) => text === `Reserved ${reserved}`)
        .length,
      1,
    );
    assert(
      reservationNotices.every(
        (text) =>
          text === `Reserved ${reserved}` ||
          /^.+ shared update: \d+ slice updates downloaded\.$/.test(text),
      ),
    );
    await call("start_work", { section: "60", takeover: false });
    await noOperationPopups();
    await call("finish_work", { section: "60" });
    await noOperationPopups();
    let plan = await call("preview_install");
    assert(plan.revisions.length > 0);
    const originals = new Map(
      plan.revisions.map((r) => [r.section, fs.readFileSync(file(r.section))]),
    );
    await assert.rejects(
      call("install_updates", { planId: plan.id, editorReady: false }),
      /Save and unload/i,
    );
    for (const [id, bytes] of originals)
      assert.deepEqual(fs.readFileSync(file(id)), bytes);
    await page
      .getByRole("button", { name: "Review and apply", exact: true })
      .click();
    const completion = page.getByRole("dialog", {
      name: "Apply updates",
      exact: true,
    });
    await completion
      .getByRole("button", { name: "Continue", exact: true })
      .click();
    await completion.getByRole("checkbox").check();
    await completion
      .getByRole("button", { name: "Back up and apply", exact: true })
      .click();
    await completion
      .getByRole("heading", { name: "Updates applied", exact: true })
      .waitFor();
    state = await call("snapshot");
    assert.equal(
      state.installation.phase,
      "verified",
      "Install must verify before returning",
    );
    assert.equal(state.installation.error, null);
    assert(fs.existsSync(state.installation.journal));
    for (const revision of plan.revisions) {
      assert.equal(sha(fs.readFileSync(file(revision.section))), revision.hash);
      const row = state.sections.find((s) => s.id === revision.section);
      assert(row.installed && !row.installationPending && !row.changed);
    }
    await noOperationPopups();
    assert(
      !/Reopen/.test(
        await completion.getByLabel("Progress", { exact: true }).innerText(),
      ),
    );
    assert(
      await completion
        .getByText("Open the stage in Miliastra to load them.", { exact: true })
        .isVisible(),
    );
    assert.equal(
      await completion.getByRole("button", { name: /Undo/ }).count(),
      0,
    );
    assert.equal(
      await completion
        .getByText("Installation details", { exact: true })
        .count(),
      0,
    );
    assert.equal(await completion.locator("code").count(), 0);
    assert.equal(
      await completion
        .getByRole("button", { name: "Check files and finish", exact: true })
        .count(),
      0,
    );
    await page.screenshot({
      path: path.join(root, "installation-complete.png"),
    });
    await completion.getByRole("button", { name: "Done", exact: true }).click();
    await completion.waitFor({ state: "hidden" });
    assert.equal(await page.locator(".task-resume").count(), 0);
    await page.reload();
    await page.locator(".section-row").first().waitFor();
    await watchNotices();
    assert.equal(await page.locator(".task-resume").count(), 0);
    assert.equal(state.draft.summary, draft.summary);
    await assert.rejects(call("verify_install"), /no pending update/i);
    assert.equal((await call("snapshot")).installation.error, null);
    const target = plan.revisions.find((r) => r.section !== "shared");
    const installedBytes = fs.readFileSync(file(target.section));
    fs.writeFileSync(
      file(target.section),
      slice(Number(target.section), "New private edits"),
    );
    await assert.rejects(
      call("undo_install", { editorReady: true }),
      /new local edits/i,
    );
    assert.equal((await call("snapshot")).installation.phase, "verified");
    fs.writeFileSync(file(target.section), installedBytes);
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page
      .getByRole("button", { name: "Undo last update", exact: true })
      .click();
    const undo = page.getByRole("dialog", {
      name: "Undo last update",
      exact: true,
    });
    assert(
      await undo
        .getByRole("button", { name: "Restore previous slices", exact: true })
        .isDisabled(),
    );
    await undo.getByRole("checkbox").check();
    await undo
      .getByRole("button", { name: "Restore previous slices", exact: true })
      .click();
    await undo
      .getByRole("heading", { name: "Undo complete", exact: true })
      .waitFor();
    await undo.getByRole("button", { name: "Done", exact: true }).click();
    await page
      .getByRole("navigation", { name: "Main navigation" })
      .getByRole("button", { name: /^Slices/ })
      .click();
    assert.equal(
      (await call("snapshot")).installation.phase,
      "undone",
      "Undo must verify before returning",
    );
    for (const [id, bytes] of originals)
      assert.deepEqual(fs.readFileSync(file(id)), bytes);
    // A write after copying must prevent completion and preserve the backup.
    plan = await call("preview_install");
    const operation = call("install_updates", {
      planId: plan.id,
      editorReady: true,
    }).then(
      () => ({ ok: true }),
      (error) => ({ ok: false, error: String(error) }),
    );
    state = await wait((s) => s.installation?.phase === "awaitingReload");
    const tampered = plan.revisions.find((r) => r.section !== "shared");
    fs.writeFileSync(
      file(tampered.section),
      slice(Number(tampered.section), "Changed while checking"),
    );
    const outcome = await operation;
    assert.equal(outcome.ok, false);
    assert.match(outcome.error, /could not be verified/);
    state = await call("snapshot");
    assert.equal(state.installation.phase, "awaitingReload");
    assert(state.installation.error);
    assert(fs.existsSync(state.installation.journal));
    fs.writeFileSync(
      file(tampered.section),
      fs.readFileSync(path.join(root, "objects", tampered.hash)),
    );
    await call("verify_install");
    assert.equal(
      (await call("snapshot")).installation.phase,
      "verified",
      "Recovery only checks files; no reload assertion",
    );
    // Local Revert and its Undo also complete in the same command.
    const privateBoat = fs.readFileSync(file("5"));
    const sequence = (await call("snapshot")).project.sequence;
    const preview = await call("preview_revert", { section: "5" });
    await call("revert_slice", { previewId: preview.id, editorReady: true });
    state = await call("snapshot");
    assert(state.installation.localRevert);
    assert.equal(state.installation.phase, "verified");
    assert.equal(state.sections.find((s) => s.id === "5").changed, false);
    await call("undo_install", { editorReady: true });
    state = await call("snapshot");
    assert.equal(state.installation.phase, "undone");
    assert.deepEqual(fs.readFileSync(file("5")), privateBoat);
    assert(state.sections.find((s) => s.id === "5").changed);
    assert.equal(state.project.sequence, sequence);
    assert.equal(state.draft.summary, draft.summary);
    await noOperationPopups();
    // Using a teammate's version takes the same automatic verification path.
    await call("practice_conflict");
    await wait(
      (s) =>
        s.sections.find((x) => x.id === "8")?.conflict &&
        s.sections.find((x) => x.id === "8")?.downloaded,
    );
    const mine = fs.readFileSync(file("8"));
    const conflict = await call("preview_conflict", { section: "8" });
    await call("resolve_conflict", {
      previewId: conflict.id,
      choice: "theirs",
      draft: conflict.draft,
      editorReady: true,
    });
    assert.equal((await call("snapshot")).installation.phase, "verified");
    assert.equal(sha(fs.readFileSync(file("8"))), conflict.theirs.hash);
    await call("undo_install", { editorReady: true });
    assert.equal((await call("snapshot")).installation.phase, "undone");
    assert.deepEqual(fs.readFileSync(file("8")), mine);
    await noOperationPopups();
    assert.equal(sha(fs.readFileSync(settingsFile)), settingsBefore);
    assert.equal(sha(fs.readFileSync(file("60"))), unrelatedBefore);
    fs.writeFileSync(
      path.join(root, "verification-report.json"),
      JSON.stringify(
        {
          manualOperationsSilent: true,
          approvedReservationCopy: true,
          repeatedReservationSilent: true,
          automaticInstall: true,
          completionUiWithoutReopen: true,
          automaticUndo: true,
          automaticRevert: true,
          automaticConflictInstall: true,
          changedFilesRejectCompletion: true,
          fileCheckRecovery: true,
          dirtyUndoRejected: true,
          draftsPreserved: true,
          unrelatedFilesPreserved: true,
          limits: [
            "Synthetic local files and coordinator; no live editor or cloud account",
          ],
        },
        null,
        2,
      ),
    );
    console.log(
      "Focused native install verification passed: automatic install/Undo/Revert/conflict install, changed-file failure, file-only recovery, dirty Undo refusal, preserved drafts and unrelated files.",
    );
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});

// Practice project: conflicts, Undo and on-screen notifications.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { execFileSync } = require("node:child_process");
const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
assert(root.includes("practice-smoke-"), "Refusing a non-isolated profile");
const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");
(async () => {
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19224");
  const page = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => !p.url().includes("overlay=1"));
  assert(page);
  const invoke = (cmd, args = {}) =>
    page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  const wait = async (f) => {
    for (let n = 0; n < 240; n++) {
      const s = await invoke("snapshot");
      if (f(s)) return s;
      await new Promise((r) => setTimeout(r, 250));
    }
    throw Error("Timed out waiting for the changed workflow");
  };
  let state = await invoke("snapshot");
  assert.equal(path.resolve(state.testDataDirectory), root);
  assert.equal(state.project, null);
  const originalSettings = state.settings;
  if (process.env.MCOLLAB_ONLY_NOTIFICATION !== "1") {
    // A leftover browser-preview flag must not affect the desktop app.
    await page.evaluate(() =>
      localStorage.setItem("miliastra-collab-preview-active", "yes"),
    );
    await invoke("begin_practice");
    state = await wait(
      (s) =>
        s.practice &&
        s.project.sequence === 80 &&
        s.sections.find((x) => x.id === "8")?.conflict,
    );
    const project = state.project.id;
    const stage = state.stage.path.replace(/^\\\\\?\\/, "");
    const target = path.join(stage, "beyond_level_slice", "42_9.gis");
    const shared = path.join(stage, "beyond_level_slice", "42.gilh");
    assert(
      path.resolve(stage).startsWith(path.join(root, "practice") + path.sep),
    );
    assert(
      fs.existsSync(
        path.join(
          path.dirname(path.dirname(path.dirname(path.dirname(stage)))),
          "exchange",
          "project.json",
        ),
      ),
    );
    assert(
      state.sections.find((x) => x.id === "5").changed &&
        !state.sections.find((x) => x.id === "5").conflict,
    );
    assert.equal(
      (await invoke("history_page", { filter: {} })).entries.length,
      50,
    );
    const localBytes = fs.readFileSync(target);
    const review = await invoke("preview_conflict", { section: "8" });
    assert.equal(
      sha(fs.readFileSync(path.join(review.folder, "mine.gis"))),
      review.mine.hash,
    );
    assert.equal(
      sha(fs.readFileSync(path.join(review.folder, "teammate.gis"))),
      review.theirs.hash,
    );
    const changed = Buffer.from(localBytes);
    changed[changed.length - 5] ^= 1;
    fs.writeFileSync(target, changed);
    await assert.rejects(
      invoke("resolve_conflict", {
        previewId: review.id,
        choice: "mine",
        draft: { summary: "Review guards", details: "" },
        editorReady: true,
      }),
      /local section changed/i,
    );
    fs.writeFileSync(target, localBytes);
    const sharedBytes = fs.readFileSync(shared);
    const dirtyShared = Buffer.from(sharedBytes);
    dirtyShared[dirtyShared.length - 5] ^= 1;
    fs.writeFileSync(shared, dirtyShared);
    await assert.rejects(
      invoke("resolve_conflict", {
        previewId: review.id,
        choice: "mine",
        draft: { summary: "Dependency guard", details: "" },
        editorReady: true,
      }),
      /Shared settings changed/i,
    );
    fs.writeFileSync(shared, sharedBytes);
    await invoke("save_conflict_draft", {
      previewId: review.id,
      draft: {
        summary: "Kept the reviewed quest route",
        details: "Both copies preserved",
      },
    });
    await invoke("leave_project");
    await invoke("reopen_project", { projectId: project });
    const reopened = await invoke("preview_conflict", { section: "8" });
    assert.equal(reopened.draft.summary, "Kept the reviewed quest route");
    const row = page
      .locator(".section-row")
      .filter({ has: page.getByText("Quests", { exact: true }) });
    await row.getByRole("button", { name: "Review conflict" }).click();
    let dialog = page.getByRole("dialog");
    await dialog.waitFor();
    await dialog.getByRole("radio", { name: /Keep my version/ }).check();
    assert.equal(
      await dialog
        .getByRole("textbox", { name: "Conflict changelog" })
        .inputValue(),
      "Kept the reviewed quest route",
    );
    await dialog.getByRole("checkbox").check();
    fs.mkdirSync("build/ui-proof", { recursive: true });
    await page.screenshot({ path: "build/ui-proof/conflict-review.png" });
    await dialog.getByRole("button", { name: "Share my version" }).click();
    await page
      .getByRole("dialog", { name: "Resolve Quests", exact: true })
      .waitFor({ state: "hidden" });
    await page
      .getByRole("dialog", { name: "Apply updates", exact: true })
      .getByRole("button", { name: "Continue later", exact: true })
      .click();
    state = await wait((s) => !s.sections.find((x) => x.id === "8").conflict);
    assert.equal(sha(fs.readFileSync(target)), sha(localBytes));
    const history = await invoke("history_page", { filter: { section: "8" } });
    assert.equal(history.entries[0].summary, "Kept the reviewed quest route");
    assert.equal(history.entries[0].revisions[0].parent, review.theirs.hash);
    await invoke("practice_conflict");
    state = await wait((s) => s.sections.find((x) => x.id === "8").conflict);
    const mineBeforeInstall = fs.readFileSync(target);
    await row.getByRole("button", { name: "Review conflict" }).click();
    dialog = page.getByRole("dialog");
    await dialog.waitFor();
    await dialog.getByRole("radio", { name: /Use teammate’s version/ }).check();
    assert(
      await dialog
        .getByRole("button", { name: "Back up and use teammate’s version" })
        .isDisabled(),
    );
    await dialog.getByRole("checkbox").check();
    await dialog
      .getByRole("button", { name: "Back up and use teammate’s version" })
      .click();
    await page
      .getByRole("dialog", { name: "Resolve Quests", exact: true })
      .waitFor({ state: "hidden" });
    state = await wait((s) => s.installation?.phase === "verified");
    assert.equal(sha(fs.readFileSync(target)), state.project.heads["8"].hash);
    assert(state.installation.undoBaselines);
    assert(fs.existsSync(state.installation.journal));
    await wait((s) => s.installation.phase === "verified");
    const installedDialog = page.getByRole("dialog", {
      name: "Apply updates",
      exact: true,
    });
    if (await installedDialog.isVisible())
      await installedDialog
        .getByRole("button", { name: "Done", exact: true })
        .click();
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.getByRole("button", { name: "Undo last update" }).click();
    dialog = page.getByRole("dialog", {
      name: "Undo last update",
      exact: true,
    });
    await dialog.waitFor();
    await dialog.getByRole("checkbox").check();
    await dialog
      .getByRole("button", { name: "Restore previous slices" })
      .click();
    state = await wait((s) => s.installation.phase === "undone");
    assert.equal(sha(fs.readFileSync(target)), sha(mineBeforeInstall));
    assert(
      state.sections.find((x) => x.id === "8").conflict,
      "Undo must restore the original conflict baseline, not mark private edits installed",
    );
    await page
      .getByRole("dialog", { name: "Undo last update", exact: true })
      .getByRole("button", { name: "Done", exact: true })
      .click();
    await page
      .getByRole("navigation", { name: "Main navigation" })
      .getByRole("button", { name: /^Slices/ })
      .click();
    await invoke("leave_practice");
    assert.equal((await invoke("snapshot")).project, null);
    await invoke("begin_practice");
    state = await invoke("snapshot");
    assert.equal(state.project.id, project);
    assert.equal(sha(fs.readFileSync(target)), sha(mineBeforeInstall));
    assert.equal(
      state.settings.endpoint,
      originalSettings.endpoint,
      "Practice changed the normal service setting",
    );
  }
  await page.getByRole("button", { name: "App settings", exact: true }).click();
  state = await invoke("snapshot");
  assert(state.testOverlayHwnd, "Native test window handle unavailable");
  const overlay = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => p.url().includes("overlay=1"));
  assert(overlay);
  const proofAssembly = path.join(root, "notification-proof.dll");
  execFileSync(
    "powershell.exe",
    [
      "-NoProfile",
      "-Command",
      `Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class Proof{[StructLayout(LayoutKind.Sequential)]public struct R{public int l,t,r,b;}[StructLayout(LayoutKind.Sequential)]public struct M{public int size;public R monitor,work;public uint flags;}[DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern IntPtr FindWindow(string c,string n);[DllImport("user32.dll")]public static extern int GetWindowLong(IntPtr h,int n);[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out R r);[DllImport("user32.dll")]public static extern IntPtr MonitorFromWindow(IntPtr h,uint f);[DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern bool GetMonitorInfo(IntPtr h,ref M m);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);[DllImport("user32.dll")]public static extern uint GetDpiForWindow(IntPtr h);}' -OutputAssembly '${proofAssembly.replace(/'/g, "''")}'`,
    ],
    { encoding: "utf8", windowsHide: true },
  );
  await invoke("begin_practice");
  await invoke("start_work", { section: "0", takeover: false });
  await overlay.getByText(/Reserved Common/).waitFor();
  const ps = `Add-Type -Path '${proofAssembly.replace(/'/g, "''")}';$h=[IntPtr]${state.testOverlayHwnd};$r=New-Object Proof+R;[Proof]::GetWindowRect($h,[ref]$r)|Out-Null;$m=New-Object Proof+M;$m.size=40;[Proof]::GetMonitorInfo([Proof]::MonitorFromWindow($h,2),[ref]$m)|Out-Null;@{visible=[Proof]::IsWindowVisible($h);style=[Proof]::GetWindowLong($h,-20);right=$r.r;bottom=$r.b;workRight=$m.work.r;workBottom=$m.work.b;dpi=[Proof]::GetDpiForWindow($h)}|ConvertTo-Json -Compress`;
  const geometry = JSON.parse(
    execFileSync("powershell.exe", ["-NoProfile", "-Command", ps], {
      encoding: "utf8",
      windowsHide: true,
    }),
  );
  assert(geometry.visible, JSON.stringify(geometry));
  assert(geometry.style & 0x20);
  assert(geometry.style & 0x08000000);
  const margin = Math.round((24 * geometry.dpi) / 96);
  assert(
    Math.abs(geometry.workRight - geometry.right - margin) <= 2,
    JSON.stringify(geometry),
  );
  assert(
    Math.abs(geometry.workBottom - geometry.bottom - margin) <= 2,
    JSON.stringify(geometry),
  );
  for (const selector of ["html", "body", "#root"])
    assert.equal(
      await overlay
        .locator(selector)
        .evaluate((el) => getComputedStyle(el).backgroundColor),
      "rgba(0, 0, 0, 0)",
    );
  await overlay.waitForFunction(() => {
    const img = document.querySelector(".game-notice .logo-mark img");
    return img?.complete && img.naturalWidth === 512;
  });
  assert.equal(
    await overlay
      .locator(".game-notice .logo-mark")
      .evaluate((el) => getComputedStyle(el).backgroundColor),
    "rgba(0, 0, 0, 0)",
  );
  await overlay.screenshot({
    path: "build/ui-proof/notification-transparent.png",
    omitBackground: true,
  });
  if (state.practice) await invoke("leave_practice");
  await browser.close();
  console.log(
    process.env.MCOLLAB_ONLY_NOTIFICATION === "1"
      ? "Focused native notification proof passed: transparency, bottom-right work-area positioning, click-through and no-activate flags."
      : "Focused native practice proof passed: real local files/coordinator/history, persisted workspace and drafts, conflict preservation, changed-file/dependency rejection, keep-mine publication, use-teammate installation, verified Undo restoring conflict, and native notification transparency/bottom-right/click-through/no-activate flags.",
  );
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

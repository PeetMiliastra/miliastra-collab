const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR);
  assert(root.includes("practice-smoke-"));
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19224");
  const page = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => !p.url().includes("overlay=1"));
  const invoke = (cmd, args = {}) =>
    page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  let state = await invoke("snapshot");
  assert.equal(path.resolve(state.testDataDirectory), root);
  const hash = (stage) =>
    crypto
      .createHash("sha256")
      .update(
        fs.readFileSync(path.join(stage, "beyond_level_slice", "42_9.gis")),
      )
      .digest("hex");
  const checkpoint = path.join(root, "restart-checkpoint.json");
  if (process.env.MCOLLAB_RESTART_PHASE === "prepare") {
    await invoke("begin_practice");
    state = await invoke("snapshot");
    assert(state.practice && state.online);
    if (state.sections.find((x) => x.id === "8")?.conflict) {
      const row = page
        .locator(".section-row")
        .filter({ has: page.getByText("Quests", { exact: true }) });
      await row.getByRole("button", { name: "Review conflict" }).click();
      const dialog = page.getByRole("dialog");
      await dialog.getByRole("radio", { name: /Keep my version/ }).check();
      assert(
        await dialog
          .getByRole("button", { name: "Share my version" })
          .evaluate(
            (el) => el.getBoundingClientRect().bottom <= window.innerHeight,
          ),
        "Primary conflict action is outside the viewport",
      );
      await page.screenshot({ path: "build/ui-proof/conflict-review.png" });
      await dialog.getByRole("button", { name: "Keep both for now" }).click();
    }
    fs.writeFileSync(
      checkpoint,
      JSON.stringify({
        id: state.project.id,
        hash: hash(state.stage.path),
        sequence: state.project.sequence,
      }),
    );
  } else {
    for (let i = 0; i < 40 && (!state.practice || !state.online); i++) {
      await new Promise((r) => setTimeout(r, 250));
      state = await invoke("snapshot");
    }
    const expected = JSON.parse(fs.readFileSync(checkpoint));
    assert(
      state.practice && state.online,
      "The saved practice project did not reconnect after the restart",
    );
    assert.equal(state.project.id, expected.id);
    assert.equal(state.project.sequence, expected.sequence);
    assert.equal(hash(state.stage.path), expected.hash);
    await invoke("leave_practice");
    console.log(
      "Focused restart proof passed: saved practice project, History and private edits survived a full desktop process restart.",
    );
  }
  await browser.close();
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

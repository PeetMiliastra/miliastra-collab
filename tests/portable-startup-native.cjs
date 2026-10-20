const { chromium } = require("playwright"),
  assert = require("node:assert/strict"),
  path = require("node:path");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(path.basename(root).startsWith("portable-startup-"));
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19245");
  try {
    const page = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => !p.url().includes("overlay=1"));
    await page
      .getByRole("link", { name: "miliastra/collab", exact: true })
      .waitFor();
    const invoke = (cmd, args = {}) =>
      page.evaluate(
        ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
        { cmd, args },
      );
    const state = await invoke("snapshot");
    assert.equal(path.resolve(state.testDataDirectory), root);
    assert.equal(state.settings.autoApply, false);
    assert.equal(state.project, null);
    assert(
      state.message.includes(
        "Updated to version " + require("../package.json").version,
      ),
    );
    await page
      .getByRole("button", { name: "App settings", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Check for an app update", exact: true })
      .click();
    await page
      .getByRole("alert")
      .filter({ hasText: "not configured" })
      .waitFor();
    assert.equal(
      await page.getByText("You are up to date.", { exact: true }).count(),
      0,
    );
    console.log(
      "Native portable startup passed: Tauri startup acknowledgement, preserved device settings and draft database, update controls, honest unconfigured-feed error.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

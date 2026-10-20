// Navigating between projects, keeping drafts, and leaving the practice project.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(root.includes("navigation-smoke-"), "Refusing a non-isolated profile");
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19225");
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
  assert.equal(state.project, null);
  await invoke("begin_practice");
  await page
    .getByRole("heading", { name: "Fisch — practice", exact: true })
    .waitFor({ timeout: 60000 });
  await page.locator(".progress").waitFor({ state: "hidden" });
  state = await invoke("snapshot");
  assert.equal(
    await page
      .locator(".eyebrow, .summary-grid, .welcome-art, .footer, .privacy-note")
      .count(),
    0,
  );
  assert.equal(
    await page
      .getByText("Your game saves are not affected.", { exact: true })
      .count(),
    0,
    "The practice banner should be gone",
  );
  assert(
    await page
      .locator(".section-list")
      .getByRole("button", { name: "Boats", exact: true })
      .isVisible(),
  );
  const practice = state.project.id;
  const endpoint = new URL(await invoke("invite")).origin;
  assert(endpoint.startsWith("http://127.0.0.1:"));
  const target = path.join(state.stage.path, "beyond_level_slice", "42_6.gis");
  const sha = () =>
    crypto.createHash("sha256").update(fs.readFileSync(target)).digest("hex");
  const before = sha();
  await page.getByRole("button", { name: /Share update/ }).click();
  await page
    .getByPlaceholder("Fixed passengers falling through the boat deck")
    .fill("Navigation draft survives");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("button", { name: "All projects", exact: true }).click();
  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  assert.equal((await invoke("snapshot")).project, null);
  assert.equal(
    await page.locator(".welcome-card, .feature-grid, .eyebrow").count(),
    0,
  );
  assert((await invoke("connections")).some((p) => p.id === practice));
  assert.equal(sha(), before);
  await page
    .getByRole("button", { name: "Fisch — practice", exact: true })
    .click();
  await page
    .getByRole("heading", { name: "Fisch — practice", exact: true })
    .waitFor();
  assert.equal(
    (await invoke("snapshot")).draft.summary,
    "Navigation draft survives",
  );
  await page.getByRole("button", { name: "All projects", exact: true }).click();
  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  // A second project to return to after leaving practice.
  const secondStage = path.join(
    root,
    "second-stage",
    "Practice",
    "Beyond_Local_Save_Level",
    "42",
  );
  const secondExchange = path.join(root, "second-exchange");
  assert(
    secondStage.startsWith(root + path.sep) &&
      secondExchange.startsWith(root + path.sep),
  );
  fs.cpSync(state.stage.path, secondStage, {
    recursive: true,
    errorOnExist: true,
  });
  fs.mkdirSync(secondExchange);
  await invoke("save_settings", { settings: { ...state.settings, endpoint } });
  await invoke("create_project", {
    name: "Second local workspace",
    displayName: "You",
    stagePath: secondStage,
    kind: "folder",
    folder: secondExchange,
  });
  const second = (await invoke("snapshot")).project.id;
  await page
    .getByRole("heading", { name: "Second local workspace", exact: true })
    .waitFor();
  await invoke("begin_practice");
  await page
    .getByRole("heading", { name: "Fisch — practice", exact: true })
    .waitFor();
  // "All projects" should go to the list, not back into the second project.
  await page.getByRole("button", { name: "All projects", exact: true }).click();
  try {
    await page
      .getByRole("heading", { name: "Your projects", exact: true })
      .waitFor();
  } catch (error) {
    console.error(await page.locator("body").innerText());
    console.error(await invoke("snapshot"));
    throw error;
  }
  assert.equal((await invoke("snapshot")).project, null);
  assert.equal((await invoke("connections")).length, 2);
  await invoke("begin_practice");
  await page
    .getByRole("heading", { name: "Fisch — practice", exact: true })
    .waitFor();
  await invoke("leave_practice");
  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  assert.equal(
    (await invoke("snapshot")).project,
    null,
    "Practice retained a stale return target",
  );
  await page
    .getByRole("button", { name: "Second local workspace", exact: true })
    .click();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("heading", { name: "History", exact: true }).waitFor();
  await page.getByRole("combobox", { name: "Slice" }).waitFor();
  assert.equal(
    await page.getByRole("textbox", { name: "Search history" }).count(),
    0,
  );
  await page.screenshot({ path: "build/ui-proof/plain-history.png" });
  await page.getByRole("button", { name: "App settings", exact: true }).click();
  await page
    .getByRole("heading", { name: "App settings", exact: true })
    .waitFor();
  assert.equal(
    await page
      .getByRole("button", { name: "Test notification", exact: true })
      .count(),
    0,
  );
  assert.equal(await page.locator("details.settings-card[open]").count(), 0);
  await page.screenshot({ path: "build/ui-proof/plain-settings.png" });
  await page.getByRole("button", { name: "All projects", exact: true }).click();
  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  assert.equal((await invoke("snapshot")).project, null);
  assert((await invoke("connections")).some((p) => p.id === second));
  assert.equal(sha(), before, "Navigation changed saved slice bytes");
  fs.mkdirSync("build/ui-proof", { recursive: true });
  await page.screenshot({ path: "build/ui-proof/projects-overview.png" });
  await page
    .getByRole("button", { name: "Fisch — practice", exact: true })
    .click();
  await page
    .getByRole("heading", { name: "Fisch — practice", exact: true })
    .waitFor();
  await page.screenshot({ path: "build/ui-proof/workspace-navigation.png" });
  await invoke("leave_project");
  await browser.close();
  console.log(
    "Focused navigation proof passed: overview/reopen, latest draft, saved connections, unchanged slices, plain Slices/History/Settings screens, essential actions and stale practice-return clearing.",
  );
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

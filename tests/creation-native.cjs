const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const net = require("node:net");
const { execFileSync } = require("node:child_process");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(root.includes("creation-smoke-"));
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19227");
  const page = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => !p.url().includes("overlay=1"));
  const invoke = (cmd, args = {}) =>
    page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  await page
    .getByRole("link", { name: "miliastra/collab", exact: true })
    .waitFor();
  let state = await invoke("snapshot");
  assert.equal(path.resolve(state.testDataDirectory), root);
  const checkpoint = path.join(root, "creation-checkpoint.json");
  if (process.env.MCOLLAB_CREATION_PHASE === "resume") {
    const expected = JSON.parse(fs.readFileSync(checkpoint));
    for (let i = 0; i < 80 && !state.online; i++) {
      await new Promise((r) => setTimeout(r, 250));
      state = await invoke("snapshot");
    }
    assert.equal(
      state.settings.endpoint,
      expected.endpoint,
      "Editable config did not override the old SQLite service URL",
    );
    assert.equal(state.project.id, expected.id);
    assert(state.online, "The project did not reconnect after the restart");
    assert(!state.practice, "Ordinary local project became practice");
    await invoke("refresh");
    assert.equal(
      (await invoke("history_page", { filter: {} })).entries.length,
      1,
    );
    await invoke("leave_project");
    console.log(
      "Creation restart passed: ordinary project, coordinator connection and baseline survived.",
    );
    await browser.close();
    return;
  }
  assert.equal(state.project, null);
  const configFile = path.join(root, "coordination.json");
  assert.equal(
    JSON.parse(fs.readFileSync(configFile)).serviceUrl,
    "http://127.0.0.1:19731",
  );
  assert.equal(state.settings.endpoint, "http://127.0.0.1:19731");
  const endpoint = await invoke("test_service");
  assert.equal(new URL(endpoint).hostname, "127.0.0.1");
  await invoke("save_settings", { settings: { ...state.settings, endpoint } });
  await invoke("begin_practice");
  state = await invoke("snapshot");
  const source = state.stage.path;
  const sourceFile = path.join(source, "42.gil");
  const hash = () =>
    crypto
      .createHash("sha256")
      .update(fs.readFileSync(sourceFile))
      .digest("hex");
  const before = hash();
  const stage = path.join(
    root,
    "saves",
    "Creator",
    "Beyond_Local_Save_Level",
    "42",
  );
  const exchange = path.join(root, "exchange");
  assert(stage.startsWith(root + path.sep));
  fs.cpSync(source, stage, { recursive: true, errorOnExist: true });
  fs.mkdirSync(exchange);
  await invoke("leave_project");
  const closedPort = await new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const port = s.address().port;
      s.close(() => resolve(port));
    });
  });
  await invoke("save_settings", {
    settings: {
      ...state.settings,
      saveRoot: path.join(root, "saves"),
      endpoint: `http://127.0.0.1:${closedPort}`,
    },
  });

  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  await page
    .getByRole("button", { name: "Create project", exact: true })
    .click();
  let dialog = page.getByRole("dialog");
  await dialog.waitFor();
  await dialog.getByPlaceholder("Display name").fill("Robin");
  await dialog.getByPlaceholder("Project name").fill("Creation check");
  await dialog.locator(".detected-stage strong").waitFor();
  assert.equal(
    await dialog
      .getByRole("combobox", { name: "Keep the team folder in" })
      .inputValue(),
    "onedrive",
  );
  await dialog
    .getByRole("combobox", { name: "Keep the team folder in" })
    .selectOption("folder");
  await dialog.getByRole("button", { name: "Choose shared folder" }).click();
  let button = dialog
    .getByRole("button", { name: "Create project", exact: true })
    .last();
  await button.click();
  await dialog.getByRole("status").waitFor();
  assert(
    await dialog
      .getByRole("button", { name: "Close", exact: true })
      .isDisabled(),
  );
  await dialog.getByRole("alert").waitFor();
  assert.match(
    await dialog.getByRole("alert").innerText(),
    /collaboration service is unavailable/i,
  );
  assert.equal(
    fs.readdirSync(exchange).length,
    0,
    "Service preflight must precede every exchange write",
  );
  assert.equal(
    await dialog.getByPlaceholder("Project name").inputValue(),
    "Creation check",
  );
  await invoke("save_settings", { settings: { ...state.settings, endpoint } });
  await dialog
    .getByRole("button", { name: "Create project", exact: true })
    .last()
    .click();
  await dialog.getByRole("status").waitFor();
  fs.mkdirSync("build/ui-proof", { recursive: true });
  await page.screenshot({ path: "build/ui-proof/creation-progress.png" });
  // After the first project, the app offers to start with Windows (once per WebView profile).
  const autostart = page.getByRole("dialog", {
    name: "Keep your team in sync",
    exact: true,
  });
  for (let i = 0; i < 300 && (await dialog.count()); i++) {
    if (await autostart.isVisible())
      await autostart
        .getByRole("button", { name: "Not now", exact: true })
        .click();
    await new Promise((r) => setTimeout(r, 100));
  }
  await dialog.waitFor({ state: "hidden" });
  await page
    .getByRole("heading", { name: "Creation check", exact: true })
    .waitFor();
  state = await invoke("snapshot");
  assert(state.online && !state.practice);
  assert.equal(state.project.sequence, 1);
  assert.equal(Object.keys(state.project.heads).length, 65);
  assert(fs.existsSync(path.join(exchange, "project.json")));
  for (let n = 0; n < 150; n++) {
    if (
      fs.existsSync(path.join(exchange, "Latest")) &&
      fs.readdirSync(path.join(exchange, "Latest")).length === 65
    )
      break;
    await new Promise((r) => setTimeout(r, 100));
  }
  assert.equal(fs.readdirSync(path.join(exchange, "Latest")).length, 65);
  assert.equal(fs.readdirSync(path.join(exchange, "History")).length, 1);
  assert.equal(hash(), before);
  await page.screenshot({ path: "build/ui-proof/creation-complete.png" });
  assert.equal(
    JSON.parse(fs.readFileSync(configFile)).serviceUrl,
    endpoint,
    "Saving the selected service did not update coordination.json",
  );
  execFileSync("python", [
    "-c",
    "import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); v=json.loads(c.execute('SELECT value FROM kv WHERE key=?',('settings',)).fetchone()[0]); v['endpoint']='https://collabs.miliastra.dev'; c.execute('UPDATE kv SET value=? WHERE key=?',(json.dumps(v),'settings')); c.commit(); c.close()",
    path.join(root, "collab.db"),
  ]);
  fs.writeFileSync(
    checkpoint,
    JSON.stringify({ id: state.project.id, endpoint }),
  );
  await browser.close();
  console.log(
    "Creation passed: immediate progress, inline service failure before writes, retained form, test coordinator check, real creation, automatic close, original fixture unchanged.",
  );
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

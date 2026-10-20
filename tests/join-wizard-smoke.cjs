// Join wizard checks in the browser, with faked app responses.
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
(async () => {
  const browser = await chromium.launch({
    headless: true,
    executablePath: "C:/Program Files/Google/Chrome/Application/chrome.exe",
  });
  try {
    const page = await browser.newPage({
      viewport: { width: 1180, height: 810 },
    });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.addInitScript(() => {
      localStorage.setItem("autostart-asked", "1");
      const invitation = {
        version: 1,
        endpoint: "http://127.0.0.1:19000",
        project: "project",
        invite: "fixture",
      };
      const state = {
        cloudProviders: ["onedrive"],
        storageAccounts: { google: "notConnected", onedrive: "connected" },
        settings: {
          endpoint: invitation.endpoint,
          saveRoot: "C:/fixture",
          oauth: {},
          autostart: false,
          autoApply: false,
          screenRecognition: false,
        },
        stage: null,
        project: null,
        member: null,
        sections: [],
        draft: { summary: "", details: "" },
        online: true,
        message: "",
        compatibility: {
          directInstall: false,
          liveDetection: false,
          reason: "",
        },
        game: { running: false, foreground: false },
      };
      window.fixture = {
        access: false,
        keySaved: false,
        imported: false,
        calls: [],
        finishDownload: null,
      };
      window.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (cmd, args) => {
          window.fixture.calls.push({ cmd, args });
          if (cmd.startsWith("plugin:event|")) return 1;
          if (cmd === "snapshot") return structuredClone(state);
          if (cmd === "connections" || cmd === "discover") return [];
          if (cmd === "pending_invitation")
            return state.project ? null : invitation;
          if (cmd === "invitation_details")
            return window.fixture.keySaved
              ? {
                  projectName: "Fisch",
                  storageKind: "onedrive",
                  mappedFolder: null,
                  privacyKeyRequired: false,
                }
              : {
                  projectName: "Encrypted project",
                  storageKind: "locked",
                  mappedFolder: null,
                  privacyKeyRequired: true,
                };
          if (cmd === "check_invitation_storage") {
            if (!window.fixture.access) throw Error("Folder not shared yet");
            return { baseStage: true };
          }
          if (cmd === "invitation_privacy_key_ready")
            return window.fixture.keySaved;
          if (cmd === "save_folder_privacy_key") {
            window.fixture.keySaved = true;
            return;
          }
          if (cmd === "request_folder_access") return "request";
          if (cmd === "folder_access_status")
            return { status: "pending", message: "" };
          if (cmd === "find_project_stage")
            return window.fixture.imported ? "C:/fixture/imported" : null;
          if (cmd === "download_base_stage")
            return new Promise((resolve) => {
              window.fixture.finishDownload = () =>
                resolve("C:/Downloads/Fisch.gil");
            });
          if (cmd === "join_project") {
            state.project = {
              id: "project",
              name: "Fisch",
              storage: { kind: "onedrive", location: "fixture" },
              members: [{ id: "me", name: "Robin", owner: false }],
              locks: [],
              heads: {},
              sequence: 1,
            };
            state.member = "me";
            return;
          }
          return null;
        },
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };
    });
    await page.goto("http://127.0.0.1:1420");
    let dialog = page.getByRole("dialog", {
      name: "Join Encrypted project",
      exact: true,
    });
    await dialog.waitFor();
    await dialog
      .getByRole("heading", { name: "Paste the project’s Encryption Key" })
      .waitFor();
    assert.equal(
      await page.evaluate(
        () =>
          window.fixture.calls.filter((c) => c.cmd === "request_folder_access")
            .length,
      ),
      0,
    );
    fs.mkdirSync("build/ui-proof", { recursive: true });
    await page.screenshot({ path: "build/ui-proof/join-privacy-key.png" });
    await dialog
      .getByLabel("Encryption Key", { exact: true })
      .fill("mcollab-key-v2:fixture");
    await dialog
      .getByRole("button", { name: "Continue with Encryption Key" })
      .click();
    dialog = page.getByRole("dialog", { name: "Join Fisch", exact: true });
    await dialog.waitFor();
    assert.equal(
      await dialog.getByRole("group", { name: "Google Drive account" }).count(),
      0,
    );
    assert.equal(
      await dialog.getByText(/Files go in|Sign in with the account/).count(),
      0,
    );
    await dialog.getByPlaceholder("Display name").fill("Robin");
    await page.screenshot({ path: "build/ui-proof/join-configure.png" });
    await dialog
      .getByRole("button", { name: "Join project", exact: true })
      .click();
    await dialog.getByText(/Waiting for the project owner/).waitFor();
    assert.equal(await dialog.getByRole("textbox").count(), 0);
    await page.screenshot({ path: "build/ui-proof/join-waiting.png" });
    await page.evaluate(() => (window.fixture.access = true));
    await dialog
      .getByText("Downloading the starting stage…", { exact: true })
      .waitFor();
    await page.screenshot({ path: "build/ui-proof/join-downloading.png" });
    await page.evaluate(() => window.fixture.finishDownload());
    await dialog
      .getByRole("heading", { name: "Import the starting stage", exact: true })
      .waitFor();
    assert.equal(
      await dialog
        .getByRole("button", { name: "Choose imported stage", exact: true })
        .isVisible(),
      true,
    );
    await page.screenshot({ path: "build/ui-proof/join-import.png" });
    await page.setViewportSize({ width: 375, height: 812 });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
    assert(
      await dialog
        .getByRole("button", { name: "Cancel", exact: true })
        .isVisible(),
    );
    await page.screenshot({ path: "build/ui-proof/join-import-narrow.png" });
    assert.equal(
      await page.evaluate(
        () =>
          window.fixture.calls.filter((c) => c.cmd === "join_project").length,
      ),
      0,
    );
    await page.evaluate(() => (window.fixture.imported = true));
    await dialog.waitFor({ state: "hidden" });
    await page.getByRole("heading", { name: "Fisch", exact: true }).waitFor();
    assert.equal(
      await page.evaluate(
        () =>
          window.fixture.calls.filter((c) => c.cmd === "download_base_stage")
            .length,
      ),
      1,
    );
    assert.equal(
      await page.evaluate(
        () =>
          window.fixture.calls.filter((c) => c.cmd === "request_folder_access")
            .length,
      ),
      1,
    );
    assert.equal(
      await page.evaluate(
        () =>
          window.fixture.calls.filter((c) => c.cmd === "join_project").length,
      ),
      1,
    );
    assert.deepEqual(errors, []);
    console.log(
      "Guided join browser checks passed: key-first unlock, configure, waiting, automatic download, import prompt, narrow layout and automatic final join; Google disabled. Explicit IPC fixtures only.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

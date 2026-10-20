const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
(async () => {
  fs.mkdirSync("build/ui-proof", { recursive: true });
  const browser = await chromium.launch({
    headless: true,
    executablePath:
      process.env.CHROME_PATH ||
      "C:/Program Files/Google/Chrome/Application/chrome.exe",
  });
  try {
    const context = await browser.newContext({
      viewport: { width: 1280, height: 900 },
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.emulateMedia({ colorScheme: "light" });
    await page.context().addInitScript(() => {
      const names = [
        "Common",
        "Save",
        "UI - Common",
        "UI - Items",
        "Fishing Rods",
        "Boats",
        "Islands",
        "NPCs",
        "Quests",
        "Totems",
        "Shops",
      ];
      const revision = (id) => ({
        section: String(id),
        name: names[id] || "Shared level settings",
        hash: "a".repeat(64),
        contentHash: "b".repeat(64),
        parent: null,
        artifact:
          "History/11111111-1111-4111-8111-111111111111/" + (id + 1) + ".gis",
        size: 100,
      });
      const project = {
        id: "p",
        name: "Fisch",
        storage: { kind: "folder", location: "" },
        members: [
          { id: "me", name: "Robin", owner: true },
          { id: "alex", name: "Alex", owner: false },
        ],
        locks: [],
        heads: Object.fromEntries(names.map((_, i) => [i, revision(i)])),
        sequence: 1,
      };
      const state = {
        settings: {
          endpoint: "http://127.0.0.1:8787",
          saveRoot: "C:/test",
          oauth: { microsoftClientId: "", googleClientId: "" },
          autostart: false,
          screenRecognition: false,
        },
        stage: {
          path: "C:/test",
          level: "1073742034",
          account: "local",
          name: "Fisch",
          sections: names.map((name, id) => ({ id: String(id), name })),
          lastSavedSection: "5",
          settingsHash: "hash",
        },
        project,
        member: "me",
        sections: names.map((name, id) => ({
          id: String(id),
          name,
          changed: id === 5,
          conflict: false,
          lock:
            id === 5
              ? {
                  section: "5",
                  member: "me",
                  name: "Robin",
                  fence: 1,
                  expires: 9999999999,
                }
              : id === 6
                ? {
                    section: "6",
                    member: "alex",
                    name: "Alex",
                    fence: 2,
                    expires: 9999999999,
                  }
                : null,
          downloaded: true,
          installed: id !== 6,
        })),
        draft: { summary: "", details: "" },
        online: true,
        message: "",
        compatibility: {
          directInstall: true,
          liveDetection: false,
          reason:
            "Installs section updates with a backup. Reload the stage afterward.",
        },
        game: { running: false, foreground: false },
      };
      const history = [
        {
          id: "baseline",
          author: "Alex",
          authorId: "alex",
          summary: "Added the first fishing island",
          details:
            "A quiet place to cast a line, with a small dock and a shop.",
          at: 1792053000,
          sequence: 1,
          revisions: [revision(6)],
          restores: null,
        },
      ];
      window.calls = [];
      window.__TAURI_INTERNALS__ = {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: () => 1,
        unregisterCallback: () => {},
        invoke: async (cmd, args) => {
          window.calls.push({ cmd, args });
          if (cmd.startsWith("plugin:event|")) return 1;
          if (cmd === "snapshot") return JSON.parse(JSON.stringify(state));
          if (cmd === "history_page") return { entries: history, next: null };
          if (cmd === "save_draft") {
            state.draft = args.draft;
            return;
          }
          if (cmd === "share_update") {
            if (!args.draft.summary.trim()) throw Error("Write a changelog");
            state.sections[5].changed = false;
            state.project.sequence++;
            state.draft = { summary: "", details: "" };
            return { id: "new" };
          }
          if (cmd === "inspect_slice")
            return {
              section: args.section,
              changes: [],
              otherChanges: false,
              baselineAvailable: true,
              assets: [
                {
                  key: "Entity:1077936129",
                  id: 1077936129,
                  name: "Boat deck",
                  kind: "Entity",
                  section: Number(args.section),
                  canGenerateId: true,
                },
                {
                  key: "Prefab:1077936130",
                  id: 1077936130,
                  name: "Fishing boat",
                  kind: "Prefab",
                  section: Number(args.section),
                  canGenerateId: true,
                },
                {
                  key: "Configuration:1098907649",
                  id: 1098907649,
                  name: "Dive",
                  kind: "Character Skill",
                  section: Number(args.section),
                  canGenerateId: true,
                },
              ],
            };
          if (cmd === "discover") return [state.stage];
          if (cmd === "pending_invitation") return null;
          return null;
        },
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };
    });
    const base = process.env.MCOLLAB_UI_URL || "http://127.0.0.1:1420";
    await page.goto(base);
    await page.getByRole("heading", { name: "Fisch", exact: true }).waitFor();
    await page.screenshot({
      path: "build/ui-proof/project.png",
      fullPage: true,
    });
    await page.getByRole("button", { name: /^Share update/ }).click();
    const submit = page.getByRole("button", {
      name: "Share update",
      exact: true,
    });
    assert.equal(await submit.isDisabled(), true);
    await page
      .getByPlaceholder("Fixed passengers falling through the boat deck")
      .fill("Fixed passengers falling through the boat deck");
    assert.equal(await submit.isEnabled(), true);
    await page.screenshot({ path: "build/ui-proof/share.png" });
    await submit.click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    assert(
      await page.evaluate(() =>
        window.calls.some(
          (c) =>
            c.cmd === "share_update" &&
            c.args.draft.summary ===
              "Fixed passengers falling through the boat deck",
        ),
      ),
    );
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.getByText("Added the first fishing island").waitFor();
    await page.screenshot({
      path: "build/ui-proof/history.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: /Added the first fishing island/ })
      .click();
    assert.equal(
      await page.getByRole("button", { name: "Get update files" }).count(),
      0,
    );
    await page.getByRole("button", { name: "Close", exact: true }).click();
    await page
      .getByRole("button", { name: "App settings", exact: true })
      .click();
    await page.getByRole("heading", { name: "App settings" }).waitFor();
    const mode = page.getByRole("combobox", {
      name: "Color mode",
      exact: true,
    });
    assert.equal(await mode.inputValue(), "system");
    await mode.selectOption("dark");
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "dark",
    );
    await page.waitForFunction(() =>
      window.calls.some(
        ({ cmd, args }) =>
          cmd === "plugin:window|set_theme" && args.value === "dark",
      ),
    );
    await page.screenshot({
      path: "build/ui-proof/appearance-settings-dark.png",
      fullPage: true,
    });
    await page.reload();
    await page.getByRole("heading", { name: "Fisch", exact: true }).waitFor();
    assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
    await page.screenshot({
      path: "build/ui-proof/appearance-project-dark.png",
      fullPage: true,
    });
    await page.getByRole("button", { name: /^Share update/ }).click();
    await page.getByRole("dialog").waitFor();
    await page.screenshot({ path: "build/ui-proof/appearance-share-dark.png" });
    await page.keyboard.press("Escape");
    await page
      .getByRole("button", { name: "Move asset between slices", exact: true })
      .click();
    const helper = page.getByRole("dialog", {
      name: "Move asset between slices",
      exact: true,
    });
    await helper
      .getByRole("button", {
        name: "Boat deck — Entity (1077936129)",
        exact: true,
      })
      .click();
    await page.screenshot({
      path: "build/ui-proof/appearance-helper-dark.png",
    });
    const palette = await page.evaluate(() => {
      const root = getComputedStyle(document.documentElement);
      const modal = getComputedStyle(document.querySelector(".modal"));
      const card = getComputedStyle(
        document.querySelector('.asset-card[aria-pressed="true"]'),
      );
      const input = getComputedStyle(
        document.querySelector(".asset-search input"),
      );
      const primary = getComputedStyle(
        document.querySelector(".modal .button.primary"),
      );
      return {
        canvas: root.backgroundColor,
        text: root.color,
        surface: modal.backgroundColor,
        muted: root.getPropertyValue("--muted").trim(),
        selected: card.backgroundColor,
        input: input.backgroundColor,
        inputText: input.color,
        primary: primary.backgroundColor,
        primaryText: primary.color,
      };
    });
    const luminance = (color) => {
      const values = color.startsWith("#")
        ? color
            .slice(1)
            .match(/../g)
            .map((v) => parseInt(v, 16))
        : color
            .match(/[\d.]+/g)
            .slice(0, 3)
            .map(Number);
      const channels = values.map((value) => {
        const v = value / 255;
        return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
      });
      return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
    };
    const contrast = (a, b) => {
      const x = luminance(a),
        y = luminance(b);
      return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
    };
    for (const [name, a, b] of [
      ["body text", palette.text, palette.canvas],
      ["modal text", palette.text, palette.surface],
      ["secondary text", palette.muted, palette.surface],
      ["input text", palette.inputText, palette.input],
      ["primary button", palette.primaryText, palette.primary],
    ]) {
      assert(contrast(a, b) >= 4.5, name + " contrast is too low");
    }
    assert.notEqual(palette.surface, "rgb(255, 255, 255)");
    assert.notEqual(palette.input, "rgb(255, 255, 255)");
    await page.keyboard.press("Escape");
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.getByText("Added the first fishing island").waitFor();
    await page.screenshot({
      path: "build/ui-proof/appearance-history-dark.png",
      fullPage: true,
    });
    await page
      .getByRole("button", { name: "App settings", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Color mode" })
      .selectOption("light");
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "light",
    );
    await page.waitForFunction(() =>
      window.calls.some(
        ({ cmd, args }) =>
          cmd === "plugin:window|set_theme" && args.value === "light",
      ),
    );
    await page.reload();
    await page.getByRole("heading", { name: "Fisch", exact: true }).waitFor();
    assert.equal(
      await page.locator("html").getAttribute("data-theme"),
      "light",
    );
    await page
      .getByRole("button", { name: "App settings", exact: true })
      .click();
    await page
      .getByRole("combobox", { name: "Color mode" })
      .selectOption("system");
    await page.emulateMedia({ colorScheme: "dark" });
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "dark",
    );
    await page.emulateMedia({ colorScheme: "light" });
    await page.waitForFunction(
      () => document.documentElement.dataset.theme === "light",
    );
    const overlay = await page.context().newPage();
    overlay.on("pageerror", (error) => errors.push(error.message));
    await overlay.goto(base + "/?overlay=1");
    await overlay.waitForFunction(() =>
      document.documentElement.classList.contains("overlay-body"),
    );
    await page
      .getByRole("combobox", { name: "Color mode" })
      .selectOption("dark");
    await overlay.waitForFunction(
      () => document.documentElement.dataset.theme === "dark",
    );
    const overlayState = await overlay.evaluate(() => ({
      html: getComputedStyle(document.documentElement).backgroundColor,
      body: getComputedStyle(document.body).backgroundColor,
      nativeThemeCalls: window.calls.filter(
        ({ cmd }) => cmd === "plugin:window|set_theme",
      ).length,
    }));
    assert.equal(overlayState.html, "rgba(0, 0, 0, 0)");
    assert.equal(overlayState.body, "rgba(0, 0, 0, 0)");
    assert.equal(
      overlayState.nativeThemeCalls,
      0,
      "Overlay must keep its transparent native window",
    );
    await overlay.close();
    await page
      .getByRole("combobox", { name: "Color mode" })
      .selectOption("system");
    fs.writeFileSync(
      "build/ui-proof/appearance-report.json",
      JSON.stringify({ palette, overlayState }, null, 2),
    );
    assert.equal(errors.length, 0, errors.join("\n"));
    console.log(
      "UI smoke passed: project, required changelog, publication, history, no creator exports, settings, appearance persistence/system changes, native theme commands, dark contrast and transparent overlays.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});

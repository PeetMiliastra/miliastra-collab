const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

(async () => {
  const root = path.resolve(__dirname, "..");
  const fixture = path.join(root, ".tools/asset-browser-layout.html");
  const output = path.join(root, "build/ui-proof/asset-browser");
  assert(!fs.existsSync(fixture), "Another layout proof is using the fixture");
  fs.mkdirSync(path.dirname(fixture), { recursive: true });
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(
    fixture,
    `<!doctype html><html><head><meta charset="UTF-8"></head><body><div id="proof"></div>
<script>
window.proofCalls = [];
window.__TAURI_INTERNALS__ = {invoke: async (command, args) => {
  window.proofCalls.push({command, args});
  const kinds = ["Prefab", "Entity", "Terrain", "Camera", "Node graph", "Configuration", "Player Template", "Creation", "Loot", "Character Skill", "Unit Status", "Projectile", "Item", "VFX Tool", "Character Control Skill", "Shield", "Scan Tag Template", "Environment", "Class", "Growth Curve", "Inventory Template", "Attribute Group"];
  const names = ["Fishing boat", "Boat deck", "Island shore", "Harbor camera", "Passenger boarding", "Diving controls", "Custom Player Template", "Large Hydro Slime", "Challenge Coin Loot", "Custom Skill", "New Status", "New Projectile", "New Item", "VFX Tool", "Control Skill", "Shield", "Scan Tag Template", "Environment Configuration", "Custom Class", "Custom Growth Curve", "Inventory Template", "Attribute Group"];
  if (command === "inspect_slice") return {
    section: args.section, changes: [], otherChanges: false, baselineAvailable: true,
    assets: kinds.flatMap((kind, i) => Array.from({length: i === 1 ? 12 : 2}, (_, j) => ({
      key: kind + ":" + (1078263809 + i * 100 + j), id: 1078263809 + i * 100 + j,
      name: names[i] + (j ? " " + (j + 1) : ""), kind, section: Number(args.section), canGenerateId: true
    }))).concat([{key: "Node graph:1610612737", id: 1610612737, name: "Stage graph", kind: "Node graph", section: Number(args.section), canGenerateId: false}])
  };
  if (command === "generate_move_id") return {token: "local-proof", oldId: 1078263909, newId: 1078329445, name: "Boat deck", source: args.source, target: args.target, expires: 9999999999};
  if (command === "validate_move_id") return "1078329445";
  throw Error("Unexpected proof command: " + command);
}};
</script>
<script type="module">
import React from "react";
import {createRoot} from "react-dom/client";
import {MoveAsset} from "/src/SliceTools.tsx";
import {empty} from "/src/App.tsx";
import "/src/style.css";
const section = {id: "5", name: "Boats", changed: false, conflict: false, installed: true, downloaded: true, lock: null};
const state = {...empty, online: false, sections: [section, {...section, id: "6", name: "Islands"}]};
function Proof() {
 const [busy, setBusy] = React.useState(false);
 const run = async (_, action, options) => {setBusy(true); try {await action();} catch(e) {options?.onError?.(String(e));} finally {setBusy(false);}};
 return React.createElement(MoveAsset, {state, busy, run, onClose: () => {}});
}
createRoot(document.getElementById("proof")).render(React.createElement(Proof));
</script></body></html>`,
  );
  let browser;
  try {
    browser = await chromium.launch({
      headless: true,
      executablePath:
        process.env.CHROME_PATH ||
        "C:/Program Files/Google/Chrome/Application/chrome.exe",
    });
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const base = process.env.MCOLLAB_UI_URL || "http://127.0.0.1:1420";
    await page.goto(base + "/.tools/asset-browser-layout.html");
    const dialog = page.getByRole("dialog", {
      name: "Move asset between slices",
    });
    await dialog
      .getByRole("button", {
        name: "Fishing boat — Prefab (1078263809)",
        exact: true,
      })
      .waitFor();
    assert(!(await dialog.innerText()).includes(String.fromCodePoint(0x00b7)));
    const layouts = [];
    for (const [width, height] of [
      [1180, 810],
      [900, 640],
      [600, 800],
    ]) {
      await page.setViewportSize({ width, height });
      const layout = await dialog.evaluate((modal) => {
        const rect = (selector) => {
          const r = modal.querySelector(selector).getBoundingClientRect();
          return { x: r.x, y: r.y, right: r.right, bottom: r.bottom };
        };
        const box = modal.getBoundingClientRect();
        const results = modal.querySelector(".asset-browser-results");
        return {
          box: { x: box.x, y: box.y, right: box.right, bottom: box.bottom },
          sidebar: rect(".asset-browser-sidebar"),
          assets: rect(".asset-browser-main"),
          modalOverflow: modal.scrollWidth > modal.clientWidth,
          scrollable: results.scrollHeight > results.clientHeight,
        };
      });
      assert(
        layout.sidebar.right <= layout.assets.x + 1,
        "Categories should be beside the assets",
      );
      assert(!layout.modalOverflow, "Modal overflows horizontally");
      assert(
        layout.box.x >= 0 &&
          layout.box.right <= width &&
          layout.box.y >= 0 &&
          layout.box.bottom <= height + 1,
      );
      assert(layout.scrollable, "Large asset lists need independent scrolling");
      layouts.push({ width, height, ...layout });
      await page.screenshot({
        path: path.join(output, `browser-${width}x${height}.png`),
      });
    }
    await page.setViewportSize({ width: 1180, height: 810 });
    for (const category of [
      "Creations",
      "Loot",
      "Character Skill",
      "Unit Status",
      "Projectile",
      "Item",
      "VFX Tool",
      "Character Control Skill",
      "Shield",
      "Scan Tag Template",
      "Environment",
      "Class",
      "Growth Curve",
      "Inventory Template",
      "Attribute Group",
      "Other",
    ]) {
      await dialog.getByRole("button", { name: category, exact: true }).click();
      await dialog
        .getByRole("region", { name: category, exact: true })
        .waitFor();
    }
    assert.equal(
      await dialog
        .getByRole("button", { name: "Configurations", exact: true })
        .count(),
      0,
    );
    await dialog
      .getByRole("button", { name: "Player Templates", exact: true })
      .click();
    await dialog
      .getByRole("button", {
        name: "Custom Player Template — Player Template (1078264409)",
        exact: true,
      })
      .waitFor();
    assert.equal(
      await dialog.getByRole("button", { name: /^Fishing boat —/ }).count(),
      0,
    );
    await page.screenshot({ path: path.join(output, "player-templates.png") });
    await dialog
      .getByRole("button", { name: "In-world objects", exact: true })
      .click();
    assert.equal(
      await dialog.getByRole("button", { name: /^Fishing boat —/ }).count(),
      0,
    );
    await dialog
      .getByRole("searchbox", { name: "Search assets" })
      .fill("Boat deck");
    const card = dialog.getByRole("button", {
      name: "Boat deck — Entity (1078263909)",
      exact: true,
    });
    await card.focus();
    await page.keyboard.press("Enter");
    assert.equal(await card.getAttribute("aria-pressed"), "true");
    await dialog.getByLabel("To slice", { exact: true }).selectOption("6");
    await page.screenshot({ path: path.join(output, "selected.png") });
    await dialog
      .getByRole("button", { name: "Generate GUID", exact: true })
      .click();
    await dialog.getByLabel("New GUID", { exact: true }).waitFor();
    assert.equal(
      await dialog.getByLabel("New GUID", { exact: true }).inputValue(),
      "1078329445",
    );
    await page.screenshot({ path: path.join(output, "generated.png") });
    for (const [width, height] of [
      [1180, 810],
      [900, 640],
    ]) {
      await page.setViewportSize({ width, height });
      const footer = await dialog.locator(".modal-actions").boundingBox();
      assert(
        footer && footer.y >= 0 && footer.y + footer.height <= height - 10,
        "Generated GUID must leave the footer visible",
      );
      await page.screenshot({
        path: path.join(output, `generated-${width}x${height}.png`),
      });
    }
    await dialog
      .getByRole("button", { name: "All assets", exact: true })
      .click();
    assert.equal(
      await dialog.getByLabel("New GUID", { exact: true }).count(),
      0,
    );
    assert(
      await dialog
        .getByRole("button", { name: "Generate GUID", exact: true })
        .isDisabled(),
    );
    await dialog
      .getByRole("searchbox", { name: "Search assets" })
      .fill("Stage graph");
    assert(
      await dialog.getByRole("button", { name: /^Stage graph —/ }).isDisabled(),
    );
    await dialog
      .getByRole("searchbox", { name: "Search assets" })
      .fill("no matching asset");
    await dialog
      .getByText("No assets match your search.", { exact: true })
      .waitFor();
    await dialog.getByLabel("From slice", { exact: true }).selectOption("6");
    assert.equal(
      await dialog
        .getByRole("searchbox", { name: "Search assets" })
        .inputValue(),
      "",
    );
    assert.equal(
      await dialog.getByLabel("To slice", { exact: true }).inputValue(),
      "",
    );
    const calls = await page.evaluate(() => window.proofCalls);
    assert(
      calls.some(
        ({ command, args }) =>
          command === "generate_move_id" &&
          args.source === "5" &&
          args.target === "6" &&
          args.assetKey === "Entity:1078263909",
      ),
    );
    assert(
      calls.every(({ command }) =>
        ["inspect_slice", "generate_move_id", "validate_move_id"].includes(
          command,
        ),
      ),
    );
    assert.deepEqual(errors, []);
    fs.writeFileSync(
      path.join(output, "report.json"),
      JSON.stringify(
        { layouts, commands: calls.map(({ command }) => command) },
        null,
        2,
      ),
    );
    console.log(
      "Asset browser layout passed: three window sizes, independent scrolling, category/search selection, keyboard selection, local generation, invalid IDs, and source changes.",
    );
  } finally {
    if (browser) await browser.close();
    fs.unlinkSync(fixture);
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});

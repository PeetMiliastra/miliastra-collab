const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
(async () => {
  const browser = await chromium.launch({
    headless: true,
    executablePath:
      process.env.CHROME_PATH ||
      "C:/Program Files/Google/Chrome/Application/chrome.exe",
  });
  const page = await browser.newPage({
    viewport: { width: 1280, height: 900 },
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("http://127.0.0.1:1420");
  await page
    .getByRole("button", { name: "Explore a demo project", exact: true })
    .click();
  await page
    .getByRole("heading", { name: "Fisch — demo", exact: true })
    .waitFor();
  assert(await page.getByText("Browser preview").isVisible());
  fs.mkdirSync("build/ui-proof", { recursive: true });
  await page.screenshot({
    path: "build/ui-proof/demo-project.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: /Share update/ }).click();
  const submit = page.getByRole("button", {
    name: "Share update",
    exact: true,
  });
  assert(await submit.isDisabled());
  await page
    .getByPlaceholder("Fixed passengers falling through the boat deck")
    .fill("Tried sharing in the demo");
  await page
    .getByRole("dialog")
    .locator("label")
    .filter({ hasText: "Quests" })
    .getByRole("checkbox")
    .uncheck();
  await submit.click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByText("Tried sharing in the demo", { exact: true }).waitFor();
  await page.getByRole("button", { name: "Load earlier changes" }).click();
  await page.screenshot({
    path: "build/ui-proof/demo-history.png",
    fullPage: true,
  });
  await page
    .getByRole("combobox", { name: "Slice", exact: true })
    .selectOption("5");
  await page.getByText("Tried sharing in the demo", { exact: true }).waitFor();
  await page.reload();
  await page
    .getByRole("heading", { name: "Fisch — demo", exact: true })
    .waitFor();
  await page
    .getByRole("button", { name: "Leave preview", exact: true })
    .click();
  await page
    .getByRole("heading", { name: "Your projects", exact: true })
    .waitFor();
  assert.equal(
    await page.evaluate(() =>
      localStorage.getItem("miliastra-collab-preview-active"),
    ),
    null,
  );
  assert.equal(errors.length, 0, errors.join("\n"));
  await browser.close();
  console.log(
    "Demo browser proof passed: explicit entry, labelled sample data, share, history pagination/slice filtering, restart persistence, safe exit.",
  );
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

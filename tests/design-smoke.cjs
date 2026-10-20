// Layout checks in the browser preview.
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
  try {
    const page = await browser.newPage({
      viewport: { width: 1180, height: 810 },
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
    await page.evaluate(() => {
      const key = "miliastra-collab-preview-v1",
        data = JSON.parse(localStorage.getItem(key));
      data.snapshot.compatibility.directInstall = true;
      localStorage.setItem(key, JSON.stringify(data));
    });
    await page.reload();
    await page
      .getByRole("heading", { name: "Fisch — demo", exact: true })
      .waitFor();
    assert.equal(
      await page
        .getByRole("region", { name: "Collaborator activity", exact: true })
        .getByRole("article")
        .count(),
      4,
    );
    assert(
      await page
        .getByRole("article", { name: "Your work", exact: true })
        .isVisible(),
    );
    assert.equal(await page.locator(".identity, .saved-note").count(), 0);
    assert.match(
      await page
        .locator(".section-row")
        .filter({
          has: page.getByRole("button", { name: "Boats", exact: true }),
        })
        .locator(".working-person")
        .innerText(),
      /You/,
    );
    assert(
      await page
        .getByText("You and a teammate both changed Quests", { exact: true })
        .isVisible(),
    );
    assert(
      await page
        .getByRole("button", { name: "Review and apply", exact: true })
        .isEnabled(),
    );
    assert.match(
      await page.locator("#install-blocker").innerText(),
      /Review Quests/,
    );
    assert.equal(
      await page
        .locator(".section-row")
        .filter({ has: page.getByText("Common", { exact: true }) })
        .count(),
      1,
    );
    assert.equal(
      await page.getByLabel("Slice 1", { exact: true }).innerText(),
      "1",
    );
    const columns = await page
      .locator(".table-head th")
      .evaluateAll((th) => th.map((el) => el.getBoundingClientRect().width));
    assert(columns[0] > columns[2] * 2);
    // Slice names get the most room; the action column fits its buttons.
    assert(columns[0] > Math.max(...columns.slice(1)));
    assert.equal(
      await page
        .locator("nav button.active")
        .evaluate((el) => getComputedStyle(el).boxShadow),
      "none",
    );
    fs.mkdirSync("build/ui-proof", { recursive: true });
    const headingY = await page
      .locator(".page-heading")
      .evaluate((e) => e.getBoundingClientRect().y);
    await page.locator(".progress-slot").evaluate((e) => {
      e.innerHTML =
        '<div class="progress" role="status">Reserving slice…</div>';
    });
    assert.equal(
      await page
        .locator(".page-heading")
        .evaluate((e) => e.getBoundingClientRect().y),
      headingY,
      "Progress moved the page",
    );
    await page.locator(".progress-slot").evaluate((e) => e.replaceChildren());
    const boats = page
      .locator(".section-list")
      .getByRole("button", { name: "Boats", exact: true });
    await boats.click();
    await page
      .getByRole("textbox", { name: "Name for Boats", exact: true })
      .fill("Harbour");
    assert.equal(await page.getByRole("dialog").count(), 0);
    await page.screenshot({ path: "build/ui-proof/inline-name.png" });
    await page
      .getByRole("textbox", { name: "Name for Boats", exact: true })
      .press("Escape");
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: "build/ui-proof/redesign-project.png" });
    const ratio = await page
      .locator(".slice-badge.changed")
      .first()
      .evaluate((el) => {
        const luminance = (c) => {
          const a = c
            .match(/[\d.]+/g)
            .slice(0, 3)
            .map(Number)
            .map((v) => {
              v /= 255;
              return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
            });
          return a[0] * 0.2126 + a[1] * 0.7152 + a[2] * 0.0722;
        };
        const style = getComputedStyle(el),
          a = luminance(style.color),
          b = luminance(style.backgroundColor);
        return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      });
    assert(ratio >= 4.5, "Local change summary contrast too low");
    assert(
      (await page
        .locator(".pill")
        .first()
        .evaluate((e) => parseFloat(getComputedStyle(e).fontSize))) >= 13,
    );
    assert.equal(
      await page
        .getByRole("button", { name: "All sections", exact: true })
        .count(),
      0,
    );
    for (const [width, height] of [
      [900, 640],
      [1440, 900],
      [590, 820],
      [375, 812],
    ]) {
      await page.setViewportSize({ width, height });
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth > innerWidth,
        ),
        false,
        "Overflow at " + width,
      );
      const owner = page
        .locator(".section-row")
        .filter({ has: page.getByText("Islands", { exact: true }) })
        .locator(".working-person");
      assert(await owner.isVisible(), "Reservation owner hidden at " + width);
      assert.match(await owner.innerText(), /Alex/);
      await page.screenshot({
        path: "build/ui-proof/redesign-" + width + ".png",
        fullPage: width < 760,
      });
    }
    await page.setViewportSize({ width: 1180, height: 810 });
    await page.getByRole("button", { name: /^Share update/ }).click();
    await page.getByRole("dialog").waitFor();
    assert.equal(
      await page
        .getByRole("button", { name: "Refresh saved changes", exact: true })
        .count(),
      0,
    );
    assert.equal(await page.getByText(/Your draft is kept/).count(), 0);
    assert(
      await page
        .getByRole("button", { name: "Cancel", exact: true })
        .isVisible(),
    );
    assert(
      await page
        .getByRole("button", { name: "Share update", exact: true })
        .isDisabled(),
    );
    await page
      .getByPlaceholder("Fixed passengers falling through the boat deck")
      .fill("Reviewed the boat deck");
    await page.screenshot({ path: "build/ui-proof/redesign-share.png" });
    await page.keyboard.press("Escape");
    assert((await page.getByRole("dialog").count()) === 0);
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.locator(".history-entry").first().waitFor();
    assert((await page.locator(".history-day").count()) > 0);
    assert.equal(
      await page.getByRole("combobox", { name: "Filter by creator" }).count(),
      0,
    );
    assert(
      await page
        .getByRole("combobox", { name: "Slice", exact: true })
        .isVisible(),
    );
    await page.screenshot({ path: "build/ui-proof/redesign-history.png" });
    await page.locator(".history-entry").first().click();
    assert.equal(
      await page.getByText("Add a correction", { exact: true }).count(),
      0,
    );
    assert.equal(
      await page.getByText("Revision details", { exact: true }).count(),
      0,
    );
    await page.keyboard.press("Escape");
    await page
      .getByRole("button", { name: "Project settings", exact: true })
      .click();
    await page
      .getByRole("heading", { name: "Project settings", exact: true })
      .waitFor();
    assert(
      (await page
        .getByRole("button", { name: "Open team folder", exact: true })
        .evaluate((e) => parseFloat(getComputedStyle(e).marginTop))) >= 20,
    );
    await page.screenshot({ path: "build/ui-proof/redesign-settings.png" });
    await page
      .getByRole("button", { name: "All projects", exact: true })
      .click();
    await page
      .getByRole("heading", { name: "Your projects", exact: true })
      .waitFor();
    assert(
      await page
        .getByRole("button", { name: "Create project", exact: true })
        .isVisible(),
    );
    assert.equal(
      await page
        .getByRole("button", { name: "Join project", exact: true })
        .count(),
      0,
    );
    assert.deepEqual(errors, []);
    console.log(
      "Design browser checks passed: all slices/ownership, blockers, readable labels, contrast " +
        ratio.toFixed(2) +
        ":1, five widths, share/Escape, History slice filter and overview.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

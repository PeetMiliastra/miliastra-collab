const { PrivateWire } = require("./private-wire.cjs");
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
(async () => {
  const base = process.env.MCOLLAB_TEST_URL || "http://127.0.0.1:18787";
  const codec = PrivateWire.generate(base);
  const response = await fetch(base + "/projects", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(
      codec.create({
        name: "Invitation browser proof",
        displayName: "Owner",
        storage: { kind: "folder", location: "" },
        baseline: [],
        baselineId: crypto.randomUUID(),
      }),
    ),
  });
  assert(response.ok);
  const joined = codec.openValue(await response.json()),
    i = joined.invitation;
  const browser = await chromium.launch({
    headless: true,
    executablePath:
      process.env.CHROME_PATH ||
      "C:/Program Files/Google/Chrome/Application/chrome.exe",
  });
  const page = await browser.newPage({
    viewport: { width: 1000, height: 800 },
    acceptDownloads: true,
  });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const link = base + "/invite/" + i.project + "#" + i.invite;
  await page.goto(link);
  assert.equal(await page.title(), "Join a project - miliastra/collab");
  await page.getByText("miliastra/collab", { exact: true }).waitFor();
  await page
    .getByRole("heading", { name: "Join project", exact: true })
    .waitFor();
  await page.waitForFunction(() => {
    const img = document.querySelector(".brand img");
    return img?.complete && img.naturalWidth === 512;
  });
  assert.equal(
    await page.locator("link[rel=icon]").getAttribute("href"),
    "../brand/logo.png",
  );
  await page.getByText("Your invitation is ready.", { exact: false }).waitFor();
  assert.equal(
    await page.getByLabel("Your invitation link").inputValue(),
    link,
  );
  const opener = page.getByRole("button", {
    name: "Open miliastra/collab",
    exact: true,
  });
  assert(!(await opener.isDisabled()));
  const protocol = new URL(await opener.getAttribute("href"));
  assert.equal(protocol.protocol, "miliastra-collab:");
  assert.equal(protocol.hostname, "invite");
  assert.equal(protocol.pathname, "/" + i.project);
  assert.equal(protocol.searchParams.get("service"), base);
  assert.equal(protocol.hash, "#" + i.invite);
  assert(!protocol.search.includes(i.invite));
  await page.getByText("App didn’t open?", { exact: true }).click();
  assert.equal(
    await page.getByRole("button", { name: "Download invitation" }).count(),
    0,
  );
  assert.equal(
    await page
      .getByRole("link", { name: "Download miliastra/collab", exact: true })
      .getAttribute("href"),
    "https://miliastra.dev",
  );
  await page
    .getByText(
      "Copy the Encryption Key from your teammate’s invitation message into the desktop app.",
      { exact: false },
    )
    .waitFor();
  assert(
    !/privacy key|install it/i.test(await page.locator("main").innerText()),
  );
  fs.mkdirSync("build/ui-proof", { recursive: true });
  await page.screenshot({ path: "build/ui-proof/invitation-page.png" });
  await page.goto(base + "/invite/" + i.project + "#" + crypto.randomUUID());
  await page
    .getByText("This invitation has expired or is invalid.", { exact: false })
    .waitFor();
  assert(
    await page
      .getByRole("button", { name: "Open miliastra/collab", exact: true })
      .isDisabled(),
  );
  assert.equal(
    await page
      .getByRole("button", { name: "Open miliastra/collab", exact: true })
      .getAttribute("href"),
    null,
  );
  const view = await fetch(base + "/projects/" + i.project, {
    headers: { authorization: "Bearer " + joined.token },
  });
  assert.equal(
    (await view.json()).members.length,
    1,
    "Viewing an invite must not create membership",
  );
  assert.equal(errors.length, 0, errors.join("\n"));
  await browser.close();
  console.log(
    "Invitation browser proof passed: hosted landing page, fragment secret, validation, protocol handoff, expired link, no automatic membership.",
  );
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

const { PrivateWire } = require("./private-wire.cjs");
const { chromium } = require("playwright");
const crypto = require("node:crypto");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(
    path.basename(root).startsWith("invitation-smoke-"),
    "Refusing a non-isolated profile",
  );
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19229");
  try {
    const page = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => !p.url().includes("overlay=1"));
    const invoke = (cmd, args = {}) =>
      page.evaluate(
        ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
        { cmd, args },
      );
    assert.equal(
      path.resolve((await invoke("snapshot")).testDataDirectory),
      root,
    );
    const checkpoint = path.join(root, "invitation-checkpoint.json");
    if (process.env.MCOLLAB_INVITATION_PHASE === "resume") {
      const saved = JSON.parse(fs.readFileSync(checkpoint));
      await page
        .getByRole("dialog", { name: "Join Shared project", exact: true })
        .waitFor();
      await page
        .getByRole("list", { name: "Join progress", exact: true })
        .waitFor();
      const pending = await invoke("pending_invitation");
      assert.equal(pending.project, saved.project);
      assert.equal(pending.endpoint, saved.endpoint);
      const state = await invoke("snapshot");
      assert.equal(state.project.id, saved.project);
      assert.equal(state.project.members.length, saved.members);
      console.log(
        "Native cold-start protocol invitation passed without automatic membership or workspace changes.",
      );
      return;
    }
    await invoke("begin_practice");
    await page
      .getByRole("heading", { name: "Fisch — practice", exact: true })
      .waitFor();
    await page.locator(".progress").waitFor({ state: "hidden" });
    const state = await invoke("snapshot");
    const link = await invoke("invite");
    const web = new URL(link);
    const protocol = new URL("miliastra-collab://invite/" + state.project.id);
    protocol.searchParams.set("service", web.origin);
    protocol.hash = web.hash;
    const output = execFileSync(process.execPath, ["tests/invite-smoke.cjs"], {
      env: { ...process.env, MCOLLAB_TEST_URL: web.origin },
      windowsHide: true,
      encoding: "utf8",
    });
    process.stdout.write(output);
    // Isolated test runs skip the single-instance lock, so a second app can't hand the link
    // over. Give it to this app the way a test profile's Invitation link field does.
    await invoke("open_invitation_link", { link });
    // The join screen can't know the project's name before the team folder is shared.
    const dialog = page.getByRole("dialog", {
      name: "Join Shared project",
      exact: true,
    });
    await dialog.waitFor();
    await dialog
      .getByRole("list", { name: "Join progress", exact: true })
      .waitFor();
    const pending = await invoke("pending_invitation");
    assert.equal(pending.project, state.project.id);
    assert.equal(pending.endpoint, web.origin);
    const metadata = await invoke("invitation_details", {
      invitation: pending,
    });
    assert.equal(metadata.projectName, "Shared project");
    assert.equal(metadata.storageKind, "folder");
    await dialog
      .getByRole("list", { name: "Join progress", exact: true })
      .waitFor();
    assert.equal(
      (await invoke("snapshot")).project.members.length,
      state.project.members.length,
    );
    fs.mkdirSync("build/ui-proof", { recursive: true });
    await page.screenshot({ path: "build/ui-proof/invitation-app.png" });
    await dialog.getByRole("button", { name: "Close", exact: true }).click();
    for (let i = 0; i < 30; i++) {
      if ((await invoke("pending_invitation")) === null) break;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.equal(
      await invoke("pending_invitation"),
      null,
      "Cold start must not reuse a pending invitation from this phase",
    );
    // Cloud invitation without signing in to a provider or transferring files.
    const cloudCodec = PrivateWire.generate(web.origin);
    const cloudResponse = await fetch(web.origin + "/projects", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(
        cloudCodec.create({
          name: "Cloud invitation check",
          displayName: "Owner",
          storage: { kind: "google", location: "opaque-team-folder" },
          baseline: [],
          baselineId: crypto.randomUUID(),
        }),
      ),
    });
    assert(cloudResponse.ok);
    const cloud = cloudCodec.openValue(await cloudResponse.json());
    await invoke("save_folder_privacy_key", {
      invitation: cloud.invitation,
      key: cloudCodec.export(),
    });
    await invoke("open_invitation_link", {
      link:
        web.origin +
        "/invite/" +
        cloud.project.id +
        "#" +
        cloud.invitation.invite,
    });
    const cloudDialog = page.getByRole("dialog", {
      name: "Join Shared project",
      exact: true,
    });
    await cloudDialog
      .getByRole("list", { name: "Join progress", exact: true })
      .waitFor();
    assert.equal(
      await cloudDialog
        .getByRole("group", { name: "Google Drive account" })
        .count(),
      0,
    );
    assert(
      await cloudDialog
        .getByRole("alert")
        .filter({ hasText: "Google Drive" })
        .isVisible(),
    );
    assert.equal(
      await cloudDialog
        .getByRole("group", { name: "OneDrive account" })
        .count(),
      0,
    );
    assert.equal(
      await cloudDialog
        .getByRole("button", { name: "Choose shared folder" })
        .count(),
      0,
    );
    assert.equal((await invoke("snapshot")).project.id, state.project.id);
    assert.equal(
      (await invoke("snapshot")).project.members.length,
      state.project.members.length,
    );
    await page.screenshot({
      path: "build/ui-proof/redesign-cloud-invitation.png",
    });
    await cloudDialog
      .getByRole("button", { name: "Close", exact: true })
      .click();
    for (let i = 0; i < 30; i++) {
      if ((await invoke("pending_invitation")) === null) break;
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
    assert.equal(await invoke("pending_invitation"), null);
    fs.writeFileSync(
      checkpoint,
      JSON.stringify({
        protocol: protocol.href,
        endpoint: web.origin,
        project: state.project.id,
        members: state.project.members.length,
      }),
    );
    console.log(
      "Native running-app protocol invitation passed: delivery, validation, join dialog over workspace, dismissal and no automatic membership.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

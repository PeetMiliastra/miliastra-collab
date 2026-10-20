const { PrivateWire } = require("./private-wire.cjs");
const { chromium } = require("playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const root = path.resolve(
  process.env.MCOLLAB_TEST_DATA_DIR || ".tools/native-smoke-state",
);
const sourceStage = process.env.MCOLLAB_SMOKE_STAGE;
assert(
  sourceStage,
  "Set MCOLLAB_SMOKE_STAGE to an existing stage for the read-only source fixture",
);
let stage = sourceStage;
const digest = () => {
  const result = {};
  function walk(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else
        result[p] = crypto
          .createHash("sha256")
          .update(fs.readFileSync(p))
          .digest("hex");
    }
  }
  walk(sourceStage);
  return result;
};
(async () => {
  const before = digest();
  const http = require("node:http");
  const requests = [];
  const proxy = http.createServer((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const bytes = Buffer.concat(chunks);
      requests.push({ path: req.url, body: bytes.toString() });
      const upstream = http.request(
        {
          hostname: "127.0.0.1",
          port: 18787,
          path: req.url,
          method: req.method,
          headers: req.headers,
        },
        (response) => {
          res.writeHead(response.statusCode, response.headers);
          response.pipe(res);
        },
      );
      upstream.on("error", () => {
        res.writeHead(502);
        res.end();
      });
      upstream.end(bytes);
    });
  });
  await new Promise((resolve) => proxy.listen(18788, "127.0.0.1", resolve));
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19223");
  const page = browser
    .contexts()
    .flatMap((c) => c.pages())
    .find((p) => !p.url().includes("overlay=1"));
  assert(page, "Main native WebView was not found");
  await page.waitForFunction(() => window.__TAURI_INTERNALS__?.invoke);
  const invoke = (cmd, args = {}) =>
    page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  const initial = await invoke("snapshot");
  assert.equal(
    path.resolve(initial.testDataDirectory || ""),
    root,
    "Refusing to mutate a non-isolated profile",
  );
  assert.equal(initial.project, null, "Use a fresh test profile");
  const settings = {
    ...initial.settings,
    endpoint: "http://127.0.0.1:18788",
    autostart: false,
  };
  await invoke("save_settings", { settings });
  const stages = await invoke("discover");
  assert(
    stages.some(
      (s) =>
        path.resolve(s.path.replace(/^\\\\\?\\/, "")) === path.resolve(stage),
    ),
  );
  const discovered = stages.find(
    (s) =>
      path.resolve(s.path.replace(/^\\\\\?\\/, "")) === path.resolve(stage),
  );
  const fixtureRoot = path.join(root, "stages");
  stage = path.join(
    fixtureRoot,
    discovered.account,
    "Beyond_Local_Save_Level",
    discovered.level,
  );
  fs.cpSync(sourceStage, stage, {
    recursive: true,
    errorOnExist: true,
    force: false,
  });
  await invoke("save_settings", {
    settings: { ...settings, saveRoot: fixtureRoot },
  });
  const exchange = path.join(root, "exchange");
  fs.mkdirSync(exchange, { recursive: true });
  await invoke("create_project", {
    name: "Desktop smoke",
    displayName: "Smoke creator",
    stagePath: stage,
    kind: "folder",
    folder: exchange,
  });
  let state = await invoke("snapshot");
  assert(state.online);
  assert.equal(state.sections.length, 65);
  assert.equal(
    state.compatibility.directInstall,
    true,
    "Recognized discovered stages can install slices without an account or stage allowlist",
  );
  assert.equal(
    state.project.locks.length,
    0,
    "Historical selection must not acquire a lock",
  );
  const project = state.project.id;
  const member = state.member;
  const overlayTest =
    state.game.foreground && process.env.MCOLLAB_SKIP_OVERLAY_PROOF !== "1";
  await invoke("start_work", { section: "5", takeover: false });
  if (overlayTest) {
    const overlay = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => p.url().includes("overlay=1"));
    assert(overlay, "Overlay WebView was not created");
    await overlay.getByText(/Reserved Boats/).waitFor();
    const { execFileSync } = require("node:child_process");
    const script = `Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public class OverlayProof{[DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern IntPtr FindWindow(string c,string n);[DllImport("user32.dll")]public static extern int GetWindowLong(IntPtr h,int n);[DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);}';$h=[OverlayProof]::FindWindow($null,'Collaboration notification');@{visible=[OverlayProof]::IsWindowVisible($h);style=[OverlayProof]::GetWindowLong($h,-20)}|ConvertTo-Json -Compress`;
    const proof = JSON.parse(
      execFileSync("powershell.exe", ["-NoProfile", "-Command", script], {
        encoding: "utf8",
        windowsHide: true,
      }),
    );
    assert(proof.visible);
    assert(proof.style & 0x20, "Overlay must be click-through");
    assert(proof.style & 0x08000000, "Overlay must not activate");
    assert(
      (await invoke("snapshot")).game.foreground,
      "Overlay stole game focus",
    );
    console.log(
      "Overlay native proof passed: visible above foreground game, click-through and no-activate window flags, game focus preserved.",
    );
  } else
    console.log("Overlay visibility proof skipped: game was not foreground.");

  state = await invoke("snapshot");
  assert.equal(state.sections.find((s) => s.id === "5").lock.member, member);
  const draft = {
    summary: "Native draft survives reconnect",
    details: "Isolated smoke test",
  };
  await invoke("save_draft", { draft, projectId: project });
  assert.deepEqual((await invoke("snapshot")).draft, draft);
  const varint = (n) => {
    const b = [];
    do {
      b.push((n % 128) | (n >= 128 ? 128 : 0));
      n = Math.floor(n / 128);
    } while (n);
    return Buffer.from(b);
  };
  const frame = (section, value) => {
    const data = Buffer.concat([
      Buffer.from([8]),
      varint(Number(state.stage.level)),
      varint(90 * 8),
      varint(value),
    ]);
    const payload = Buffer.concat([
      Buffer.from([8, section, 18]),
      varint(data.length),
      data,
    ]);
    const header = Buffer.alloc(20);
    [payload.length + 20, 1, 806, 6, payload.length].forEach((n, i) =>
      header.writeUInt32BE(n, i * 4),
    );
    const footer = Buffer.alloc(4);
    footer.writeUInt32BE(1657);
    return Buffer.concat([header, payload, footer]);
  };
  // Change the test stage, then share it through the app.
  const slice = path.join(
    stage,
    "beyond_level_slice",
    state.stage.level + "_6.gis",
  );
  fs.writeFileSync(slice, frame(5, 53));
  assert((await invoke("snapshot")).sections.find((s) => s.id === "5").changed);
  const own = await invoke("share_update", { sections: ["5"], draft });
  assert.equal(own.summary, draft.summary);
  assert.equal((await invoke("snapshot")).draft.summary, "");
  await invoke("save_draft", { draft, projectId: project });
  state = await invoke("snapshot");
  const history = await invoke("history_page", { filter: {} });
  assert.equal(history.entries.length, 2);
  assert.equal(history.entries[0].sequence, 2);

  await assert.rejects(
    invoke("install_updates", {
      planId: crypto.randomUUID(),
      editorReady: false,
    }),
    /Save and unload/i,
  );
  // A second creator writes made-up slice files straight to team storage.
  const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");
  const owner = fs.readFileSync(
    path.join(
      root,
      "test-secrets",
      sha(Buffer.from("membership:http://127.0.0.1:18788:" + project)),
    ),
    "utf8",
  );
  const codec = new PrivateWire(
    await invoke("folder_privacy_key"),
    "http://127.0.0.1:18788",
    project,
  );
  const api = async (suffix, body, token = owner) => {
    const response = await fetch(
      "http://127.0.0.1:18787/projects/" + project + suffix,
      {
        method: body === undefined ? "GET" : "POST",
        headers: {
          "content-type": "application/json",
          authorization: "Bearer " + token,
        },
        body:
          body === undefined
            ? undefined
            : JSON.stringify(codec.command(suffix, body)),
      },
    );
    const value = await response.json();
    assert(response.ok, JSON.stringify(value));
    return codec.openValue(value);
  };
  const invitation = await api("/invite", {});
  const guest = await api("/join", {
    invite: invitation.invite,
    displayName: "Synthetic collaborator",
  });
  const lease = await api("/locks/acquire", { section: "6" }, guest.token);
  const publish = async (value, parent, corrupt = false) => {
    const data = Buffer.concat([
      Buffer.from([8]),
      varint(1073938433),
      varint(90 * 8),
      varint(value),
    ]);
    const payload = Buffer.concat([
      Buffer.from([8, 6, 18]),
      varint(data.length),
      data,
    ]);
    const header = Buffer.alloc(20);
    [payload.length + 20, 1, 806, 6, payload.length].forEach((n, i) =>
      header.writeUInt32BE(n, i * 4),
    );
    const footer = Buffer.alloc(4);
    footer.writeUInt32BE(1657);
    const bytes = Buffer.concat([header, payload, footer]);
    const hash = sha(bytes);
    const publication = crypto.randomUUID();
    const artifact = `History/${publication}/7.gis`;
    fs.mkdirSync(path.join(exchange, "History", publication), {
      recursive: true,
    });
    fs.writeFileSync(
      path.join(exchange, artifact),
      corrupt ? Buffer.alloc(bytes.length) : bytes,
    );
    const revision = {
      section: "6",
      name: "Islands",
      hash,
      contentHash: sha(data),
      size: bytes.length,
      parent,
      artifact,
      sharedDependency: state.project.heads.shared.hash,
    };
    const update = await api(
      "/publish",
      {
        id: publication,
        summary: "Synthetic download " + value,
        details: "Transport validation only; not an editor compatibility proof",
        revisions: [revision],
        fences: { 6: lease.fence },
        restores: null,
      },
      guest.token,
    );
    return { bytes, hash, update, artifact };
  };
  const wait = async (test, ms = 20000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      const s = await invoke("snapshot");
      if (test(s)) return s;
      await new Promise((r) => setTimeout(r, 300));
    }
    throw Error("Timed out waiting for native download state");
  };
  const first = await publish(1, state.project.heads["6"].hash);
  await invoke("refresh");
  await wait((s) => s.sections.find((x) => x.id === "6").downloaded);
  assert.equal(
    (await invoke("history_page", { filter: { section: "6" } })).entries[0]
      .summary,
    "Synthetic download 1",
  );
  assert.equal(
    (await invoke("snapshot")).sections.find((x) => x.id === "6").installed,
    false,
  );
  const damaged = await publish(2, first.hash, true);
  await invoke("refresh");
  await wait((s) => !!s.sections.find((x) => x.id === "6").downloadError);
  assert.equal(
    fs.existsSync(path.join(root, "objects", damaged.hash)),
    false,
    "Corrupt download must never enter verified cache",
  );
  fs.writeFileSync(path.join(exchange, damaged.artifact), damaged.bytes);
  await wait((s) => s.sections.find((x) => x.id === "6").downloaded, 25000);
  assert.equal(
    (await invoke("history_page", { filter: {} })).entries.length,
    4,
  );
  if (state.compatibility.directInstall) {
    const target = path.join(
      stage,
      "beyond_level_slice",
      state.stage.level + "_7.gis",
    );
    const original = fs.readFileSync(target);
    const plan = await invoke("preview_install");
    assert(plan.revisions.some((r) => r.section === "6"));
    await assert.rejects(
      invoke("install_updates", { planId: plan.id, editorReady: false }),
      /Save and unload/i,
    );
    assert.deepEqual(fs.readFileSync(target), original);
    await page
      .getByRole("button", { name: "Review and apply", exact: true })
      .click();
    await page.getByRole("button", { name: "Continue", exact: true }).click();
    let dialog = page.getByRole("dialog");
    await dialog.waitFor();
    assert(
      await dialog
        .getByRole("button", { name: "Back up and apply" })
        .isDisabled(),
    );
    await page.screenshot({ path: "build/ui-proof/install-preview.png" });
    await dialog.getByRole("checkbox").check();
    await dialog.getByRole("button", { name: "Back up and apply" }).click();
    await page.getByText("Updates applied", { exact: true }).waitFor();
    await page.screenshot({ path: "build/ui-proof/install-verified.png" });
    let pending = await invoke("snapshot");
    assert.equal(pending.installation.phase, "verified");
    assert.equal(
      pending.sections.find((x) => x.id === "6").installationPending,
      false,
    );
    assert.equal(pending.sections.find((x) => x.id === "6").installed, true);
    assert.deepEqual(
      fs.readFileSync(target),
      damaged.bytes,
      "Renaming must preserve original sender bytes",
    );
    assert.equal(
      fs.existsSync(path.join(stage, "beyond_level_slice", "1073938433_7.gis")),
      false,
    );
    await assert.rejects(invoke("verify_install"), /no pending update/i);
    await invoke("leave_project");
    await invoke("reopen_project", { projectId: project });
    assert.equal(
      (await invoke("snapshot")).installation.phase,
      "verified",
      "Completed installation must survive reconnect",
    );
    assert.equal(await page.locator(".task-resume").count(), 0);
    // Simulate an ID-only editor resave after automatic completion.
    fs.writeFileSync(target, frame(6, 2));
    await invoke("refresh_saved_changes");
    pending = await invoke("snapshot");
    assert.equal(pending.sections.find((x) => x.id === "6").changed, false);
    assert(pending.sections.find((x) => x.id === "6").installed);
    await assert.rejects(
      invoke("share_update", { sections: ["6"], draft }),
      /no saved changes/i,
    );
    await assert.rejects(
      invoke("undo_install", { editorReady: false }),
      /Save and unload/i,
    );
    fs.writeFileSync(target, frame(6, 3));
    await assert.rejects(
      invoke("undo_install", { editorReady: true }),
      /new local edits/i,
    );
    assert.equal(
      (await invoke("snapshot")).installation.phase,
      "verified",
      "Refusing dirty Undo must not invalidate the installed revision",
    );
    fs.writeFileSync(target, frame(6, 2));
    const installedDialog = page.getByRole("dialog", {
      name: "Apply updates",
      exact: true,
    });
    if (await installedDialog.isVisible())
      await installedDialog
        .getByRole("button", { name: "Done", exact: true })
        .click();
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.getByRole("button", { name: "Undo last update" }).click();
    dialog = page.getByRole("dialog", {
      name: "Undo last update",
      exact: true,
    });
    await dialog.waitFor();
    assert(
      await dialog
        .getByRole("button", { name: "Restore previous slices" })
        .isDisabled(),
    );
    await dialog.getByRole("checkbox").check();
    await dialog
      .getByRole("button", { name: "Restore previous slices" })
      .click();
    assert.deepEqual(fs.readFileSync(target), original);
    assert.equal((await invoke("snapshot")).installation.phase, "undone");
    await page
      .getByRole("heading", { name: "Undo complete", exact: true })
      .waitFor();
    await page
      .getByRole("dialog", { name: "Undo last update", exact: true })
      .getByRole("button", { name: "Done", exact: true })
      .click();
    await page
      .getByRole("navigation", { name: "Main navigation" })
      .getByRole("button", { name: /^Slices/ })
      .click();
    assert.equal((await invoke("snapshot")).installation.phase, "undone");
    console.log(
      "Native slice installation proof passed on a disposable stage copy: recognized layout, preview/consent, sender bytes under recipient filename, backup, automatic verification persistence, ID-only resave comparison, and automatic Undo.",
    );
  }
  const settingsFile = path.join(
    stage,
    "beyond_level_setting",
    "local_setting.gils",
  );
  const settingsBefore = fs.readFileSync(settingsFile);
  state = await invoke("snapshot");
  assert(state.nameSync);
  const namesDraft = {
    edits: {
      5: {
        name: "Ships",
        baseName: state.project.sectionNames["5"],
        parentVersion: state.project.namesVersion,
      },
    },
  };
  await invoke("save_names_draft", { projectId: project, draft: namesDraft });
  await invoke("leave_project");
  await invoke("reopen_project", { projectId: project });
  assert.deepEqual((await invoke("snapshot")).nameSync.draft, namesDraft);
  const namesRow = page.locator(".section-row").filter({
    has: page.getByRole("textbox", { name: "Name for Boats", exact: true }),
  });
  assert.equal(
    await namesRow
      .getByRole("textbox", { name: "Name for Boats", exact: true })
      .inputValue(),
    "Ships",
  );
  await namesRow
    .getByRole("button", { name: "Save name for Boats", exact: true })
    .click();
  await namesRow.getByRole("textbox").waitFor({ state: "hidden" });
  state = await invoke("snapshot");
  assert.equal(state.project.sectionNames["5"], "Ships");
  assert.equal(state.sections.find((x) => x.id === "5").name, "Ships");
  assert.equal(state.stage.sections.find((x) => x.id === "5").name, "Ships");
  assert.equal(state.nameSync.pending.length, 0);
  assert.equal(state.sections.find((x) => x.id === "5").changed, false);
  assert.notDeepEqual(
    fs.readFileSync(settingsFile),
    settingsBefore,
    "Canonical names must be applied automatically",
  );
  fs.writeFileSync(settingsFile, settingsBefore);
  await invoke("refresh_saved_changes");
  assert.equal(
    (await invoke("snapshot")).stage.sections.find((x) => x.id === "5").name,
    "Ships",
  );
  const namesHistory = await invoke("history_page", {
    filter: { section: "5" },
  });
  assert.equal(namesHistory.entries[0].summary, "Updated slice names");
  assert.equal(namesHistory.entries[0].revisions.length, 0);
  assert.equal(namesHistory.entries[0].nameChanges[0].after, "Ships");
  await page.screenshot({ path: "build/ui-proof/section-names.png" });
  // Another member sees the new names without receiving the settings file.
  assert.equal(
    (await api("", undefined, guest.token)).sectionNames["5"],
    "Ships",
  );
  await invoke("start_work", { section: "5", takeover: false });
  assert.match((await invoke("snapshot")).message, /Reserved Ships/);
  await invoke("share_section_names", {
    names: { 5: "Boats" },
    parentVersion: state.project.namesVersion,
  });
  console.log(
    "Native section names proof passed: recovered private draft, real React sharing, canonical labels, guest synchronization, immutable rename History, automatic editor-settings reapplication.",
  );
  assert(requests.length > 5);
  for (const request of requests) {
    if (request.body) {
      const body = JSON.parse(request.body);
      assert(
        !/"(?:bytes|refresh_token|access_token)"\s*:/.test(request.body),
        "File bytes/provider tokens reached coordinator",
      );
      assert.equal(typeof body, "object");
    }
  }
  console.log(
    "Native transfer proof passed: second creator, automatic download, missed-event reconciliation, checksum rejection/retry, project history, metadata-only HTTP requests.",
  );
  await invoke("finish_work");
  const finished = await invoke("snapshot");
  assert(
    finished.project.locks
      .filter((l) => l.member === member)
      .every((l) => finished.sections.find((s) => s.id === l.section)?.changed),
    "Finish must retain only slices with unshared changes",
  );
  await invoke("leave_project");
  assert.equal((await invoke("snapshot")).project, null);
  assert((await invoke("connections")).some((p) => p.id === project));
  await invoke("reopen_project", { projectId: project });
  state = await invoke("snapshot");
  assert.equal(state.member, member);
  assert(state.project.members.find((m) => m.id === member).owner);
  assert.deepEqual(state.draft, draft);
  assert.deepEqual(
    digest(),
    before,
    "Native smoke changed original game saves",
  );
  await page.screenshot({ path: "build/ui-proof/native-project.png" });
  console.log(
    "Native desktop smoke passed: isolated profile, real discovery/IPC, shared-folder project, historical selection, locks, real publication, draft, history, scoped installation, leave/reopen owner identity, original save hashes unchanged.",
  );
  await invoke("leave_project");
  await browser.close();
  proxy.closeAllConnections();
  await new Promise((resolve) => proxy.close(resolve));
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

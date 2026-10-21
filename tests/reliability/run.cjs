const assert = require("node:assert/strict"),
  fs = require("node:fs"),
  path = require("node:path"),
  net = require("node:net"),
  crypto = require("node:crypto");
const { spawn, execFileSync } = require("node:child_process");
const { chromium } = require("playwright");
const { FaultProxy } = require("./fault-proxy.cjs");
const testCoordinator = require("../coordinator.cjs");
const { sha, slice, settings, makeStage } = require("./fixtures.cjs");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const options = Object.fromEntries(
  process.argv.slice(2).map((a) => a.replace(/^--/, "").split("=")),
);
const binary = path.resolve(
  options.binary || ".tools/review2-target/debug/miliastra-collab.exe",
);
const cycles = Number(options.cycles || 12),
  seed = Number(options.seed || 1729);
assert(Number.isInteger(cycles) && cycles >= 1 && cycles <= 100);
const root = path.resolve(".tools", "reliability-" + crypto.randomUUID());
fs.mkdirSync(root, { recursive: true });
fs.writeFileSync(
  path.join(root, "reliability-run.json"),
  JSON.stringify({ seed, cycles }),
);
const bin = path.join(root, "bin");
fs.mkdirSync(bin);
const executable = path.join(bin, "miliastra-collab.exe");
fs.copyFileSync(binary, executable);
const results = [],
  clients = [],
  canaries = [
    "PrivateProjectCanary",
    "PrivateAuthorCanaryA",
    "PrivateAuthorCanaryB",
    "PrivateStageCanary",
    "BaselinePayloadCanary",
    "SharedBaselineCanary",
    "PrivateChangeCanary",
    "PrivateOAuthCanary",
  ];
let proxy, coordinator;
async function step(name, action) {
  const start = Date.now();
  console.log("RUN " + name);
  try {
    await action();
    results.push({ name, status: "passed", durationMs: Date.now() - start });
    console.log("PASS " + name);
  } catch (error) {
    results.push({
      name,
      status: "failed",
      durationMs: Date.now() - start,
      error: String(error.message).replaceAll(root, "<run>"),
    });
    throw error;
  } finally {
    fs.writeFileSync(
      path.join(root, "report.json"),
      JSON.stringify({ seed, cycles, scenarios: results }, null, 2),
    );
  }
}
async function freePort() {
  const s = net.createServer();
  await new Promise((r) => s.listen(0, "127.0.0.1", r));
  const n = s.address().port;
  await new Promise((r) => s.close(r));
  return n;
}
class Client {
  constructor(name) {
    this.name = name;
    this.root = path.join(root, name);
    fs.mkdirSync(this.root);
    const secrets = path.join(this.root, "test-secrets");
    fs.mkdirSync(secrets);
    fs.writeFileSync(
      path.join(secrets, sha(Buffer.from("oauth:onedrive"))),
      JSON.stringify({
        access_token: "PrivateOAuthCanary",
        refresh_token: "PrivateOAuthCanary-refresh",
        expires_at: 0,
      }),
    );
    this.stage = makeStage(this.root, name === "A" ? 501 : 502);
    clients.push(this);
  }
  async start() {
    if (!this.port) this.port = await freePort();
    if (!fs.existsSync(path.join(this.root, "coordination.json")))
      fs.writeFileSync(
        path.join(this.root, "coordination.json"),
        '{"serviceUrl":"http://127.0.0.1:9"}',
      );
    const env = {
      ...process.env,
      MCOLLAB_TEST_DATA_DIR: this.root,
      MCOLLAB_RELIABILITY_ROOT: root,
      MCOLLAB_RELIABILITY_BINARY: executable,
      WEBVIEW2_USER_DATA_FOLDER: path.join(this.root, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
        "--remote-debugging-port=" + this.port,
    };
    const pidFile = path.join(this.root, "pid.txt");
    fs.rmSync(pidFile, { force: true });
    const launch =
      '$p=Start-Process -FilePath $env:MCOLLAB_RELIABILITY_BINARY -ArgumentList "--headless-smoke" -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $env:MCOLLAB_TEST_DATA_DIR "stdout.log") -RedirectStandardError (Join-Path $env:MCOLLAB_TEST_DATA_DIR "stderr.log"); [IO.File]::WriteAllText((Join-Path $env:MCOLLAB_TEST_DATA_DIR "pid.txt"),[string]$p.Id)';
    const launcher = spawn(
      "powershell.exe",
      ["-NoProfile", "-Command", launch],
      { env, windowsHide: true, stdio: "ignore" },
    );
    for (let n = 0; n < 150 && !fs.existsSync(pidFile); n++) {
      if (launcher.exitCode !== null && launcher.exitCode !== 0)
        throw Error("Synthetic launcher failed " + launcher.exitCode);
      await sleep(100);
    }
    assert(fs.existsSync(pidFile), "Synthetic launcher did not report its PID");
    this.pid = Number(fs.readFileSync(pidFile, "utf8"));
    assert(this.pid > 0);
    for (let n = 0; n < 300; n++) {
      try {
        await fetch("http://127.0.0.1:" + this.port + "/json/version").then(
          (r) => r.json(),
        );
        break;
      } catch {
        await sleep(100);
      }
    }
    this.browser = await chromium.connectOverCDP(
      "http://127.0.0.1:" + this.port,
    );
    this.page = this.browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => !p.url().includes("overlay=1"));
    this.page.setDefaultTimeout(15000);
    await this.page.locator(".brand").waitFor();
    assert.equal(
      path.resolve((await this.call("snapshot")).testDataDirectory),
      this.root,
      "Test data escaped isolated root",
    );
  }
  call(cmd, args = {}) {
    return this.page.evaluate(
      ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
      { cmd, args },
    );
  }
  async settings(origin) {
    const state = await this.call("snapshot");
    await this.call("save_settings", {
      settings: {
        ...state.settings,
        endpoint: origin,
        saveRoot: path.join(this.root, "saves"),
        autoApply: false,
      },
    });
  }
  async stop() {
    if (this.pid) {
      const env = {
        ...process.env,
        MCOLLAB_RELIABILITY_PID: String(this.pid),
        MCOLLAB_RELIABILITY_BINARY: executable,
      };
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-Command",
          "$p=Get-Process -Id $env:MCOLLAB_RELIABILITY_PID -ErrorAction SilentlyContinue; if($p -and $p.Path -eq $env:MCOLLAB_RELIABILITY_BINARY){Stop-Process -Id $p.Id}",
        ],
        { env, windowsHide: true, stdio: "ignore", timeout: 15000 },
      );
      this.pid = null;
    }
    if (this.browser) {
      await this.browser.close().catch(() => {});
      this.browser = null;
    }
  }
  async fault(point, reference, action) {
    const id = crypto.randomUUID();
    fs.writeFileSync(
      path.join(this.root, "fault.json"),
      JSON.stringify({ id, point, reference, action }),
    );
    return id;
  }
  async hit(id) {
    for (let n = 0; n < 180; n++) {
      if (fs.existsSync(path.join(this.root, "hit-" + id + ".json"))) return;
      await sleep(100);
    }
    throw Error("Synthetic checkpoint not reached");
  }
  clearFault() {
    fs.rmSync(path.join(this.root, "fault.json"), { force: true });
  }
  file(section) {
    return path.join(
      this.stage,
      "beyond_level_slice",
      path.basename(this.stage) + "_" + (Number(section) + 1) + ".gis",
    );
  }
  async edit(section, label) {
    await this.call("start_work", {
      section: String(section),
      takeover: false,
    });
    fs.writeFileSync(this.file(section), slice(Number(section), label));
    await this.call("refresh_saved_changes");
  }
  async share(sections, summary) {
    return this.call("share_update", {
      sections: sections.map(String),
      draft: { summary, details: "Synthetic reliability rehearsal" },
      keepReserved: false,
    });
  }
  async refresh() {
    await this.call("refresh");
    return this.call("snapshot");
  }
  async wait(predicate, timeoutMs = 24000) {
    for (let n = 0; n < Math.ceil(timeoutMs / 100); n++) {
      const s = await this.call("snapshot");
      if (predicate(s)) return s;
      await sleep(100);
    }
    throw Error("Client " + this.name + " convergence timed out");
  }
  async apply() {
    await this.refresh();
    let s = await this.wait(
      (s) =>
        !s.metadataPending &&
        s.sections.every((x) => x.installed || x.downloaded),
    );
    if (s.sections.every((x) => x.installed)) return;
    let plan;
    try {
      plan = await this.call("preview_install");
    } catch (error) {
      assert.match(
        String(error.message),
        /There are no new updates to install/,
      );
      assert(
        (await this.call("snapshot")).sections.every((x) => x.installed),
        "Preview raced with incomplete reconciliation",
      );
      return;
    }
    if (plan.revisions.length) {
      await this.call("install_updates", {
        planId: plan.id,
        editorReady: true,
      });
      assert.equal(
        (await this.call("snapshot")).installation.phase,
        "verified",
      );
    }
  }
}
async function history(client) {
  let before = null,
    entries = [];
  do {
    const page = await client.call("history_page", {
      filter: before ? { before } : {},
    });
    entries.push(...page.entries);
    before = page.next;
  } while (before);
  return entries;
}
(async () => {
  const a = new Client("A"),
    b = new Client("B");
  let project, invitation, sequence;
  try {
    await step("isolated native clients and the real coordinator", async () => {
      coordinator = await testCoordinator.start({ port: 0 });
      proxy = await new FaultProxy(coordinator.url).start();
      await a.start();
      await b.start();
      await a.settings(proxy.origin);
      await b.settings(proxy.origin);
      assert.notEqual(a.root, b.root);
      const exchange = path.join(root, "exchange");
      fs.mkdirSync(exchange);
      await a.call("create_project", {
        name: "PrivateProjectCanary",
        displayName: "PrivateAuthorCanaryA",
        stagePath: a.stage,
        kind: "folder",
        folder: exchange,
      });
      let s = await a.refresh();
      project = s.project.id;
      sequence = s.project.sequence;
      assert.equal(sequence, 1);
      const url = new URL(await a.call("invite"));
      invitation = {
        version: 1,
        endpoint: url.origin,
        project,
        invite: url.hash.slice(1),
      };
      const key = await a.call("folder_privacy_key");
      canaries.push(key);
      await b.call("save_folder_privacy_key", { invitation, key });
      const failedJoin = await b.fault(
        "storage-read",
        "History/*/manifest.json",
        "fail",
      );
      await assert.rejects(
        b.call("join_project", {
          invitation,
          displayName: "PrivateAuthorCanaryB",
          stagePath: b.stage,
          folder: exchange,
        }),
      );
      await b.hit(failedJoin);
      b.clearFault();
      await b.call("join_project", {
        invitation,
        displayName: "PrivateAuthorCanaryB",
        stagePath: b.stage,
        folder: exchange,
      });
      assert.equal(
        (await a.refresh()).project.members.length,
        2,
        "Retrying setup created a duplicate membership",
      );
      await b.apply();
      assert.equal((await b.call("snapshot")).project.id, project);
      assert.notEqual(path.basename(a.stage), path.basename(b.stage));
    });
    await step(
      "saved selection reserves only after fresh settings and never publishes ordinary saves",
      async () => {
        for (const client of [a, b])
          fs.writeFileSync(path.join(client.root, "editor-running.json"), "{}");
        await a.call("refresh_saved_changes");
        let state = await a.refresh();
        assert(
          !state.project.locks.some((l) => l.section === "63"),
          "Historical startup selection acquired a lock",
        );
        const unsaved = settings(Number(path.basename(a.stage)), 54, 1); // In memory only: the app has no live UI observation.
        await a.call("refresh_saved_changes");
        state = await a.refresh();
        assert(!state.project.locks.some((l) => l.section === "54"));
        fs.writeFileSync(
          path.join(a.stage, "beyond_level_setting/local_setting.gils"),
          unsaved,
        );
        await a.call("refresh_saved_changes");
        state = await a.wait((s) =>
          s.project.locks.some(
            (l) => l.section === "54" && l.member === s.member,
          ),
        );
        const fence = state.project.locks.find((l) => l.section === "54").fence;
        fs.writeFileSync(
          path.join(a.stage, "beyond_level_setting/local_setting.gils"),
          settings(Number(path.basename(a.stage)), 54, 2),
        );
        await a.call("refresh_saved_changes");
        assert.equal(
          (await a.refresh()).project.locks.find((l) => l.section === "54")
            .fence,
          fence,
          "Saving an owned slice replaced its lease",
        );
        fs.writeFileSync(
          path.join(b.stage, "beyond_level_setting/local_setting.gils"),
          settings(Number(path.basename(b.stage)), 54, 1),
        );
        await b.call("refresh_saved_changes");
        assert.equal(
          (await b.refresh()).project.locks.find((l) => l.section === "54")
            .member,
          state.member,
          "Conflicting saved selection stole ownership",
        );
        assert.equal(
          (await a.refresh()).project.sequence,
          sequence,
          "Ordinary saves published an update",
        );
        await a.call("finish_work", { section: "54" });
        for (const client of [a, b])
          fs.rmSync(path.join(client.root, "editor-running.json"));
      },
    );
    await step(
      "simultaneous reservations, unchanged release and mandatory changelog",
      async () => {
        const acquired = await Promise.allSettled([
          a.call("start_work", { section: "5", takeover: false }),
          b.call("start_work", { section: "5", takeover: false }),
        ]);
        assert.equal(
          acquired.filter((r) => r.status === "fulfilled").length,
          1,
        );
        const s = await a.refresh(),
          owner = s.project.locks.find((l) => l.section === "5").member;
        const winner = s.member === owner ? a : b;
        await winner.call("finish_work", { section: "5" });
        await assert.rejects(a.share([5], "   "), /changed|sharing|teammates/i);
        assert.equal((await a.refresh()).project.sequence, sequence);
      },
    );
    await step(
      "two creators publish different slices and converge",
      async () => {
        await a.edit(5, "PrivateChangeCanary-A1");
        await b.edit(6, "PrivateChangeCanary-B1");
        const [ua, ub] = await Promise.all([
          a.share([5], "PrivateChangeCanary alpha"),
          b.share([6], "PrivateChangeCanary beta"),
        ]);
        assert.notEqual(ua.id, ub.id);
        sequence += 2;
        await a.apply();
        await b.apply();
        assert.equal(
          sha(fs.readFileSync(a.file(5))),
          sha(fs.readFileSync(b.file(5))),
        );
        assert.equal(
          sha(fs.readFileSync(a.file(6))),
          sha(fs.readFileSync(b.file(6))),
        );
      },
    );
    await step(
      "healthy idle sockets do not poll project state or cached History",
      async () => {
        await a.call("finish_work");
        await b.call("finish_work");
        await Promise.all([a.refresh(), b.refresh()]);
        for (let n = 0; n < 150 && proxy.bridges.size < 2; n++)
          await sleep(100);
        assert(
          proxy.bridges.size >= 2,
          "Both live connections must be established",
        );
        // Drain join/presence notifications before measuring a full former polling interval.
        await sleep(1000);
        const start = proxy.records.length;
        await sleep(35000);
        const repeated = proxy.records
          .slice(start)
          .filter(
            (r) =>
              r.method === "GET" &&
              /^\/projects\/[^/]+(?:\/history(?:\?.*)?)?$/.test(r.path),
          );
        assert.deepEqual(
          repeated.map((r) => r.path),
          [],
          "Idle clients downloaded unchanged project/history metadata",
        );
        assert(
          proxy.bridges.size >= 2,
          "Keep-alive must preserve both sockets",
        );
      },
    );
    await step(
      "missed events reconcile automatically without a live socket",
      async () => {
        proxy.setEvents(false);
        await a.edit(7, "PrivateChangeCanary missed event");
        await a.share([7], "PrivateChangeCanary missed event");
        sequence++;
        // Do not invoke manual refresh until the disconnected fallback has caught up.
        await b.wait(
          (s) => s.project.sequence === sequence && !s.metadataPending,
          45000,
        );
        await b.apply();
        proxy.setEvents(true);
        assert.equal((await b.refresh()).project.sequence, sequence);
      },
    );
    await step("coordinator outage preserves work and changelog", async () => {
      await a.edit(8, "PrivateChangeCanary offline");
      proxy.setOffline(true);
      await assert.rejects(a.share([8], "PrivateChangeCanary offline draft"));
      let s = await a.call("snapshot");
      assert.equal(s.draft.summary, "PrivateChangeCanary offline draft");
      assert.equal(
        sha(fs.readFileSync(a.file(8))),
        sha(slice(8, "PrivateChangeCanary offline")),
      );
      proxy.setOffline(false);
      await a.refresh();
      await a.share([8], "PrivateChangeCanary offline draft");
      sequence++;
      await b.apply();
    });
    await step(
      "lost publication reply reconciles one immutable update",
      async () => {
        await a.edit(9, "PrivateChangeCanary lost reply");
        proxy.drop = "/publish";
        await assert.rejects(a.share([9], "PrivateChangeCanary lost reply"));
        sequence++;
        await a.refresh();
        await b.apply();
        const entries = await history(a);
        assert.equal(
          entries.filter((u) => u.summary === "PrivateChangeCanary lost reply")
            .length,
          1,
        );
        assert.equal((await a.call("snapshot")).draft.summary, "");
        assert.equal((await a.call("snapshot")).project.sequence, sequence);
      },
    );
    await step(
      "interrupted multi-slice upload retries without partial publication",
      async () => {
        await a.edit(10, "PrivateChangeCanary upload 1");
        await a.edit(11, "PrivateChangeCanary upload 2");
        const hit = await a.fault("storage-write", "History/*/12.gis", "pause");
        const pending = a
          .share([10, 11], "PrivateChangeCanary interrupted upload")
          .catch(() => null);
        await a.hit(hit);
        assert.equal((await b.refresh()).project.sequence, sequence);
        await a.stop();
        await pending;
        a.clearFault();
        await a.start();
        await a.refresh();
        assert.equal(
          (await a.call("snapshot")).draft.summary,
          "PrivateChangeCanary interrupted upload",
        );
        await a.share([10, 11], "PrivateChangeCanary interrupted upload");
        sequence++;
        await b.apply();
        assert.equal(
          (await history(a)).filter(
            (u) => u.summary === "PrivateChangeCanary interrupted upload",
          ).length,
          1,
        );
      },
    );
    await step(
      "partial/corrupt downloads never become installable and retry",
      async () => {
        proxy.setEvents(false);
        await a.edit(12, "PrivateChangeCanary download");
        const update = await a.share([12], "PrivateChangeCanary download");
        sequence++;
        const before = sha(fs.readFileSync(b.file(12)));
        const hit = await b.fault(
          "storage-read",
          "History/*/13.gis",
          "corrupt",
        );
        await b.refresh();
        await b.hit(hit);
        assert.equal(sha(fs.readFileSync(b.file(12))), before);
        const cached = path.join(b.root, "objects", update.revisions[0].hash);
        if (fs.existsSync(cached))
          assert.equal(
            sha(fs.readFileSync(cached)),
            update.revisions[0].hash,
            "Corrupt bytes entered verified cache",
          );
        b.clearFault();
        await b.apply();
        proxy.setEvents(true);
        assert.equal(
          sha(fs.readFileSync(b.file(12))),
          update.revisions[0].hash,
        );
      },
    );
    await step(
      "storage permission failure preserves draft and immutable history",
      async () => {
        await a.edit(13, "PrivateChangeCanary access");
        const hit = await a.fault("storage-write", "History/*/14.gis", "fail");
        await assert.rejects(
          a.share([13], "PrivateChangeCanary access denied"),
          /failure|access/i,
        );
        await a.hit(hit);
        a.clearFault();
        assert.equal((await b.refresh()).project.sequence, sequence);
        assert.equal(
          (await a.call("snapshot")).draft.summary,
          "PrivateChangeCanary access denied",
        );
        await a.share([13], "PrivateChangeCanary access denied");
        sequence++;
        await b.apply();
      },
    );
    await step(
      "dirty work and a changed installation preview block replacement",
      async () => {
        await b.edit(14, "PrivateChangeCanary local unsent");
        proxy.setEvents(false);
        await a.edit(15, "PrivateChangeCanary incoming");
        await a.share([15], "PrivateChangeCanary incoming");
        sequence++;
        await b.refresh();
        await b.wait(
          (s) =>
            !s.metadataPending &&
            !s.sections.find((x) => x.id === "15").installed &&
            s.sections.find((x) => x.id === "15").downloaded,
        );
        const plan = await b.call("preview_install");
        const before = fs.readFileSync(b.file(15));
        fs.writeFileSync(
          b.file(15),
          slice(15, "PrivateChangeCanary changed after preview"),
        );
        await assert.rejects(
          b.call("install_updates", { planId: plan.id, editorReady: true }),
        );
        assert.equal(
          sha(fs.readFileSync(b.file(14))),
          sha(slice(14, "PrivateChangeCanary local unsent")),
        );
        fs.writeFileSync(b.file(15), before);
        await b.call("refresh_saved_changes");
        await b.share([14], "PrivateChangeCanary preserve local");
        sequence++;
        await a.apply();
        await b.apply();
        proxy.setEvents(true);
      },
    );
    await step(
      "interrupted installation restores complete original bytes after restart",
      async () => {
        await a.edit(16, "PrivateChangeCanary install1");
        await a.edit(17, "PrivateChangeCanary install2");
        await a.share([16, 17], "PrivateChangeCanary interrupted installation");
        sequence++;
        await b.refresh();
        await b.wait(
          (s) =>
            !s.metadataPending &&
            ["16", "17"].every(
              (id) =>
                !s.sections.find((x) => x.id === id).installed &&
                s.sections.find((x) => x.id === id).downloaded,
            ),
        );
        const originals = [16, 17].map((i) => sha(fs.readFileSync(b.file(i)))),
          plan = await b.call("preview_install");
        const hit = await b.fault(
          "install-file",
          path.basename(b.file(16)),
          "pause",
        );
        const pending = b
          .call("install_updates", { planId: plan.id, editorReady: true })
          .catch(() => null);
        await b.hit(hit);
        await b.stop();
        await pending;
        b.clearFault();
        await b.start();
        await b.call("undo_install", { editorReady: true });
        assert.equal((await b.call("snapshot")).installation.phase, "undone");

        assert.deepEqual(
          [16, 17].map((i) => sha(fs.readFileSync(b.file(i)))),
          originals,
        );
        await b.apply();
      },
    );
    await step(
      "real Share dialog retains drafts, shows errors and progress, and retries once",
      async () => {
        await a.edit(42, "PrivateChangeCanary UI");
        await a.refresh();
        await a.page.getByRole("button", { name: /^Share update \(/ }).click();
        let dialog = a.page.getByRole("dialog", {
          name: "Share update",
          exact: true,
        });
        const input = dialog.getByPlaceholder(
          "Fixed passengers falling through the boat deck",
        );
        await input.fill("PrivateChangeCanary UI retry");
        let submit = dialog.getByRole("button", {
          name: "Share update",
          exact: true,
        });
        await a.page.waitForFunction(() => {
          const d = document.querySelector("[role=dialog]");
          return d && !d.querySelector("[role=status]");
        });
        const failed = await a.fault(
          "storage-write",
          "History/*/43.gis",
          "fail",
        );
        await submit.click();
        await a.hit(failed);
        await dialog.getByRole("alert").waitFor();
        assert.match(
          await dialog.getByRole("alert").innerText(),
          /storage access failure/i,
        );
        assert.equal(await input.inputValue(), "PrivateChangeCanary UI retry");
        assert.equal(await submit.isEnabled(), true);
        a.clearFault();
        await dialog
          .getByRole("button", { name: "Cancel", exact: true })
          .click();
        await dialog.waitFor({ state: "detached" });
        await a.page.getByRole("button", { name: /^Share update \(/ }).click();
        dialog = a.page.getByRole("dialog", {
          name: "Share update",
          exact: true,
        });
        assert.equal(
          await dialog
            .getByPlaceholder("Fixed passengers falling through the boat deck")
            .inputValue(),
          "PrivateChangeCanary UI retry",
        );
        await a.page.waitForFunction(() => {
          const d = document.querySelector("[role=dialog]");
          return d && !d.querySelector("[role=status]");
        });
        const paused = await a.fault(
          "storage-write",
          "History/*/43.gis",
          "pause",
        );
        submit = dialog.getByRole("button", {
          name: "Share update",
          exact: true,
        });
        await submit.click();
        await a.hit(paused);
        await dialog.getByRole("status").waitFor();
        assert.equal(await submit.isDisabled(), true);
        assert.equal(
          await dialog
            .getByRole("button", { name: "Close", exact: true })
            .isDisabled(),
          true,
        );
        const started = Date.now();
        await a.call("start_work", { section: "46", takeover: false });
        assert(
          Date.now() - started < 2000,
          "Share upload blocked reserving an unrelated slice",
        );
        fs.writeFileSync(
          a.file(47),
          slice(47, "PrivateChangeCanary save during upload"),
        );
        const newer = slice(
          42,
          "PrivateChangeCanary newer version during upload",
        );
        fs.writeFileSync(a.file(42), newer);
        const saved = Date.now();
        await a.call("refresh_saved_changes");
        assert(
          Date.now() - saved < 2000,
          "Share upload blocked saved-section detection",
        );
        assert(
          (await a.call("snapshot")).sections.find((s) => s.id === "47")
            .changed,
          "Saved work during upload was lost",
        );
        await a.call("finish_work", { section: "46" });
        a.clearFault();
        await dialog.waitFor({ state: "detached" });
        sequence++;
        await b.apply();
        const after = await a.call("snapshot");
        assert.equal(
          sha(fs.readFileSync(a.file(42))),
          sha(newer),
          "Publication replaced a newer editor save",
        );
        const retained = after.sections.find((s) => s.id === "42");
        assert(
          retained.changed && retained.lock?.member === after.member,
          "Newer unshared edits lost their reservation",
        );
        assert.equal(
          (await history(a)).filter(
            (u) => u.summary === "PrivateChangeCanary UI retry",
          ).length,
          1,
        );
      },
    );
    await step(
      "coordinator restart retains membership, history and ordering",
      async () => {
        await coordinator.restart();
        await a.refresh();
        await b.refresh();
        assert.equal((await a.call("snapshot")).project.sequence, sequence);
        assert.equal((await history(b)).length, sequence);
      },
    );
    await step(
      "slice names and restore history remain in team storage",
      async () => {
        const names = (await a.refresh()).project.sectionNames;
        await a.call("share_section_names", {
          names: { ...names, 5: "PrivateSliceCanary Boats" },
          parentVersion: (await a.call("snapshot")).project.namesVersion,
        });
        sequence++;
        await b.refresh();
        await b.wait(
          (s) =>
            !s.metadataPending &&
            s.project.sectionNames["5"] === "PrivateSliceCanary Boats",
        );
        canaries.push("PrivateSliceCanary");
        const baseline = (await history(a)).find((u) => u.sequence === 1),
          oldRevision = baseline.revisions.find((r) => r.section === "5");
        const restored = await a.call("restore_version", {
          updateId: baseline.id,
          section: "5",
          reason: "PrivateChangeCanary restore old boat",
        });
        sequence++;
        assert.equal(restored.restores, baseline.id);
        assert.equal(restored.revisions[0].hash, oldRevision.hash);
        await a.apply();
        await b.apply();
        const baseReceipt = JSON.parse(
            fs.readFileSync(
              path.join(
                root,
                "exchange",
                "History",
                baseline.id,
                "receipt.json",
              ),
            ),
          ),
          restoredReceipt = JSON.parse(
            fs.readFileSync(
              path.join(
                root,
                "exchange",
                "History",
                restored.id,
                "receipt.json",
              ),
            ),
          );
        assert.notEqual(
          baseReceipt.revisions.find(
            (r) => r.section === restoredReceipt.revisions[0].section,
          ).revision,
          restoredReceipt.revisions[0].revision,
          "Restoring identical bytes reused a revision ID",
        );
      },
    );
    await step("seeded repeated two-client editing converges", async () => {
      let state = seed >>> 0;
      for (let i = 0; i < cycles; i++) {
        state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
        const section = 20 + (state % 20),
          actor = i % 2 ? a : b,
          other = actor === a ? b : a;
        await actor.edit(section, "PrivateChangeCanary cycle " + i);
        await actor.share([section], "PrivateChangeCanary cycle " + i);
        sequence++;
        if (i % 3 === 0) proxy.setEvents(false);
        await other.apply();
        proxy.setEvents(true);
        assert.equal(
          sha(fs.readFileSync(a.file(section))),
          sha(fs.readFileSync(b.file(section))),
        );
        console.log("Cycle " + (i + 1) + "/" + cycles + " converged");
      }
      assert.equal((await a.refresh()).project.sequence, sequence);
      assert.equal((await b.refresh()).project.sequence, sequence);
      const entries = await history(a);
      assert.equal(new Set(entries.map((u) => u.id)).size, sequence);
    });
    if (options["skip-lease-expiry"] !== "true")
      await step(
        "expired real lease rejects stale renewal fencing token",
        async () => {
          await a.call("start_work", { section: "50", takeover: false });
          let s = await a.call("snapshot");
          const lease = s.project.locks.find((l) => l.section === "50");
          const request = [...proxy.records]
            .reverse()
            .find(
              (r) =>
                r.path.endsWith("/locks/acquire") &&
                r.authorization &&
                JSON.parse(r.body).takeover === false,
            );
          assert(request);
          const body = JSON.parse(request.body);
          await a.stop();
          const deadline = lease.expires * 1000 + 1500;
          let last = 0;
          while (Date.now() < deadline) {
            await sleep(Math.min(1000, deadline - Date.now()));
            if (Date.now() - last > 25000) {
              console.log("Waiting for real lease expiry");
              last = Date.now();
            }
          }
          await b.call("start_work", { section: "50", takeover: false });
          const response = await fetch(
            proxy.origin + "/projects/" + project + "/locks/renew",
            {
              method: "POST",
              headers: {
                "content-type": "application/json",
                authorization: request.authorization,
              },
              body: JSON.stringify({
                section: body.section,
                fence: lease.fence,
              }),
            },
          );
          assert.equal(response.status, 409);
          await b.call("finish_work", { section: "50" });
          await a.start();
          await a.refresh();
        },
      );
    else
      results.push({
        name: "expired real lease rejects stale renewal fencing token",
        status: "skipped",
        reason: "Explicit quick run",
      });
    await step(
      "unavailable or altered manifests preserve History and block installation",
      async () => {
        proxy.setEvents(false);
        await a.edit(43, "PrivateChangeCanary manifest");
        const pause = await a.fault("control-finalize", "/publish", "pause"),
          sharing = a.share([43], "PrivateChangeCanary unavailable details");
        await a.hit(pause);
        const published = new Set((await history(a)).map((u) => u.id));
        const candidate = fs
          .readdirSync(path.join(root, "exchange", "History"))
          .find(
            (id) =>
              !published.has(id) &&
              fs.existsSync(
                path.join(root, "exchange", "History", id, "manifest.json"),
              ) &&
              JSON.parse(
                fs.readFileSync(
                  path.join(root, "exchange", "History", id, "manifest.json"),
                ),
              ).update.summary === "PrivateChangeCanary unavailable details",
          );
        assert(candidate, "Missing staged update manifest");
        const manifest = path.join(
            root,
            "exchange",
            "History",
            candidate,
            "manifest.json",
          ),
          held = manifest + ".held";
        fs.renameSync(manifest, held);
        a.clearFault();
        const u = await sharing;
        sequence++;
        const previous = (await history(b)).length;
        await b.refresh();
        assert.equal((await b.call("snapshot")).metadataPending, true);
        assert(
          (await history(b)).length >= previous,
          "Unavailable storage erased cached History",
        );
        await assert.rejects(
          b.call("preview_install"),
          /Waiting for update details/,
        );
        await b.call("start_work", { section: "44", takeover: false });
        await b.call("finish_work", { section: "44" });
        fs.writeFileSync(manifest, Buffer.from("altered manifest"));
        await b.refresh();
        assert.equal((await b.call("snapshot")).metadataPending, true);
        await assert.rejects(
          b.call("preview_install"),
          /Waiting for update details/,
        );
        fs.rmSync(manifest);
        fs.renameSync(held, manifest);
        const delayed = await b.fault(
          "storage-read",
          "History/*/manifest.json",
          "pause",
        );
        await b.refresh();
        await b.hit(delayed);
        const started = Date.now();
        await b.call("start_work", { section: "45", takeover: false });
        assert(
          Date.now() - started < 2000,
          "Provider transfer delayed a reservation",
        );
        await b.call("finish_work", { section: "45" });
        b.clearFault();
        await b.wait((s) => !s.metadataPending);
        await b.apply();
        proxy.setEvents(true);
        assert.equal(sha(fs.readFileSync(b.file(43))), u.revisions[0].hash);
      },
    );
    await step(
      "wire and coordinator storage exclude private canaries and level bytes",
      async () => {
        const audit = proxy.audit(canaries);
        for (const request of proxy.records.filter(
          (r) =>
            r.status >= 200 &&
            r.status < 300 &&
            r.method === "POST" &&
            (r.path === "/projects" ||
              r.path.endsWith("/publish") ||
              r.path.endsWith("/names")),
        )) {
          const value = JSON.parse(request.body);
          for (const forbidden of [
            "summary",
            "details",
            "name",
            "displayName",
            "storage",
            "artifact",
            "size",
            "contentHash",
          ])
            assert(
              !Object.hasOwn(value, forbidden),
              "Description reached coordinator: " + forbidden,
            );
          for (const revision of value.revisions || value.baseline || []) {
            assert.deepEqual(
              Object.keys(revision).sort(),
              ["parent", "revision", "section", "sharedDependency"].sort(),
            );
            assert.match(revision.revision, /^[a-f0-9-]{36}$/);
          }
        }

        const rows = coordinator.query(
          "SELECT state::text FROM projects UNION ALL SELECT document::text FROM updates",
        );
        for (const document of rows) {
          const value = JSON.parse(document);
          for (const key of [
            "name",
            "storage",
            "section_names",
            "summary",
            "details",
            "author",
            "nameChanges",
          ])
            assert(
              !Object.hasOwn(value, key),
              "Coordinator retained descriptive field " + key,
            );
        }
        const stored = coordinator.dump();
        for (const value of canaries)
          assert(
            !stored.includes(value),
            "Readable metadata in coordinator database",
          );
        assert.equal(
          Number(coordinator.query("SELECT count(*) FROM updates")[0]),
          sequence,
        );
        fs.writeFileSync(
          path.join(root, "privacy-audit.json"),
          JSON.stringify(
            {
              ...audit,
              coordinatorRows: sequence,
              privateCanariesAbsentFromDatabase: true,
            },
            null,
            2,
          ),
        );
      },
    );
    const report = {
      seed,
      cycles,
      status: "passed",
      scenarios: results,
      limits: [
        "Synthetic framed stages do not prove real-editor import/save compatibility",
        "Storage faults simulate access revocation; no live provider account used",
        "Confirmation simulates an editor reopen; no game input was sent",
      ],
    };
    fs.writeFileSync(
      path.join(root, "report.json"),
      JSON.stringify(report, null, 2),
    );
    console.log("REPORT " + path.join(root, "report.json"));
  } catch (error) {
    console.error(error.message);
    console.error("REPORT " + path.join(root, "report.json"));
    process.exitCode = 1;
  } finally {
    for (const client of clients.reverse()) await client.stop().catch(() => {});
    if (proxy) await proxy.close();
    if (coordinator) await coordinator.stop();
  }
})();

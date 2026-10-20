// Encryption checks with the real app on Windows. No real saves or storage accounts.
const { chromium } = require("playwright"),
  assert = require("node:assert/strict"),
  fs = require("node:fs"),
  path = require("node:path"),
  crypto = require("node:crypto"),
  { execFileSync } = require("node:child_process");
const { PrivateWire } = require("./private-wire.cjs");
(async () => {
  const root = path.resolve(process.env.MCOLLAB_TEST_DATA_DIR || "");
  assert(path.basename(root).startsWith("privacy-native-"));
  assert.equal(path.basename(path.dirname(root)), ".tools");
  const browser = await chromium.connectOverCDP("http://127.0.0.1:19237");
  try {
    const page = browser
      .contexts()
      .flatMap((c) => c.pages())
      .find((p) => !p.url().includes("overlay=1"));
    assert(page);
    page.setDefaultTimeout(20000);
    await page
      .getByRole("link", { name: "miliastra/collab", exact: true })
      .waitFor();
    const invoke = (cmd, args = {}) =>
      page.evaluate(
        ({ cmd, args }) => window.__TAURI_INTERNALS__.invoke(cmd, args),
        { cmd, args },
      );
    const poll = async (f) => {
      for (let n = 0; n < 180; n++) {
        const s = await invoke("snapshot");
        if (f(s)) return s;
        await new Promise((r) => setTimeout(r, 250));
      }
      throw Error("Synthetic workspace timed out");
    };
    assert.equal(
      path.resolve((await invoke("snapshot")).testDataDirectory),
      root,
    );
    await invoke("begin_practice");
    let state = await poll(
      (s) =>
        s.practice &&
        !s.metadataPending &&
        s.project.sequence === 80 &&
        s.sections.every((x) => x.installed || x.downloaded),
    );
    assert.equal(state.sections.length, 65);
    assert.equal(state.project.sectionNames["5"], "Boats");
    const link = new URL(await invoke("invite"));
    assert.equal(link.hostname, "127.0.0.1");
    const bundle = await invoke("folder_privacy_key"),
      codec = new PrivateWire(bundle, link.origin, state.project.id);
    const request = async (route, body, token) => {
      const r = await fetch(link.origin + route, {
        method: body === undefined ? "GET" : "POST",
        headers: {
          "content-type": "application/json",
          ...(token ? { authorization: "Bearer " + token } : {}),
        },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
      const text = await r.text();
      let value;
      try {
        value = JSON.parse(text);
      } catch {
        value = { error: text };
      }
      return { status: r.status, value };
    };
    const base = "/projects/" + state.project.id;
    const j = await request(
      base + "/join",
      codec.command("/join", {
        invite: link.hash.slice(1),
        displayName: "Private peer proof",
      }),
    );
    assert.equal(j.status, 200);
    const peer = codec.openValue(j.value);
    let wire = await request(base, undefined, peer.token);
    assert.equal(wire.status, 200);
    const serialized = JSON.stringify(wire.value);
    for (const text of [
      "Fisch",
      "Boats",
      "Quests",
      "Private peer proof",
      state.project.heads["5"].hash,
      state.project.heads["5"].artifact,
      state.project.storage.location,
      bundle,
    ])
      if (text)
        assert(
          !serialized.includes(text),
          "Readable metadata leaked to service",
        );
    assert(
      !Object.hasOwn(wire.value, "storage") &&
        !Object.hasOwn(wire.value, "name"),
    );
    assert(
      Object.values(wire.value.heads).every(
        (r) =>
          !Object.hasOwn(r, "artifact") && /^[a-f0-9-]{36}$/.test(r.revision),
      ),
    );
    assert.equal(codec.openValue(wire.value).sectionNames["5"], "Boats");
    const bad = await request(
      base + "/history?q=private-search-proof",
      undefined,
      peer.token,
    );
    assert.equal(bad.status, 400, "Service must reject private search filters");
    let result = await invoke("history_page", { filter: { section: "6" } });
    assert(result.entries.length > 0);
    assert(
      result.entries.every(
        (u) =>
          u.revisions.some((r) => r.section === "6") ||
          u.nameChanges.some((c) => c.section === "6"),
      ),
    );
    const first = await invoke("history_page", { filter: {} });
    assert.equal(first.entries.length, 50);
    assert(first.next);
    const next = await invoke("history_page", {
      filter: { before: first.next },
    });
    assert.equal(next.entries.length, 30);
    assert(
      first.entries.every((a) => next.entries.every((b) => a.id !== b.id)),
    );
    // The invitation shows nothing until the right key is entered.
    const other = PrivateWire.generate(link.origin),
      raw = {
        name: "Separate key project",
        displayName: "Separate owner",
        storage: { kind: "folder", location: "creator-private-folder" },
        baseline: [],
      };
    const created = await request("/projects", other.create(raw));
    assert.equal(created.status, 200);
    const joined = other.openValue(created.value),
      invitation = joined.invitation;
    let details = await invoke("invitation_details", { invitation });
    assert.equal(details.privacyKeyRequired, true);
    assert.equal(details.storageKind, "locked");
    assert.equal(details.projectName, "Encrypted project");
    await invoke("save_folder_privacy_key", {
      invitation,
      key: PrivateWire.generate(link.origin).export(),
    });
    details = await invoke("invitation_details", { invitation });
    assert.equal(details.privacyKeyRequired, true);
    await invoke("save_folder_privacy_key", {
      invitation,
      key: other.export(),
    });
    details = await invoke("invitation_details", { invitation });
    assert.equal(details.projectName, "Shared project");
    assert.equal(details.storageKind, "folder");
    assert.equal(details.privacyKeyRequired, false);
    assert.equal((await invoke("snapshot")).project.id, state.project.id);
    await invoke("refresh");
    state = await poll(
      (s) =>
        !s.metadataPending &&
        s.project.members.some((m) => m.name === "Private peer proof"),
    );
    await invoke("share_section_names", {
      names: { ...state.project.sectionNames, 5: "Harbour" },
      parentVersion: 0,
    });
    state = await poll(
      (s) =>
        !s.metadataPending &&
        s.project.sequence === 81 &&
        s.project.sectionNames["5"] === "Harbour",
    );
    const updated = await invoke("history_page", { filter: { section: "5" } });
    assert.equal(updated.entries[0].sequence, 81);
    assert.equal(updated.entries[0].nameChanges[0].after, "Harbour");
    await invoke("refresh");
    const old = await invoke("history_page", { filter: { before: 2 } });
    assert.equal(old.entries[0].sequence, 1);
    assert(old.entries[0].summary);
    await invoke("save_folder_privacy_key", { key: bundle });
    {
      const current = (k) =>
        JSON.parse(Buffer.from(k.slice("mcollab-key-v2:".length), "base64url"))
          .current;
      assert.equal(
        current(await invoke("folder_privacy_key")),
        current(bundle),
        "Restoring the same key keeps it unchanged",
      );
    }
    const local = (p) =>
      p.startsWith(String.fromCharCode(92, 92, 63, 92)) ? p.slice(4) : p;
    const stage = local(state.stage.path);
    assert(stage.startsWith(root + path.sep));
    const exchange = path.join(
      path.dirname(path.dirname(path.dirname(path.dirname(stage)))),
      "exchange",
    );
    assert(fs.existsSync(path.join(exchange, "project.json")));
    for (
      let n = 0;
      n < 100 &&
      !fs.existsSync(
        path.join(exchange, "History", updated.entries[0].id, "changelog.json"),
      );
      n++
    )
      await new Promise((r) => setTimeout(r, 100));
    fs.writeFileSync(
      path.join(root, "recovery-checkpoint.json"),
      JSON.stringify({ exchange, endpoint: link.origin }),
    );
    // Check what the coordinator actually stores, not just what the app shows.
    const stored = require("./coordinator.cjs").fromEnv().dump();
    for (const s of [
      "Fisch",
      "Harbour",
      "Private peer proof",
      "Separate key project",
      "creator-private-folder",
      "History/",
      "mcollab-key-v2:",
    ])
      assert(!stored.includes(s), "Coordinator database contains " + s);
    // Make sure this looked at the right database.
    assert(stored.includes(state.project.id));
    const files = fs
      .readdirSync(root, { recursive: true })
      .filter((f) => f.endsWith(".dpapi"));
    assert(files.length >= 2);
    for (const f of files) {
      const b = fs.readFileSync(path.join(root, f));
      assert(b.subarray(0, 8).equals(Buffer.from("MCCRED01")));
      assert(!b.includes(Buffer.from(codec.keys.index)));
      assert(!b.includes(Buffer.from(bundle)));
    }
    process.stdout.write(
      execFileSync(process.execPath, ["tests/invite-smoke.cjs"], {
        env: { ...process.env, MCOLLAB_TEST_URL: link.origin },
        encoding: "utf8",
        windowsHide: true,
      }),
    );
    console.log(
      "Native privacy checks passed: team-metadata practice creation and peer joining, service wire/database audit, key-first invitation recovery, local filtered/paginated History, team-owned slice renames, historical-key rotation/backup merge and DPAPI. Synthetic data only.",
    );
  } finally {
    await browser.close();
  }
})().catch((e) => {
  console.error(e);
  process.exit(1);
});

// Plays the team-folder side of the protocol for tests. Names and changelogs are kept
// here, like the team folder would, and never sent to the coordinator.
const { PrivateWire: CryptoWire } = require("./private-wire-crypto.cjs");
const crypto = require("node:crypto"),
  assert = require("node:assert/strict"),
  fs = require("node:fs"),
  path = require("node:path"),
  { execFileSync } = require("node:child_process");
const sha = (b) => crypto.createHash("sha256").update(b).digest("hex");
class PrivateWire extends CryptoWire {
  constructor(bundle, endpoint, project, state) {
    super(bundle, endpoint, project);
    this.state = state || {
      profiles: {},
      manifests: {},
      documents: {},
      revisions: {},
      names: {},
      accepted: {},
      project: null,
    };
    this.state.documents ??= {};
    this.state.committed ??= {};
    this.loadTestFolder();
  }
  loadTestFolder() {
    const configured = process.env.MCOLLAB_TEST_DATA_DIR;
    if (!configured) return;
    const root = path.resolve(configured);
    if (
      path.basename(path.dirname(root)) !== ".tools" ||
      !fs.existsSync(path.join(root, "collab.db"))
    )
      return;
    const script =
      "import sqlite3,json,sys; c=sqlite3.connect('file:'+sys.argv[1]+'?mode=ro',uri=True); r=c.execute(\"SELECT value FROM kv WHERE key='active'\").fetchone(); print(r[0] if r else 'null')";
    const active = JSON.parse(
      execFileSync("python", ["-c", script, path.join(root, "collab.db")], {
        encoding: "utf8",
        windowsHide: true,
      }),
    );
    if (active?.project.id !== this.project || !active.folder) return;
    const prefix = String.fromCharCode(92, 92, 63, 92);
    const folder = path.resolve(
      active.folder.startsWith(prefix) ? active.folder.slice(4) : active.folder,
    );
    assert(
      folder.startsWith(root + path.sep),
      "Fixture storage escaped the synthetic profile",
    );
    this.loadFolder(folder);
    this.state.actor = active.member;
  }
  loadFolder(folder) {
    const document = JSON.parse(
      fs.readFileSync(path.join(folder, "Metadata/project.json"), "utf8"),
    );
    assert.equal(document.project, this.project);
    this.storageFolder = folder;
    this.state.project = {
      name: document.name,
      storage: document.storage,
      owner: document.owner,
    };
    const initial = path.join(folder, "project.json");
    if (fs.existsSync(initial))
      this.state.names =
        JSON.parse(fs.readFileSync(initial)).sectionNames || {};
    for (const member of fs.readdirSync(path.join(folder, "Members"))) {
      for (const file of fs.readdirSync(path.join(folder, "Members", member))) {
        const bytes = fs.readFileSync(
            path.join(folder, "Members", member, file),
          ),
          profile = JSON.parse(bytes);
        assert.equal(sha(bytes) + ".json", file);
        this.state.profiles[member] = {
          hash: sha(bytes),
          name: profile.name,
          nonce: profile.nonce,
        };
      }
    }
    const receipts = [];
    for (const id of fs.readdirSync(path.join(folder, "History"))) {
      const file = path.join(folder, "History", id, "manifest.json");
      if (!fs.existsSync(file)) continue;
      const bytes = fs.readFileSync(file),
        manifest = JSON.parse(bytes),
        fingerprint = sha(bytes);
      this.state.manifests[id] = {
        manifest,
        hash: fingerprint,
        controls: manifest.controls,
      };
      this.state.documents[fingerprint] = this.state.manifests[id];
      if (this.storageFolder) {
        const folder = path.join(this.storageFolder, "History", id);
        fs.mkdirSync(folder, { recursive: true });
        fs.writeFileSync(
          path.join(folder, "manifest.json"),
          JSON.stringify(manifest),
        );
      }
      for (let i = 0; i < manifest.controls.length; i++)
        this.state.revisions[manifest.controls[i].revision] = {
          file: manifest.update.revisions[i],
          control: manifest.controls[i],
        };
      const receipt = path.join(folder, "History", id, "receipt.json");
      if (fs.existsSync(receipt))
        receipts.push(JSON.parse(fs.readFileSync(receipt)));
    }
    for (const receipt of receipts.sort((a, b) => a.sequence - b.sequence))
      this.openValue(receipt);
    return this;
  }
  static generate(endpoint, project = crypto.randomUUID()) {
    const base = CryptoWire.generate(endpoint, project);
    return new PrivateWire(base.export(), endpoint, project);
  }
  at(endpoint) {
    return new PrivateWire(this.export(), endpoint, this.project, this.state);
  }
  profile(id, name) {
    assert(name && name.trim() && name.length <= 100);
    const nonce = this.state.profiles[id]?.nonce || crypto.randomUUID();
    const bytes = JSON.stringify({
      version: 1,
      nonce,
      project: this.project,
      member: id,
      name,
    });
    const hash = sha(bytes);
    this.state.profiles[id] = { hash, name, nonce };
    if (this.storageFolder) {
      const folder = path.join(this.storageFolder, "Members", id);
      fs.mkdirSync(folder, { recursive: true });
      fs.writeFileSync(path.join(folder, hash + ".json"), bytes);
    }
    return hash;
  }
  build(raw, id) {
    const previous = this.state.manifests[id],
      known = Object.values(this.state.revisions);
    const ids = Object.fromEntries(
      (raw.revisions || []).map((r) => [
        r.section,
        previous?.controls.find((c) => c.section === this.resource(r.section))
          ?.revision || crypto.randomUUID(),
      ]),
    );
    const controls = (raw.revisions || []).map((r) => {
      assert(
        /^[a-f0-9]{64}$/.test(r.hash) &&
          r.size > 0 &&
          r.size <= 128 * 1024 * 1024,
        "Invalid file metadata",
      );
      const find = (hash) =>
        known
          .filter(
            (x) =>
              this.state.committed[x.control.revision] && x.file.hash === hash,
          )
          .at(-1)?.control.revision;
      const parent = r.parent ? find(r.parent) : null,
        sharedDependency = r.sharedDependency
          ? ids.shared || find(r.sharedDependency)
          : null;
      // Unknown parent IDs stay valid UUIDs, so the coordinator is what rejects them.
      return {
        section: this.resource(r.section),
        revision: ids[r.section],
        parent: r.parent ? parent || crypto.randomUUID() : null,
        sharedDependency: r.sharedDependency
          ? sharedDependency || crypto.randomUUID()
          : null,
      };
    });
    const manifest = {
      version: 1,
      nonce: previous?.manifest.nonce || crypto.randomUUID(),
      project: this.project,
      update: { ...structuredClone(raw), id, at: 0, sequence: 0 },
      controls,
    };
    const fingerprint = sha(JSON.stringify(manifest));
    this.state.manifests[id] = { manifest, hash: fingerprint, controls };
    this.state.documents[fingerprint] = this.state.manifests[id];
    for (let i = 0; i < controls.length; i++)
      if (!this.state.committed[controls[i].revision])
        this.state.revisions[controls[i].revision] = {
          file: structuredClone(raw.revisions[i]),
          control: controls[i],
        };
    return this.state.manifests[id];
  }
  create(raw) {
    raw = structuredClone(raw);
    const member = raw.memberId || crypto.randomUUID(),
      id = raw.baselineId || crypto.randomUUID();
    this.state.project = {
      name: raw.name,
      storage: raw.storage,
      owner: member,
    };
    this.state.names = Object.fromEntries(
      (raw.baseline || [])
        .filter((r) => r.section !== "shared")
        .map((r) => [r.section, r.name]),
    );
    const profileHash = this.profile(member, raw.displayName);
    const manifest = this.build(
      {
        author: raw.displayName,
        authorId: member,
        summary: raw.summary || "Initialized project",
        details: raw.details || "Initial saved project version.",
        revisions: raw.baseline || [],
        restores: null,
        nameChanges: [],
        namesParent: null,
      },
      id,
    );
    return {
      id: this.project,
      memberId: member,
      profileHash,
      projectHash: sha(JSON.stringify(this.state.project)),
      bootstrap: this.seal("storage-bootstrap", raw.storage),
      baseline: manifest.controls,
      baselineId: id,
      manifestHash: manifest.hash,
    };
  }
  command(route, raw) {
    if (raw == null) return raw;
    const v = structuredClone(raw);
    if (route === "/join") {
      const member = v.memberId || crypto.randomUUID();
      return {
        invite: v.invite,
        memberId: member,
        profileHash: this.profile(member, v.displayName),
      };
    }
    if (route === "/invite")
      return {
        bootstrap: this.seal("storage-bootstrap", this.state.project.storage),
      };
    if (
      (route === "/publish" && v.revisions) ||
      (route === "/names" && v.changes)
    ) {
      assert(
        v.summary.trim() &&
          v.summary.length <= 500 &&
          (v.details || "").length <= 10000,
        "Write a changelog",
      );
      if (route === "/names")
        for (const c of v.changes) {
          assert(
            c.before.trim() &&
              c.after.trim() &&
              c.after.length <= 256 &&
              !/[\x00-\x1f]/.test(c.after),
          );
          const old = this.state.manifests[v.id]?.manifest.update.nameChanges;
          assert(
            this.state.names[c.section] === c.before ||
              JSON.stringify(old) === JSON.stringify(v.changes),
            "Stale name baseline",
          );
        }
      const authorId = v.authorId || this.state.actor || "fixture-author",
        author = this.state.profiles[authorId]?.name || "Fixture author";
      const rawUpdate = {
        id: v.id,
        author,
        authorId,
        summary: v.summary,
        details: v.details || "",
        revisions: v.revisions || [],
        restores: v.restores || null,
        nameChanges: v.changes || [],
        namesParent: route === "/names" ? v.parentVersion : null,
      };
      // The coordinator sets the author; we fill it in when decoding.
      const m = this.build(rawUpdate, v.id);
      if (route === "/names")
        return {
          ...(v.bytes ? { bytes: v.bytes } : {}),
          id: v.id,
          manifestHash: m.hash,
          parentVersion: v.parentVersion,
          sections: v.changes.map((c) => this.resource(c.section)),
        };
      return {
        ...(v.bytes ? { bytes: v.bytes } : {}),
        id: v.id,
        manifestHash: m.hash,
        revisions: m.controls,
        fences: Object.fromEntries(
          Object.entries(v.fences).map(([s, f]) => [this.resource(s), f]),
        ),
        restores: v.restores || null,
      };
    }
    if (route.startsWith("/locks/")) v.section = this.resource(v.section);
    if (route === "/access-requests/result" && Object.hasOwn(v, "granted"))
      return {
        id: v.id,
        result: this.seal("access-result:" + v.id, {
          status: v.granted ? "granted" : "failed",
          message: v.message || "",
        }),
      };
    return v;
  }
  openValue(raw) {
    if (Array.isArray(raw)) return raw.map((v) => this.openValue(v));
    if (!raw || typeof raw !== "object") return raw;
    const v = structuredClone(raw);
    if (v.project && v.member) this.state.actor = v.member;
    if (v.manifestHash && v.authorId) {
      const stored = this.state.documents[v.manifestHash];
      assert(
        stored && stored.hash === v.manifestHash,
        "Missing or changed fixture manifest",
      );
      const u = structuredClone(stored.manifest.update);
      u.at = v.at;
      u.sequence = v.sequence;
      u.authorId = v.authorId;
      u.author = this.state.profiles[v.authorId]?.name || "Fixture author";
      this.state.accepted[v.sequence] = v;
      for (const c of v.revisions) this.state.committed[c.revision] = true;
      for (const c of u.nameChanges || [])
        this.state.names[c.section] = c.after;
      return u;
    }
    if (v.revision && v.section) {
      const value = this.state.revisions[v.revision];
      assert(value, "Unknown revision");
      return structuredClone(value.file);
    }
    if (v.projectHash && v.heads) {
      for (const c of Object.values(v.heads))
        this.state.committed[c.revision] = true;
      v.name = this.state.project.name;
      v.storage = this.state.project.storage;
      v.sectionNames = { ...this.state.names };
      delete v.projectHash;
      v.heads = Object.fromEntries(
        Object.entries(v.heads).map(([s, r]) => [
          this.resourceName(s),
          this.openValue(r),
        ]),
      );
    }
    if (v.bootstrap) {
      // Before folder access, the invitation only has a generic title.
      v.projectName = "Shared project";
      v.storage = this.open(v.bootstrap, "storage-bootstrap");
      v.storageKind = v.storage.kind;
      delete v.bootstrap;
    }
    if (v.id && v.profileHash) {
      v.name = this.state.profiles[v.id]?.name || "Teammate";
      delete v.profileHash;
    }
    if (v.section && !v.revision) {
      v.section = this.resourceName(v.section);
      if (v.member) v.name = this.state.profiles[v.member]?.name || "Teammate";
    }
    for (const key of ["createdBy", "from", "holder"])
      if (typeof v[key] === "string")
        v[key] = this.state.profiles[v[key]]?.name || "Teammate";
    if (v.member && Object.hasOwn(v, "status")) return super.openValue(v);
    for (const key of ["project", "members", "locks", "entries", "presence"])
      if (v[key]) v[key] = this.openValue(v[key]);
    if (v.status === "settled") return this.open(v.message);
    return v;
  }
  transcode(value, target) {
    if (Array.isArray(value))
      return value.map((v) => this.transcode(v, target));
    if (typeof value === "string" && value.startsWith("m2:"))
      return target.seal(JSON.parse(value.slice(3)).p, this.open(value));
    if (!value || typeof value !== "object") return value;
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [
        k,
        k === "endpoint" && v === this.endpoint
          ? target.endpoint
          : this.transcode(v, target),
      ]),
    );
  }
}
module.exports = { PrivateWire };

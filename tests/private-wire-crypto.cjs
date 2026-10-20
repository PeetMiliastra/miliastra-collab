// The app's encryption, reimplemented in Node for tests.
// Keys are passed in locally and never sent to the coordinator.
const crypto = require("node:crypto"),
  assert = require("node:assert/strict");
const sha = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
const origin = (value) => {
  while (value.endsWith("/")) value = value.slice(0, -1);
  return value;
};
class PrivateWire {
  constructor(bundle, endpoint, project) {
    assert(
      typeof bundle === "string" &&
        bundle.startsWith("mcollab-key-v2:") &&
        bundle.length <= 40000,
    );
    this.keys = JSON.parse(
      Buffer.from(bundle.slice("mcollab-key-v2:".length), "base64url"),
    );
    assert(/^[a-f0-9]{64}$/.test(this.keys.index));
    assert(
      this.keys.keys[this.keys.current] &&
        Object.keys(this.keys.keys).length <= 128,
    );
    for (const [id, key] of Object.entries(this.keys.keys)) {
      assert(/^[a-f0-9]{64}$/.test(key));
      assert.equal(sha(Buffer.from(key, "hex")), id);
    }
    this.endpoint = origin(endpoint);
    this.project = project;
    this.resources = new Map([["shared", "shared"]]);
    for (let n = 0; n < 64; n++)
      this.resources.set(this.resource(String(n)), String(n));
  }
  static generate(endpoint, project = crypto.randomUUID()) {
    const key = crypto.randomBytes(32).toString("hex"),
      current = sha(Buffer.from(key, "hex"));
    const bundle =
      "mcollab-key-v2:" +
      Buffer.from(
        JSON.stringify({
          current,
          index: crypto.randomBytes(32).toString("hex"),
          keys: { [current]: key },
        }),
      ).toString("base64url");
    return new PrivateWire(bundle, endpoint, project);
  }
  export() {
    return (
      "mcollab-key-v2:" +
      Buffer.from(JSON.stringify(this.keys)).toString("base64url")
    );
  }
  at(endpoint) {
    return new PrivateWire(this.export(), endpoint, this.project);
  }
  context(purpose, clear) {
    return Buffer.from(
      JSON.stringify([
        "miliastra-metadata-v2",
        this.endpoint,
        this.project,
        purpose,
        [...clear],
      ]),
    );
  }
  commitment(purpose, clear) {
    return crypto
      .createHmac("sha256", Buffer.from(this.keys.index, "hex"))
      .update(this.context(purpose, clear))
      .digest("hex");
  }
  token(hash) {
    return this.commitment("revision-token", Buffer.from(hash));
  }
  resource(section) {
    if (section === "shared") return section;
    assert(/^(?:[0-9]|[1-5][0-9]|6[0-3])$/.test(section));
    return this.commitment("resource-token", Buffer.from(section));
  }
  resourceName(alias) {
    assert(this.resources.has(alias), "Unknown private slice");
    return this.resources.get(alias);
  }
  seal(purpose, value) {
    const clear = Buffer.from(JSON.stringify(value));
    assert(clear.length <= 32764 && purpose.length <= 256);
    const padded = Buffer.alloc(
      Math.max(256, 2 ** Math.ceil(Math.log2(clear.length + 4))),
    );
    padded.writeUInt32BE(clear.length);
    clear.copy(padded, 4);
    const e = {
      v: 2,
      k: this.keys.current,
      p: purpose,
      c: this.commitment(purpose, clear),
      n: crypto.randomBytes(12).toString("hex"),
      d: "",
    };
    const cipher = crypto.createCipheriv(
      "aes-256-gcm",
      Buffer.from(this.keys.keys[e.k], "hex"),
      Buffer.from(e.n, "hex"),
    );
    cipher.setAAD(
      Buffer.from(
        JSON.stringify([
          "miliastra-metadata-v2",
          this.endpoint,
          this.project,
          e.p,
          e.k,
          e.c,
        ]),
      ),
    );
    e.d = Buffer.concat([
      cipher.update(padded),
      cipher.final(),
      cipher.getAuthTag(),
    ]).toString("hex");
    padded.fill(0);
    return "m2:" + JSON.stringify(e);
  }
  open(value, purpose) {
    assert(
      typeof value === "string" &&
        value.startsWith("m2:") &&
        value.length <= 90000,
    );
    const e = JSON.parse(value.slice(3));
    assert.equal(e.v, 2);
    if (purpose) assert.equal(e.p, purpose);
    const bytes = Buffer.from(e.d, "hex");
    assert(bytes.length >= 272 && bytes.length <= 32784);
    const decipher = crypto.createDecipheriv(
      "aes-256-gcm",
      Buffer.from(this.keys.keys[e.k], "hex"),
      Buffer.from(e.n, "hex"),
    );
    decipher.setAAD(
      Buffer.from(
        JSON.stringify([
          "miliastra-metadata-v2",
          this.endpoint,
          this.project,
          e.p,
          e.k,
          e.c,
        ]),
      ),
    );
    decipher.setAuthTag(bytes.subarray(-16));
    const padded = Buffer.concat([
      decipher.update(bytes.subarray(0, -16)),
      decipher.final(),
    ]);
    try {
      const size = padded.readUInt32BE();
      assert(size <= padded.length - 4);
      assert(padded.subarray(4 + size).every((b) => b === 0));
      const clear = padded.subarray(4, 4 + size);
      assert(
        crypto.timingSafeEqual(
          Buffer.from(e.c, "hex"),
          Buffer.from(this.commitment(e.p, clear), "hex"),
        ),
      );
      return JSON.parse(clear.toString());
    } finally {
      padded.fill(0);
    }
  }
  revision(raw) {
    const r = structuredClone(raw),
      section = this.resource(r.section),
      hash = this.token(r.hash);
    const publication = r.artifact.split("/")[1];
    assert(publication);
    return {
      section,
      name: this.seal("slice:" + section, r.name),
      hash,
      contentHash: "0".repeat(64),
      parent: r.parent ? this.token(r.parent) : null,
      artifact: this.seal(
        "revision:" + publication + ":" + section + ":" + hash,
        r,
      ),
      size: 1,
      sharedDependency: r.sharedDependency
        ? this.token(r.sharedDependency)
        : null,
    };
  }
  openRevision(wire) {
    const envelope = JSON.parse(wire.artifact.slice(3)),
      publication = envelope.p.split(":")[1];
    const raw = this.open(
      wire.artifact,
      "revision:" + publication + ":" + wire.section + ":" + wire.hash,
    );
    assert.equal(this.resource(raw.section), wire.section);
    assert.equal(this.token(raw.hash), wire.hash);
    assert.equal(
      raw.artifact,
      "History/" +
        publication +
        "/" +
        (raw.section === "shared"
          ? "shared.gilh"
          : String(Number(raw.section) + 1) + ".gis"),
    );
    assert.equal(raw.parent ? this.token(raw.parent) : null, wire.parent);
    assert.equal(
      raw.sharedDependency ? this.token(raw.sharedDependency) : null,
      wire.sharedDependency,
    );
    assert.equal(this.open(wire.name, "slice:" + wire.section), raw.name);
    assert.equal(wire.size, 1);
    assert.equal(wire.contentHash, "0".repeat(64));
    return raw;
  }
  create(raw) {
    raw = structuredClone(raw);
    raw.id = this.project;
    raw.memberId ??= crypto.randomUUID();
    raw.baselineId ??= crypto.randomUUID();
    raw.summary = this.seal(
      "summary:" + raw.baselineId,
      raw.summary ?? "Initialized project",
    );
    raw.details = this.seal(
      "details:" + raw.baselineId,
      raw.details ?? "Initial saved project version.",
    );
    raw.name = this.seal("project-name", raw.name);
    raw.displayName = this.seal("creator:" + raw.memberId, raw.displayName);
    raw.storage = {
      kind: "sealed",
      location: this.seal("storage", raw.storage),
      driveId: null,
    };
    raw.baseline = (raw.baseline || []).map((r) => this.revision(r));
    return raw;
  }
  command(route, raw) {
    if (raw === undefined || raw === null) return raw;
    const value = structuredClone(raw);
    if (route === "/join") {
      value.memberId ??= crypto.randomUUID();
      value.displayName = this.seal(
        "creator:" + value.memberId,
        value.displayName,
      );
    } else if (route.startsWith("/locks/"))
      value.section = this.resource(value.section);
    else if (route === "/publish" || route === "/names") {
      assert(value.summary.trim(), "Write a changelog");
      value.summary = this.seal("summary:" + value.id, value.summary);
      value.details = this.seal("details:" + value.id, value.details ?? "");
      if (route === "/publish") {
        value.revisions = value.revisions.map((r) => this.revision(r));
        value.fences = Object.fromEntries(
          Object.entries(value.fences).map(([id, f]) => [this.resource(id), f]),
        );
      } else
        value.changes = value.changes.map((c) => {
          const section = this.resource(c.section);
          return {
            section,
            before: this.seal("slice:" + section, c.before),
            after: this.seal("slice:" + section, c.after),
          };
        });
    } else if (
      route === "/access-requests/result" &&
      Object.hasOwn(value, "granted")
    ) {
      return {
        id: value.id,
        result: this.seal("access-result:" + value.id, {
          status: value.granted ? "granted" : "failed",
          message: value.message || "",
        }),
      };
    }
    return value;
  }
  openValue(value) {
    if (Array.isArray(value)) return value.map((v) => this.openValue(v));
    if (!value || typeof value !== "object") return value;
    const v = structuredClone(value);
    if (v.artifact && v.contentHash) return this.openRevision(v);
    if (v.projectName && v.storage) {
      assert.equal(v.storage.kind, "sealed");
      v.projectName = this.open(v.projectName, "project-name");
      v.storage = this.open(v.storage.location, "storage");
      v.storageKind = v.storage.kind;
    }
    if (v.storage && v.heads) {
      assert.equal(v.storage.kind, "sealed");
      v.storage = this.open(v.storage.location, "storage");
      v.name = this.open(v.name, "project-name");
    } else if (typeof v.name === "string")
      v.name = this.open(v.name, "creator:" + (v.member || v.id));
    if (v.authorId && v.summary) {
      v.author = this.open(v.author, "creator:" + v.authorId);
      v.summary = this.open(v.summary, "summary:" + v.id);
      v.details = this.open(v.details, "details:" + v.id);
    }
    if (v.nameChanges)
      v.nameChanges = v.nameChanges.map((c) => ({
        section: this.resourceName(c.section),
        before: this.open(c.before, "slice:" + c.section),
        after: this.open(c.after, "slice:" + c.section),
      }));
    if (v.sectionNames)
      v.sectionNames = Object.fromEntries(
        Object.entries(v.sectionNames).map(([id, name]) => [
          this.resourceName(id),
          this.open(name, "slice:" + id),
        ]),
      );
    if (v.member && Object.hasOwn(v, "status")) {
      let activity = null;
      try {
        assert(typeof v.status === "string" && v.status.startsWith("m2:"));
        activity = this.open(v.status, "presence:" + v.member);
        assert(
          activity &&
            typeof activity === "object" &&
            Object.keys(activity).length === 1 &&
            typeof activity.game === "boolean",
        );
      } catch {
        activity = null;
      }
      v.status = activity;
    }
    for (const key of ["createdBy", "from", "holder"])
      if (typeof v[key] === "string") {
        assert(JSON.parse(v[key].slice(3)).p.startsWith("creator:"));
        v[key] = this.open(v[key]);
      }
    if (v.section) v.section = this.resourceName(v.section);
    for (const key of [
      "project",
      "members",
      "locks",
      "entries",
      "revisions",
      "presence",
    ])
      if (Object.hasOwn(v, key)) v[key] = this.openValue(v[key]);
    if (v.heads)
      v.heads = Object.fromEntries(
        Object.entries(v.heads).map(([id, r]) => [
          this.resourceName(id),
          this.openValue(r),
        ]),
      );
    if (v.status === "settled") return this.open(v.message);
    return v;
  }
  // Lets a test proxy re-encrypt between two coordinator addresses, with test keys.
  transcode(value, target) {
    if (typeof value === "string" && value.startsWith("m2:")) {
      const e = JSON.parse(value.slice(3)),
        clear = this.open(value);
      let purpose = e.p;
      if (purpose.startsWith("slice:"))
        purpose =
          "slice:" + target.resource(this.resourceName(purpose.slice(6)));
      return target.seal(purpose, clear);
    }
    if (Array.isArray(value))
      return value.map((v) => this.transcode(v, target));
    if (!value || typeof value !== "object") return value;
    if (value.artifact && value.contentHash)
      return target.revision(this.openRevision(value));
    const out = {};
    for (const [key, v] of Object.entries(value)) {
      if (key === "endpoint" && v === this.endpoint) out[key] = target.endpoint;
      else if (key === "section")
        out[key] = target.resource(this.resourceName(v));
      else if (key === "heads" || key === "sectionNames" || key === "fences")
        out[key] = Object.fromEntries(
          Object.entries(v).map(([id, item]) => [
            target.resource(this.resourceName(id)),
            this.transcode(item, target),
          ]),
        );
      else out[key] = this.transcode(v, target);
    }
    return out;
  }
  sealState(raw) {
    const state = structuredClone(raw);
    state.name = this.seal("project-name", state.name);
    state.storage = {
      kind: "sealed",
      location: this.seal("storage", state.storage),
      driveId: null,
    };
    state.members = Object.fromEntries(
      Object.entries(state.members).map(([id, m]) => [
        id,
        { ...m, name: this.seal("creator:" + id, m.name) },
      ]),
    );
    state.heads = Object.fromEntries(
      Object.entries(state.heads).map(([id, r]) => [
        this.resource(id),
        this.revision(r),
      ]),
    );
    state.section_names = Object.fromEntries(
      Object.entries(state.section_names).map(([id, name]) => {
        const alias = this.resource(id);
        return [alias, this.seal("slice:" + alias, name)];
      }),
    );
    state.invite_creators = {};
    state.access_requests = {};
    return state;
  }
  sealUpdate(raw) {
    const u = structuredClone(raw);
    u.author = this.seal("creator:" + u.authorId, u.author);
    u.summary = this.seal("summary:" + u.id, u.summary);
    u.details = this.seal("details:" + u.id, u.details);
    u.revisions = u.revisions.map((r) => this.revision(r));
    if (u.nameChanges)
      u.nameChanges = u.nameChanges.map((c) => {
        const section = this.resource(c.section);
        return {
          section,
          before: this.seal("slice:" + section, c.before),
          after: this.seal("slice:" + section, c.after),
        };
      });
    return u;
  }
}
module.exports = { PrivateWire };
if (require.main === module) {
  const fs = require("node:fs"),
    args = process.argv.slice(2),
    arg = (name) => args[args.indexOf(name) + 1];
  const input = JSON.parse(fs.readFileSync(0, "utf8")),
    endpoint = arg("--endpoint"),
    keyFile = arg("--key-file");
  const codec = new PrivateWire(
    fs.readFileSync(keyFile, "utf8").trim(),
    endpoint,
    input.state.id,
  );
  process.stdout.write(
    JSON.stringify({
      state: codec.sealState(input.state),
      updates: input.updates.map((u) => codec.sealUpdate(u)),
    }),
  );
}

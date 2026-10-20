const assert = require("node:assert/strict"),
  crypto = require("node:crypto");
const { PrivateWire } = require("./private-wire.cjs");
const { PrivateWire: CryptoWire } = require("./private-wire-crypto.cjs");
const codec = PrivateWire.generate("https://one.example"),
  other = codec.at("https://two.example");
const member = crypto.randomUUID(),
  publication = crypto.randomUUID();
const raw = {
  section: "5",
  name: "Private boats",
  hash: "a".repeat(64),
  contentHash: "b".repeat(64),
  parent: null,
  artifact: "History/" + publication + "/6.gis",
  size: 123456,
  sharedDependency: null,
};
const create = codec.create({
  name: "Private world",
  displayName: "Private member",
  memberId: member,
  baselineId: publication,
  baseline: [raw],
  storage: { kind: "folder", location: "", driveId: null },
});
for (const key of ["name", "storage", "summary", "details", "displayName"])
  assert(!Object.hasOwn(create, key));
for (const secret of [
  "Private world",
  "Private member",
  "Private boats",
  raw.hash,
  "123456",
])
  assert(!JSON.stringify(create).includes(secret));
assert.match(create.baseline[0].revision, /^[a-f0-9-]{36}$/);
assert.deepEqual(
  Object.keys(create.baseline[0]).sort(),
  ["section", "revision", "parent", "sharedDependency"].sort(),
);
const receipt = {
  id: publication,
  authorId: member,
  manifestHash: create.manifestHash,
  at: 1,
  sequence: 1,
  revisions: create.baseline,
  restores: null,
  nameSections: [],
  namesParent: null,
};
assert.deepEqual(codec.openValue(receipt).revisions[0], raw);
const profile = codec.command("/join", {
  invite: "secret",
  displayName: "Another private member",
});
assert(profile.profileHash && !profile.displayName);
assert(!JSON.stringify(profile).includes("Another private member"));
const next = crypto.randomUUID(),
  revision = {
    ...raw,
    hash: "c".repeat(64),
    parent: raw.hash,
    artifact: "History/" + next + "/6.gis",
  };
const request = {
  id: next,
  summary: "Private fix",
  details: "Private details",
  revisions: [revision],
  fences: { 5: 1 },
  restores: null,
};
const compact = codec.command("/publish", request),
  retry = codec.command("/publish", request);
assert.equal(compact.manifestHash, retry.manifestHash);
assert.equal(compact.revisions[0].revision, retry.revisions[0].revision);
assert.equal(compact.revisions[0].parent, create.baseline[0].revision);
assert(!JSON.stringify(compact).includes("Private fix"));
assert(!Object.hasOwn(compact, "summary"));
assert.notEqual(
  codec.command("/publish", { ...request, summary: "Different content" })
    .manifestHash,
  compact.manifestHash,
);
const sealed = codec.seal("storage-bootstrap", {
  kind: "folder",
  location: "",
});
assert.deepEqual(codec.open(sealed, "storage-bootstrap"), {
  kind: "folder",
  location: "",
});
assert.throws(() => other.open(sealed, "storage-bootstrap"));
assert.deepEqual(
  other.open(
    codec.transcode({ bootstrap: sealed }, other).bootstrap,
    "storage-bootstrap",
  ),
  { kind: "folder", location: "" },
);
const unrelated = CryptoWire.generate(codec.endpoint, codec.project);
assert.throws(() => unrelated.open(sealed, "storage-bootstrap"));
const presence = [
  { member, status: codec.seal("presence:" + member, { game: true }) },
  { member: "bad", status: codec.seal("presence:bad", { game: "true" }) },
  { member: "plain", status: { game: true } },
];
assert.equal(codec.openValue(presence)[0].status.game, true);
assert.equal(codec.openValue(presence)[1].status, null);
assert.equal(codec.openValue(presence)[2].status, null);
console.log(
  "Compact fixture codec passed: team-owned descriptions, random revision IDs, stable retries, manifest fingerprints, encrypted bootstrap/presence, context/key binding.",
);

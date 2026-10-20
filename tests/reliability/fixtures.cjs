// Fake stage files in the game's file format (copied from the practice fixture).
const fs = require("node:fs"),
  path = require("node:path"),
  crypto = require("node:crypto");
const sha = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
const varint = (value) => {
  const out = [];
  do {
    let b = value % 128;
    value = Math.floor(value / 128);
    out.push(b | (value ? 128 : 0));
  } while (value);
  return Buffer.from(out);
};
const scalar = (field, value) =>
  Buffer.concat([varint(field * 8), varint(value)]);
const string = (field, bytes) => {
  bytes = Buffer.from(bytes);
  return Buffer.concat([varint(field * 8 + 2), varint(bytes.length), bytes]);
};
const frame = (kind, payload) => {
  const header = Buffer.alloc(20);
  [payload.length + 20, 1, 806, kind, payload.length].forEach((n, i) =>
    header.writeUInt32BE(n, i * 4),
  );
  const tail = Buffer.alloc(4);
  tail.writeUInt32BE(1657);
  return Buffer.concat([header, payload, tail]);
};
const slice = (section, label) =>
  frame(
    6,
    Buffer.concat([
      ...(section ? [scalar(1, section)] : []),
      string(2, Buffer.concat([scalar(1, 42), string(90, label)])),
    ]),
  );
function settings(level, selected = 63, stamp = 0) {
  const rows = [];
  for (let id = 0; id < 64; id++)
    rows.push(
      string(
        1,
        Buffer.concat([
          ...(id ? [scalar(1, id)] : []),
          string(2, "Slice " + (id + 1)),
        ]),
      ),
    );
  rows.push(scalar(2, selected));
  return frame(
    8,
    Buffer.concat([
      scalar(1, level),
      string(5, Buffer.concat(rows)),
      scalar(90, stamp),
    ]),
  );
}
function makeStage(root, level) {
  const stage = path.join(
    root,
    "saves",
    "SyntheticAccount",
    "Beyond_Local_Save_Level",
    String(level),
  );
  fs.mkdirSync(path.join(stage, "beyond_level_setting"), { recursive: true });
  fs.mkdirSync(path.join(stage, "beyond_level_slice"));
  fs.writeFileSync(
    path.join(stage, level + ".gil"),
    frame(
      2,
      Buffer.concat([
        scalar(1, level),
        string(2, "PrivateStageCanary"),
        string(43, "practice-v1"),
      ]),
    ),
  );
  for (let id = 0; id < 64; id++) {
    fs.writeFileSync(
      path.join(stage, "beyond_level_slice", level + "_" + (id + 1) + ".gis"),
      slice(id, "BaselinePayloadCanary-" + id),
    );
  }
  fs.writeFileSync(
    path.join(stage, "beyond_level_setting", "local_setting.gils"),
    settings(level),
  );
  fs.writeFileSync(
    path.join(stage, "beyond_level_slice", level + ".gilh"),
    frame(
      7,
      Buffer.concat([scalar(1, level), string(90, "SharedBaselineCanary")]),
    ),
  );
  return stage;
}
module.exports = { sha, slice, settings, makeStage };

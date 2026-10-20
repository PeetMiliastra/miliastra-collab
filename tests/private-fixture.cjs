// Test helper: encodes requests like the app does. Names and other details stay local.
const fs = require("node:fs"),
  { PrivateWire } = require("./private-wire.cjs");
const input = JSON.parse(fs.readFileSync(0, "utf8")),
  c = new PrivateWire(input.bundle, input.endpoint, input.project, input.state);
const value =
  input.operation === "decode"
    ? c.openValue(input.value)
    : input.route === "/projects"
      ? c.create(input.value)
      : c.command(input.route, input.value);
process.stdout.write(JSON.stringify({ value, state: c.state }));

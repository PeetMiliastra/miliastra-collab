const { test } = require("node:test"),
  assert = require("node:assert/strict"),
  http = require("node:http");
const { FaultProxy } = require("./fault-proxy.cjs");
test("proxy separates offline requests from lost replies after a real commit", async () => {
  let commits = 0;
  const server = http.createServer((req, res) => {
    req.resume();
    req.on("end", () => {
      if (req.url === "/publish") commits++;
      res.writeHead(200, { "content-type": "application/json" });
      res.end('{"ok":true}');
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const p = await new FaultProxy(
    "http://127.0.0.1:" + server.address().port,
  ).start();
  try {
    p.setOffline(true);
    assert.equal(
      (await fetch(p.origin + "/publish", { method: "POST", body: "{}" }))
        .status,
      503,
    );
    assert.equal(commits, 0);
    p.setOffline(false);
    p.drop = "/publish";
    await assert.rejects(
      fetch(p.origin + "/publish", { method: "POST", body: "{}" }),
    );
    assert.equal(commits, 1);
    assert.equal(
      (await fetch(p.origin + "/publish", { method: "POST", body: "{}" }))
        .status,
      200,
    );
    assert.equal(commits, 2);
    assert.equal(p.audit(["private-canary"]).privateCanariesAbsent, true);
    assert.throws(() => p.audit(['"ok":true']));
  } finally {
    await p.close();
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
});

test("public invitation origins are rewritten without changing encrypted values; auth is audited", async () => {
  let upstreamOrigin;
  const opaque = "m2:opaque-authenticated-envelope";
  const server = http.createServer((req, res) => {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(
      JSON.stringify({
        invitation: { endpoint: upstreamOrigin },
        name: opaque,
      }),
    );
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  upstreamOrigin = "http://127.0.0.1:" + server.address().port;
  const proxy = await new FaultProxy(upstreamOrigin).start();
  try {
    const response = await fetch(proxy.origin + "/invitation", {
      headers: { authorization: "Bearer private-header-canary" },
    });
    const document = await response.json();
    assert.equal(document.invitation.endpoint, proxy.origin);
    assert.equal(document.name, opaque);
    assert.throws(
      () => proxy.audit(["private-header-canary"]),
      /Private canary/,
    );
  } finally {
    await proxy.close();
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
  }
});

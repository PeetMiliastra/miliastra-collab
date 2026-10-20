const http = require("node:http"),
  net = require("node:net"),
  crypto = require("node:crypto");
class FaultProxy {
  constructor(upstream) {
    this.upstream = new URL(upstream);
    if (
      this.upstream.protocol !== "http:" ||
      this.upstream.hostname !== "127.0.0.1"
    )
      throw Error("Synthetic loopback upstream required");
    this.offline = false;
    this.events = true;
    this.drop = null;
    this.records = [];
    this.bridges = new Set();
    this.server = http.createServer((req, res) => this.request(req, res));
    this.server.on("upgrade", (req, socket, head) =>
      this.upgrade(req, socket, head),
    );
  }
  async start() {
    await new Promise((r) => this.server.listen(0, "127.0.0.1", r));
    this.origin = "http://127.0.0.1:" + this.server.address().port;
    return this;
  }
  setOffline(value) {
    this.offline = value;
    if (value) this.disconnectEvents();
  }
  disconnectEvents() {
    for (const socket of this.bridges) socket.destroy();
    this.bridges.clear();
  }
  setEvents(value) {
    this.events = value;
    if (!value) this.disconnectEvents();
  }
  async request(req, res) {
    if (this.offline) {
      res.writeHead(503, { "content-type": "application/json" });
      res.end(
        JSON.stringify({ error: "Synthetic coordinator offline; retry" }),
      );
      return;
    }
    const parts = [];
    for await (const chunk of req) {
      parts.push(chunk);
      if (parts.reduce((n, b) => n + b.length, 0) > 2 * 1024 * 1024) {
        res.writeHead(413);
        res.end();
        return;
      }
    }
    const body = Buffer.concat(parts);
    const record = {
      method: req.method,
      path: req.url,
      body: body.toString(),
      authorization: req.headers.authorization || "",
      response: "",
      status: 0,
    };
    if (this.records.length >= 5000) {
      res.writeHead(507);
      res.end();
      return;
    }
    this.records.push(record);
    const upstream = http.request(
      new URL(req.url, this.upstream),
      {
        method: req.method,
        headers: { ...req.headers, host: this.upstream.host },
      },
      (response) => {
        const chunks = [];
        response.on("data", (c) => chunks.push(c));
        response.on("end", () => {
          record.response = Buffer.concat(chunks).toString();
          record.status = response.statusCode;
          if (
            this.drop &&
            req.method === "POST" &&
            req.url.endsWith(this.drop)
          ) {
            this.drop = null;
            res.destroy();
            return;
          }
          const headers = { ...response.headers };
          delete headers["transfer-encoding"];
          delete headers["content-length"];
          // Act like a reverse proxy: rewrite the public address field and nothing else.
          let outgoing = Buffer.concat(chunks);
          if ((headers["content-type"] || "").includes("application/json")) {
            const document = JSON.parse(outgoing.toString());
            const visit = (value) => {
              if (!value || typeof value !== "object") return;
              for (const key of Object.keys(value)) {
                if (key === "endpoint" && value[key] === this.upstream.origin)
                  value[key] = this.origin;
                else visit(value[key]);
              }
            };
            visit(document);
            outgoing = Buffer.from(JSON.stringify(document));
          }
          res.writeHead(response.statusCode, headers);
          res.end(outgoing);
        });
      },
    );
    upstream.on("error", () => {
      if (!res.destroyed) {
        res.writeHead(502);
        res.end("Synthetic upstream unavailable");
      }
    });
    upstream.end(body);
  }
  upgrade(req, socket, head) {
    if (this.offline || !this.events) {
      socket.destroy();
      return;
    }
    const target = net.connect(Number(this.upstream.port), "127.0.0.1");
    this.bridges.add(socket);
    this.bridges.add(target);
    const dispose = () => {
      this.bridges.delete(socket);
      this.bridges.delete(target);
      socket.destroy();
      target.destroy();
    };
    target.on("error", dispose);
    socket.on("error", dispose);
    socket.on("close", dispose);
    target.on("connect", () => {
      const headers = Object.entries({
        ...req.headers,
        host: this.upstream.host,
      })
        .map(([k, v]) => k + ": " + v)
        .join("\r\n");
      target.write(
        req.method + " " + req.url + " HTTP/1.1\r\n" + headers + "\r\n\r\n",
      );
      if (head.length) target.write(head);
      socket.pipe(target);
      target.pipe(socket);
    });
  }
  audit(canaries) {
    const bytes = this.records
      .map(
        (r) =>
          r.body + "\n" + r.response + "\n" + r.path + "\n" + r.authorization,
      )
      .join("\n");
    for (const secret of canaries)
      if (secret && bytes.includes(secret))
        throw Error("Private canary reached coordinator wire");
    return {
      requests: this.records.length,
      publicationRequests: this.records.filter((r) =>
        r.path.endsWith("/publish"),
      ).length,
      privateCanariesAbsent: true,
    };
  }
  async close() {
    this.disconnectEvents();
    this.server.closeAllConnections();
    await new Promise((r) => this.server.close(r));
    this.records.length = 0;
  }
}
module.exports = { FaultProxy };

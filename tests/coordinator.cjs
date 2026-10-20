// Runs the real coordinator on a throwaway PostgreSQL database, for native tests.
//
//   node tests/coordinator.cjs [--port=8787] [--minutes=20] [-- <command> <args>]
//
// With a command, it runs the command with MCOLLAB_TEST_URL, MCOLLAB_TEST_DATABASE_URL and
// MCOLLAB_PG_BIN set, then removes the database. A command still running after --minutes is
// stopped and fails. Without a command, it keeps running until Ctrl+C.
//
// The coordinator source is MCOLLAB_COORDINATOR_DIR, or the "Miliastra Collab Coordinator"
// folder next to this project. PostgreSQL comes from MCOLLAB_PG_BIN, .tools/pgsql/bin or PGBIN.
const { execFileSync, spawn } = require("node:child_process");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const exe = (name) => (process.platform === "win32" ? name + ".exe" : name);

function coordinatorDir() {
  const dir =
    process.env.MCOLLAB_COORDINATOR_DIR ||
    path.join(ROOT, "..", "Miliastra Collab Coordinator");
  if (!fs.existsSync(path.join(dir, "Cargo.toml")))
    throw new Error(
      `Coordinator source not found at ${dir}. Set MCOLLAB_COORDINATOR_DIR.`,
    );
  return dir;
}

function pgBin() {
  for (const dir of [
    process.env.MCOLLAB_PG_BIN,
    path.join(ROOT, ".tools", "pgsql", "bin"),
    process.env.PGBIN,
  ])
    if (dir && fs.existsSync(path.join(dir, exe("initdb")))) return dir;
  throw new Error(
    "PostgreSQL not found. Set MCOLLAB_PG_BIN to its bin folder.",
  );
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

function portInUse(port) {
  return new Promise((resolve) => {
    const socket = net.connect(port, "127.0.0.1");
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => resolve(false));
  });
}

async function waitForHealth(url, service, log) {
  for (let i = 0; i < 200; i++) {
    if (service.exitCode !== null)
      throw new Error(
        "The coordinator stopped:\n" + fs.readFileSync(log, "utf8"),
      );
    try {
      if ((await fetch(url + "/health")).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(
    "The coordinator didn't start:\n" + fs.readFileSync(log, "utf8"),
  );
}

/** A database client for checks on what the coordinator stored. */
function database(databaseUrl, bin = pgBin()) {
  const run = (tool, args) =>
    execFileSync(path.join(bin, exe(tool)), [...args, databaseUrl], {
      encoding: "utf8",
      windowsHide: true,
    });
  return {
    databaseUrl,
    /** One value per row, columns separated by "|". */
    query: (sql) =>
      run("psql", ["-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-c", sql])
        .split(/\r?\n/)
        .filter(Boolean),
    /** Everything the coordinator stored, as text. */
    dump: () => run("pg_dump", ["--data-only", "--no-owner"]),
  };
}

/** The database a wrapping `node tests/coordinator.cjs -- ...` started. */
function fromEnv() {
  const url = process.env.MCOLLAB_TEST_DATABASE_URL;
  if (!url)
    throw new Error(
      "Run this test with node tests/coordinator.cjs -- <command>.",
    );
  return database(url, process.env.MCOLLAB_PG_BIN || pgBin());
}

/** Stops and removes databases whose test run was killed before it could clean up. */
function removeAbandoned(bin) {
  for (const name of fs.readdirSync(os.tmpdir())) {
    if (!name.startsWith("mcollab-coordinator-")) continue;
    const root = path.join(os.tmpdir(), name);
    try {
      process.kill(
        Number(fs.readFileSync(path.join(root, "owner.pid"), "utf8")),
        0,
      );
      continue; // Its run is still going.
    } catch {}
    try {
      execFileSync(
        path.join(bin, exe("pg_ctl")),
        ["-D", path.join(root, "pgdata"), "-m", "immediate", "-w", "stop"],
        { stdio: "ignore", windowsHide: true },
      );
    } catch {}
    fs.rmSync(root, { recursive: true, force: true });
  }
}

async function start({ port = 8787 } = {}) {
  const dir = coordinatorDir(),
    bin = pgBin();
  removeAbandoned(bin);
  if (port && (await portInUse(port)))
    throw new Error(
      `Port ${port} is in use. Quit miliastra/collab or the other service, then try again.`,
    );
  port ||= await freePort();
  const env = { ...process.env, CARGO_TARGET_DIR: path.join(dir, "target") };
  execFileSync("cargo", ["build", "--locked", "--quiet"], {
    cwd: dir,
    env,
    stdio: "inherit",
    windowsHide: true,
  });
  const service = path.join(dir, "target", "debug", exe("miliastra-service"));

  const root = fs.mkdtempSync(path.join(os.tmpdir(), "mcollab-coordinator-"));
  fs.writeFileSync(path.join(root, "owner.pid"), String(process.pid));
  const data = path.join(root, "pgdata"),
    pgPort = await freePort();
  const pg = (tool, args) =>
    execFileSync(path.join(bin, exe(tool)), args, {
      stdio: "ignore",
      windowsHide: true,
    });
  pg("initdb", [
    "-D",
    data,
    "-U",
    "collab",
    "-A",
    "trust",
    "-E",
    "UTF8",
    "--no-locale",
  ]);
  pg("pg_ctl", [
    "-D",
    data,
    "-l",
    path.join(root, "postgres.log"),
    "-o",
    `-p ${pgPort} -h 127.0.0.1`,
    "-w",
    "start",
  ]);
  const databaseUrl = `postgres://collab@127.0.0.1:${pgPort}/collab`;
  pg("createdb", [
    "-h",
    "127.0.0.1",
    "-p",
    String(pgPort),
    "-U",
    "collab",
    "collab",
  ]);

  const url = `http://127.0.0.1:${port}`,
    log = path.join(root, "coordinator.log");
  let child;
  const launch = async () => {
    const out = fs.openSync(log, "a");
    child = spawn(service, [], {
      env: {
        ...process.env,
        DATABASE_URL: databaseUrl,
        BIND_ADDR: `127.0.0.1:${port}`,
        PUBLIC_URL: url,
        DISABLE_RATE_LIMITS: "1",
      },
      stdio: ["ignore", out, out],
      windowsHide: true,
    });
    await waitForHealth(url, child, log);
  };
  const halt = async () => {
    if (child && child.exitCode === null) {
      const exited = new Promise((r) => child.once("exit", r));
      child.kill();
      await exited;
    }
  };
  await launch();
  return {
    ...database(databaseUrl, bin),
    url,
    log,
    /** Stops and starts the service; the database is kept. */
    restart: async () => {
      await halt();
      await launch();
    },
    stop: async () => {
      await halt();
      try {
        pg("pg_ctl", ["-D", data, "-m", "immediate", "-w", "stop"]);
      } catch {}
      fs.rmSync(root, { recursive: true, force: true });
    },
  };
}

module.exports = { start, database, fromEnv };

if (require.main === module) {
  (async () => {
    const args = process.argv.slice(2),
      split = args.indexOf("--");
    const options = split < 0 ? args : args.slice(0, split),
      command = split < 0 ? [] : args.slice(split + 1);
    const port = options.find((o) => o.startsWith("--port="));
    const limit = options.find((o) => o.startsWith("--minutes="));
    const minutes = limit ? Number(limit.slice(10)) : 20;
    const coordinator = await start({
      port: port ? Number(port.slice(7)) : 8787,
    });
    console.log(`Test coordinator at ${coordinator.url}`);
    let code = 0;
    try {
      if (!command.length) {
        console.log("Press Ctrl+C to stop it.");
        await new Promise((r) => process.once("SIGINT", r));
      } else {
        const child = spawn(command[0], command.slice(1), {
          stdio: "inherit",
          env: {
            ...process.env,
            MCOLLAB_TEST_URL: coordinator.url,
            MCOLLAB_TEST_DATABASE_URL: coordinator.databaseUrl,
            MCOLLAB_PG_BIN: pgBin(),
          },
        });
        const timer = setTimeout(() => {
          console.error(
            `Stopped after ${minutes} minutes: ${command.join(" ")}`,
          );
          // Stop the whole process tree, including apps a test script started.
          if (process.platform === "win32")
            try {
              execFileSync(
                "taskkill",
                ["/pid", String(child.pid), "/t", "/f"],
                {
                  stdio: "ignore",
                },
              );
            } catch {}
          else child.kill();
        }, minutes * 60_000);
        code = await new Promise((resolve) =>
          child.once("exit", (c) => resolve(c ?? 1)),
        );
        clearTimeout(timer);
      }
    } finally {
      await coordinator.stop();
    }
    process.exit(code);
  })().catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}

/* global process, fetch, AbortSignal */
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { createReadStream } from "node:fs";
import { mkdir, realpath, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import path from "node:path";
import { setTimeout } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const expectedIdentifier = "com.mediaingest.tool.macos-ui-tests";

function readArguments(args) {
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const option = args[index];
    if (
      !["--app", "--output", "--expected-arch"].includes(option) ||
      !args[index + 1]
    ) {
      throw new Error(
        "Usage: node scripts/run-macos-ui.mjs --app <test.app> --output <new external directory> --expected-arch <x86_64|arm64>",
      );
    }
    if (values[option]) throw new Error("Each smoke argument must appear only once");
    values[option] = args[index + 1];
  }
  for (const option of ["--app", "--output", "--expected-arch"]) {
    if (!values[option]) throw new Error(`Missing required option ${option}`);
  }
  if (!["x86_64", "arm64"].includes(values["--expected-arch"])) {
    throw new Error("Expected architecture must be x86_64 or arm64");
  }
  return values;
}

function nativeText(command, args, allowMissing = false) {
  const result = spawnSync(command, args, { encoding: "utf8", shell: false });
  if (result.error || result.status !== 0) {
    if (allowMissing) return "";
    throw new Error(`Required native check ${path.basename(command)} failed`);
  }
  return result.stdout.trim();
}

async function unusedLoopbackPort() {
  const server = createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

async function stopApp(app) {
  if (app.exitCode !== null || app.signalCode !== null) return false;
  const exited = once(app, "exit");
  app.kill("SIGTERM");
  await Promise.race([exited, setTimeout(5_000, undefined, { ref: false })]);
  if (app.exitCode !== null || app.signalCode !== null) return false;
  app.kill("SIGKILL");
  await Promise.race([exited, setTimeout(5_000, undefined, { ref: false })]);
  return true;
}

async function waitForDriver(app, port) {
  const deadline = Date.now() + 60_000;
  do {
    if (app.exitCode !== null || app.signalCode !== null) {
      throw new Error("The instrumented native app exited before driver startup");
    }
    try {
      const response = await fetch(`http://127.0.0.1:${port}/status`, {
        signal: AbortSignal.timeout(1_000),
      });
      const data = await response.json();
      if (response.ok && data.value?.ready === true) return;
    } catch {
      // Connection refusal is expected until the native app has initialized.
    }
    await setTimeout(100);
  } while (Date.now() < deadline);
  throw new Error("The test-only embedded WebDriver did not become ready");
}

async function main() {
  const options = readArguments(process.argv.slice(2));
  if (process.platform !== "darwin") {
    throw new Error(
      "Native WKWebView smoke must run on macOS; other OSes cannot validate it",
    );
  }
  const expectedArch = options["--expected-arch"];
  const nodeArch = process.arch === "x64" ? "x86_64" : process.arch;
  const hardwareArch =
    nativeText("/usr/sbin/sysctl", ["-n", "hw.optional.arm64"]) === "1"
      ? "arm64"
      : "x86_64";
  if (
    nodeArch !== expectedArch ||
    hardwareArch !== expectedArch ||
    nativeText("/usr/sbin/sysctl", ["-in", "sysctl.proc_translated"], true) === "1"
  ) {
    throw new Error(
      "Runner, Node, and expected CPU must match without Rosetta translation",
    );
  }
  const appDirectory = await realpath(options["--app"]);
  if (!appDirectory.endsWith(".app") || !(await stat(appDirectory)).isDirectory()) {
    throw new Error("A built test-only .app bundle is required");
  }
  const plist = path.join(appDirectory, "Contents/Info.plist");
  const identifier = nativeText("/usr/bin/plutil", [
    "-extract",
    "CFBundleIdentifier",
    "raw",
    "-o",
    "-",
    plist,
  ]);
  if (identifier !== expectedIdentifier) {
    throw new Error(
      "Refusing to instrument a shipping bundle; build with the isolated test config",
    );
  }
  const executableName = nativeText("/usr/bin/plutil", [
    "-extract",
    "CFBundleExecutable",
    "raw",
    "-o",
    "-",
    plist,
  ]);
  if (!executableName || path.basename(executableName) !== executableName) {
    throw new Error("The test bundle executable name is invalid");
  }
  const executable = await realpath(
    path.join(appDirectory, "Contents/MacOS", executableName),
  );
  if (!executable.startsWith(`${appDirectory}${path.sep}`)) {
    throw new Error("The test executable must stay inside its .app bundle");
  }
  const binaryArchitectures = nativeText("/usr/bin/lipo", ["-archs", executable]).split(
    /\s+/,
  );
  if (binaryArchitectures.length !== 1 || binaryArchitectures[0] !== expectedArch) {
    throw new Error(
      "The smoke bundle must contain only the requested native CPU architecture",
    );
  }
  const outputDirectory = path.resolve(options["--output"]);
  if (outputDirectory === root || outputDirectory.startsWith(`${root}${path.sep}`)) {
    throw new Error("Keep native evidence outside the repository");
  }
  // Require a fresh output directory so prior screenshots cannot pass a new run.
  await mkdir(outputDirectory, { recursive: false });
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(executable)) hash.update(chunk);
  const report = {
    schemaVersion: 1,
    evidenceKind: "native-wkwebview-test-build",
    identifier,
    architecture: expectedArch,
    macosVersion: nativeText("/usr/bin/sw_vers", ["-productVersion"]),
    sourceCommit: nativeText("git", ["-C", root, "rev-parse", "HEAD"]),
    dirtySource: Boolean(nativeText("git", ["-C", root, "status", "--porcelain"])),
    binarySha256: hash.digest("hex"),
    appleDeveloperCredentialsRequired: false,
    mockedCommands: false,
    expectedEmptyInventory: true,
    status: "failed",
  };
  const port = await unusedLoopbackPort();
  const app = spawn(executable, [], {
    cwd: root,
    env: {
      ...process.env,
      MEDIA_INGEST_MACOS_UI_TESTS: "1",
      TAURI_WEBDRIVER_PORT: String(port),
    },
    // Native logs can contain private paths/device details. This smoke publishes
    // only bounded assertions, counts and the empty-state screenshot.
    stdio: "ignore",
    shell: false,
  });
  try {
    await once(app, "spawn");
    await waitForDriver(app, port);
    const tests = spawn(
      process.execPath,
      ["--test", "--test-timeout=120000", "tests/native-macos/smoke.native.mjs"],
      {
        cwd: root,
        env: {
          ...process.env,
          TAURI_WEBDRIVER_PORT: String(port),
          MEDIA_INGEST_MACOS_UI_OUTPUT: outputDirectory,
          MEDIA_INGEST_MACOS_UI_EXPECT_EMPTY: "1",
        },
        stdio: "inherit",
        shell: false,
      },
    );
    const [code] = await once(tests, "exit");
    report.status = code === 0 ? "passed" : "failed";
    report.testExitCode = code;
    process.exitCode = code === 0 ? 0 : 1;
  } finally {
    report.forcedAppTermination = await stopApp(app);
    await writeFile(
      path.join(outputDirectory, "run.json"),
      `${JSON.stringify(report, null, 2)}\n`,
      {
        encoding: "utf8",
        flag: "wx",
      },
    );
  }
}

main().catch((error) => {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});

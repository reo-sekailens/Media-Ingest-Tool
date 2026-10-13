import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { constants, createReadStream } from "node:fs";
import {
  access,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const repository = fileURLToPath(new URL("../", import.meta.url));
const architectures = {
  "x86_64-apple-darwin": "x86_64",
  "aarch64-apple-darwin": "arm64",
};
export const PRIVACY_KEYS = Object.freeze([
  "NSRemovableVolumesUsageDescription",
  "NSNetworkVolumesUsageDescription",
  "NSDocumentsFolderUsageDescription",
  "NSDownloadsFolderUsageDescription",
  "NSDesktopFolderUsageDescription",
]);

function native(command, args, options = {}) {
  return execFileSync(command, args, {
    encoding: "utf8",
    timeout: 60000,
    maxBuffer: 8 * 1024 * 1024,
    ...options,
  }).trim();
}

export function versionAtMost(actual, maximum) {
  const parse = (value) => {
    if (typeof value !== "string" || !/^\d+\.\d+(?:\.\d+)?$/.test(value))
      throw new Error("Invalid macOS deployment version.");
    return value.split(".").map(Number);
  };
  const left = parse(actual);
  const right = parse(maximum);
  for (let index = 0; index < 3; index++) {
    if ((left[index] ?? 0) !== (right[index] ?? 0))
      return (left[index] ?? 0) < (right[index] ?? 0);
  }
  return true;
}

export function deploymentVersions(output) {
  const versions = [];
  for (const block of output.split(/Load command \d+/).slice(1)) {
    if (/\bcmd LC_BUILD_VERSION\b/.test(block)) {
      if (!/\bplatform (?:1|macOS)\b/i.test(block))
        throw new Error("Mach-O targets a platform other than macOS.");
      const version = block.match(/\bminos (\d+\.\d+(?:\.\d+)?)/)?.[1];
      if (!version) throw new Error("Missing LC_BUILD_VERSION minimum.");
      versions.push(version);
    } else if (/\bcmd LC_VERSION_MIN_MACOSX\b/.test(block)) {
      const version = block.match(/\bversion (\d+\.\d+(?:\.\d+)?)/)?.[1];
      if (!version) throw new Error("Missing LC_VERSION_MIN_MACOSX minimum.");
      versions.push(version);
    }
  }
  if (versions.length === 0) throw new Error("Missing Mach-O deployment target.");
  return versions;
}

export function linkedLibraries(output) {
  return output
    .split(/\r?\n/)
    .slice(1)
    .filter((line) => line.trim())
    .map((line) => {
      const library = line.match(/^\s+(.+?) \(compatibility version /)?.[1];
      if (!library) throw new Error("Unrecognized Mach-O dependency output.");
      return library;
    });
}

export function assertPlist(plist, config, macConfig) {
  if (
    plist.CFBundleIdentifier !== config.identifier ||
    plist.CFBundleShortVersionString !== config.version ||
    plist.CFBundlePackageType !== "APPL" ||
    plist.LSMinimumSystemVersion !== macConfig.bundle.macOS.minimumSystemVersion
  )
    throw new Error("App Info.plist identity/version/minimum differs from config.");
  if (
    typeof plist.CFBundleExecutable !== "string" ||
    !/^[a-zA-Z0-9._-]+$/.test(plist.CFBundleExecutable)
  )
    throw new Error("App executable must be a simple bundle-local filename.");
  if (typeof plist.CFBundleIconFile !== "string" || !plist.CFBundleIconFile)
    throw new Error("App icon is missing from Info.plist.");
  for (const key of PRIVACY_KEYS) {
    if (typeof plist[key] !== "string" || !plist[key].trim())
      throw new Error(`Missing macOS privacy purpose string: ${key}.`);
  }
}

export function assertSignature(output, hardenedRuntime) {
  if (!/^Signature=adhoc$/m.test(output) || /^Authority=/m.test(output))
    throw new Error("Manual-install bundle must carry an ad-hoc signature.");
  if (hardenedRuntime && !/^CodeDirectory .*\bruntime\b/m.test(output))
    throw new Error("Hardened runtime flag is absent from the signed binary.");
}

export function contained(root, file) {
  const relative = path.relative(root, file);
  return (
    relative === "" ||
    (!path.isAbsolute(relative) &&
      relative !== ".." &&
      !relative.startsWith(`..${path.sep}`))
  );
}

export function assertArchitecture(archs, expectedArch, isExecutable) {
  if (!archs.includes(expectedArch) || (isExecutable && archs.length !== 1))
    throw new Error("Bundle Mach-O architecture does not match its package target.");
}

export function isSystemLibrary(library) {
  const normalized = path.posix.normalize(library);
  return (
    normalized.startsWith("/usr/lib/") || normalized.startsWith("/System/Library/")
  );
}

export async function assertBundledDependency(library, candidates, bundle) {
  if (isSystemLibrary(library)) return;
  for (const candidate of candidates) {
    if (
      path.isAbsolute(candidate) &&
      contained(bundle, candidate) &&
      (await exists(candidate)) &&
      contained(bundle, await realpath(candidate))
    )
      return;
  }
  throw new Error(`Dependency is not a system or bundled library: ${library}.`);
}

async function exists(file) {
  try {
    await access(file);
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

async function findDmgs(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isFile() && entry.name.endsWith(".dmg")) files.push(file);
    else if (entry.isDirectory() && !entry.name.endsWith(".app"))
      files.push(...(await findDmgs(file)));
  }
  return files;
}

async function bundleFiles(root, directory = root) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isSymbolicLink()) {
      if (!contained(root, await realpath(file)))
        throw new Error("App bundle contains an external symbolic link.");
      // Framework aliases resolve to their separately inspected Versions files.
    } else if (entry.isDirectory()) files.push(...(await bundleFiles(root, file)));
    else if (entry.isFile()) files.push(file);
    else throw new Error("App bundle contains a non-regular entry.");
  }
  return files;
}

async function checkBinary(file, executable, bundle, expectedArch, minimum) {
  const archs = native("/usr/bin/lipo", ["-archs", file]).split(/\s+/);
  assertArchitecture(archs, expectedArch, file === executable);
  const loadCommands = native("/usr/bin/otool", ["-arch", expectedArch, "-l", file]);
  const versions = deploymentVersions(loadCommands);
  if (versions.some((version) => !versionAtMost(version, minimum)))
    throw new Error("Bundled Mach-O requires a newer macOS than the declared minimum.");
  const libraries = linkedLibraries(
    native("/usr/bin/otool", ["-arch", expectedArch, "-L", file]),
  );
  const expand = (value) =>
    value
      .replace(/^@executable_path(?=\/|$)/, path.dirname(executable))
      .replace(/^@loader_path(?=\/|$)/, path.dirname(file));
  const rpaths = loadCommands
    .split(/Load command \d+/)
    .filter((block) => /\bcmd LC_RPATH\b/.test(block))
    .map((block) => block.match(/\bpath (.+?) \(offset \d+\)/)?.[1]);
  for (const library of libraries) {
    if (isSystemLibrary(library)) continue;
    const candidates = library.startsWith("@rpath/")
      ? rpaths
          .filter(Boolean)
          .map((rpath) => path.join(expand(rpath), library.slice(7)))
      : [expand(library)];
    await assertBundledDependency(library, candidates, bundle);
  }
  return {
    file: path.relative(bundle, file),
    architectures: archs,
    versions,
    libraries,
  };
}

async function logSnapshot(directory) {
  if (!(await exists(directory))) return new Map();
  const snapshot = new Map();
  for (const name of await readdir(directory)) {
    if (!name.endsWith(".log")) continue;
    const file = path.join(directory, name);
    if ((await lstat(file)).isFile()) snapshot.set(name, await readFile(file, "utf8"));
  }
  return snapshot;
}

async function launchSmoke(bundle, executable, identifier, version) {
  const data = path.join(os.homedir(), "Library", "Application Support", identifier);
  if (await exists(path.join(data, "media-ingest.sqlite3")))
    throw new Error("Startup validation requires a fresh disposable user profile.");
  const logs = path.join(os.homedir(), "Library", "Logs", identifier);
  const before = await logSnapshot(logs);
  const environment = { ...process.env, PATH: "/usr/bin:/bin:/usr/sbin:/sbin" };
  const launcher = spawn("/usr/bin/open", ["-n", "-W", bundle], {
    env: environment,
    stdio: "ignore",
  });
  let launcherError;
  let launcherExited = false;
  launcher.once("error", (error) => {
    launcherError = error;
  });
  launcher.once("exit", () => {
    launcherExited = true;
  });
  let pid;
  try {
    const pattern = `^${executable.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}([[:space:]]|$)`;
    const deadline = Date.now() + 30000;
    let ready = false;
    while (Date.now() < deadline) {
      if (launcherError) throw launcherError;
      if (launcherExited) throw new Error("LaunchServices startup exited early.");
      try {
        const pids = native("/usr/bin/pgrep", ["-f", pattern]).split(/\s+/).map(Number);
        if (pids.length !== 1 || !Number.isSafeInteger(pids[0]))
          throw new Error("Startup did not resolve one exact installed process.");
        pid = pids[0];
      } catch (error) {
        if (error.status !== 1) throw error;
      }
      const current = await logSnapshot(logs);
      const appended = [...current]
        .map(([name, text]) =>
          text.startsWith(before.get(name) ?? "")
            ? text.slice((before.get(name) ?? "").length)
            : text,
        )
        .join("\n");
      ready =
        Boolean(pid) &&
        appended.includes(`application_start version=${version} platform=macos`) &&
        appended.includes("local_store_ready interrupted_runs_reconciled=true");
      if (ready) break;
      await delay(500);
    }
    if (!ready)
      throw new Error("Fresh startup and SQLite-ready markers were not observed.");
    // Detect immediate dyld, setup, or native-window crashes after successful setup.
    for (let second = 0; second < 10; second++) {
      await delay(1000);
      process.kill(pid, 0);
      if (launcherExited)
        throw new Error("Installed app terminated during startup smoke.");
    }
    return {
      launchServices: true,
      freshProfile: true,
      startupLogged: true,
      sqliteReady: true,
      survivedSeconds: 10,
    };
  } finally {
    stopProcess(pid);
    if (!launcherExited) launcher.kill("SIGTERM");
  }
}

function stopProcess(pid) {
  if (!pid) return;
  try {
    process.kill(pid, "SIGTERM");
  } catch (error) {
    if (error.code !== "ESRCH") throw error;
  }
}

async function removeTemporary(temporary) {
  const resolved = await realpath(temporary);
  if (
    !contained(await realpath(os.tmpdir()), resolved) ||
    !path.basename(resolved).startsWith("media-ingest-macos-validation-")
  )
    throw new Error("Refusing to remove an unexpected validation directory.");
  await rm(resolved, { recursive: true, force: true });
}

export async function verifyBundle(bundleDirectory, target, reportFile) {
  if (process.platform !== "darwin")
    throw new Error("Native bundle validation requires macOS.");
  if (!Object.hasOwn(architectures, target))
    throw new Error("Unsupported macOS target.");
  const expectedArch = architectures[target];
  if (native("/usr/bin/uname", ["-m"]) !== expectedArch)
    throw new Error(
      "Startup validation must run natively on the package architecture.",
    );
  const config = JSON.parse(
    await readFile(path.join(repository, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const macConfig = JSON.parse(
    await readFile(path.join(repository, "src-tauri/tauri.macos.conf.json"), "utf8"),
  );
  const minimum = macConfig.bundle.macOS.minimumSystemVersion;
  versionAtMost(minimum, minimum);
  const dmgs = await findDmgs(path.resolve(bundleDirectory));
  if (dmgs.length !== 1)
    throw new Error("Expected exactly one architecture-specific DMG.");
  const dmg = dmgs[0];
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(dmg)) digest.update(chunk);
  const report = {
    schemaVersion: 1,
    sourceCommit: native("git", ["rev-parse", "HEAD"], { cwd: repository }),
    target,
    osVersion: native("/usr/bin/sw_vers", ["-productVersion"]),
    minimumSystemVersion: minimum,
    dmg: { file: path.basename(dmg), sha256: digest.digest("hex") },
    scope:
      "DMG/copy/signature/native-startup; no hardware or rendered-UI certification",
    status: "running",
  };
  const temporary = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "media-ingest-macos-validation-")),
  );
  const mounted = path.join(temporary, "mounted");
  let attached = false;
  try {
    native("/usr/bin/hdiutil", ["verify", dmg]);
    report.imageVerified = true;
    await mkdir(mounted);
    native("/usr/bin/hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-noautoopen",
      "-mountpoint",
      mounted,
      dmg,
    ]);
    attached = true;
    const apps = (await readdir(mounted, { withFileTypes: true })).filter(
      (entry) => entry.isDirectory() && entry.name.endsWith(".app"),
    );
    if (apps.length !== 1) throw new Error("DMG must contain exactly one app bundle.");
    const sourceApp = path.join(mounted, apps[0].name);
    native("/usr/bin/codesign", [
      "--verify",
      "--deep",
      "--strict",
      "--verbose=2",
      sourceApp,
    ]);
    const installed = path.join(temporary, "Applications", apps[0].name);
    await mkdir(path.dirname(installed));
    native("/usr/bin/ditto", ["--rsrc", sourceApp, installed]);
    native("/usr/bin/codesign", [
      "--verify",
      "--deep",
      "--strict",
      "--verbose=2",
      installed,
    ]);
    // codesign emits display metadata to stderr, unlike verification success.
    const signature = spawnSync("/usr/bin/codesign", ["-d", "--verbose=4", installed], {
      encoding: "utf8",
      timeout: 30000,
    });
    if (signature.error || signature.status !== 0)
      throw new Error("Signature display failed.");
    const signatureDisplay = `${signature.stdout}\n${signature.stderr}`;
    assertSignature(signatureDisplay, macConfig.bundle.macOS.hardenedRuntime);
    report.signature = {
      kind: "adhoc",
      verifiedBeforeAndAfterCopy: true,
      hardenedRuntime: macConfig.bundle.macOS.hardenedRuntime === true,
      notarized: false,
    };
    const plistFile = path.join(installed, "Contents", "Info.plist");
    native("/usr/bin/plutil", ["-lint", plistFile]);
    const plist = JSON.parse(
      native("/usr/bin/plutil", ["-convert", "json", "-o", "-", plistFile]),
    );
    assertPlist(plist, config, macConfig);
    const iconName = plist.CFBundleIconFile.endsWith(".icns")
      ? plist.CFBundleIconFile
      : `${plist.CFBundleIconFile}.icns`;
    if (
      !contained(
        path.join(installed, "Contents", "Resources"),
        path.join(installed, "Contents", "Resources", iconName),
      ) ||
      !(await exists(path.join(installed, "Contents", "Resources", iconName)))
    )
      throw new Error("App icon does not exist inside Resources.");
    report.plistVerified = true;
    const executable = path.join(
      installed,
      "Contents",
      "MacOS",
      plist.CFBundleExecutable,
    );
    await access(executable, constants.X_OK);
    const files = await bundleFiles(installed);
    report.binaries = [];
    for (const file of files) {
      if (native("/usr/bin/file", ["-b", file]).includes("Mach-O"))
        report.binaries.push(
          await checkBinary(file, executable, installed, expectedArch, minimum),
        );
    }
    if (
      !report.binaries.some(
        (binary) => binary.file === path.relative(installed, executable),
      )
    )
      throw new Error("App executable is missing or is not Mach-O.");
    native("/usr/bin/hdiutil", ["detach", mounted]);
    attached = false;
    report.dmgCopiedAndDetached = true;
    report.startup = await launchSmoke(
      installed,
      executable,
      config.identifier,
      config.version,
    );
    report.status = "passed";
  } catch (error) {
    report.status = "failed";
    report.failure = error.message;
    throw error;
  } finally {
    await mkdir(path.dirname(path.resolve(reportFile)), { recursive: true });
    await writeFile(reportFile, `${JSON.stringify(report, null, 2)}\n`);
    if (attached) native("/usr/bin/hdiutil", ["detach", mounted]);
    await removeTemporary(temporary);
  }
  return report;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const [bundleDirectory, target, reportFile, ...extra] = process.argv.slice(2);
  if (!bundleDirectory || !target || !reportFile || extra.length)
    throw new Error(
      "Usage: node scripts/verify-macos-bundle.mjs <bundle-directory> <target> <report-file>",
    );
  const report = await verifyBundle(bundleDirectory, target, reportFile);
  process.stdout.write(`macOS ${report.osVersion} ${target}: ${report.status}\n`);
}

import { spawnSync } from "node:child_process";
import {
  lstatSync,
  mkdirSync,
  opendirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL, URL } from "node:url";

const repository = fileURLToPath(new URL("../", import.meta.url));
const filesystemEnums = new Set([
  "apfs",
  "hfs",
  "hfs+",
  "msdos",
  "exfat",
  "ntfs",
  "udf",
  "cd9660",
  "ufs",
  "lifs",
  "webdav",
  "smbfs",
  "nfs",
  "fusefs",
  "autofs",
  "none",
]);
const protocolEnums = new Set([
  "USB",
  "SATA",
  "SAS",
  "SCSI",
  "ATA",
  "ATAPI",
  "PCI",
  "PCI-Express",
  "NVMe",
  "Thunderbolt",
  "FireWire",
  "Secure Digital",
  "SD",
  "Disk Image",
  "Apple Fabric",
  "Virtual",
  "Unknown",
]);

function native(command, args, input) {
  const result = spawnSync(command, args, {
    input,
    encoding: "buffer",
    timeout: 30000,
    maxBuffer: 16 * 1024 * 1024,
    stdio: ["pipe", "pipe", "pipe"],
  });
  // Neither stderr nor an exception message may enter evidence: both may
  // include a volume name, mount, BSD identifier or other private values.
  return { okay: !result.error && result.status === 0, stdout: result.stdout };
}

function plist(commandArgs, run) {
  try {
    const result = run("/usr/sbin/diskutil", commandArgs);
    if (!result.okay) return null;
    const converted = run(
      "/usr/bin/plutil",
      ["-convert", "json", "-o", "-", "-"],
      result.stdout,
    );
    if (!converted.okay) return null;
    const parsed = JSON.parse(converted.stdout.toString("utf8"));
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? parsed
      : null;
  } catch {
    return null;
  }
}

function* diskEntries(dictionary) {
  if (!dictionary || typeof dictionary !== "object" || Array.isArray(dictionary))
    return;
  yield dictionary;
  for (const key of ["AllDisksAndPartitions", "Partitions", "APFSVolumes"]) {
    if (Array.isArray(dictionary[key])) {
      for (const child of dictionary[key]) yield* diskEntries(child);
    }
  }
}

function safeEnum(value, allowed, normalize = (entry) => entry) {
  if (typeof value !== "string") return "missing";
  const normalized = normalize(value);
  return allowed.has(normalized) ? normalized : "other";
}

export function safeProperties(dictionary) {
  const propertyKeys = Object.keys(dictionary)
    .filter((key) => /^[A-Za-z][A-Za-z0-9_]{0,127}$/.test(key))
    .sort();
  return {
    propertyKeys,
    booleanProperties: Object.fromEntries(
      propertyKeys
        .filter((key) => typeof dictionary[key] === "boolean")
        .map((key) => [key, dictionary[key]]),
    ),
    filesystemType: safeEnum(dictionary.FilesystemType, filesystemEnums, (value) =>
      value.toLowerCase(),
    ),
    busProtocol: safeEnum(dictionary.BusProtocol, protocolEnums),
    deviceProtocol: safeEnum(dictionary.DeviceProtocol, protocolEnums),
    virtualOrPhysical: safeEnum(
      dictionary.VirtualOrPhysical,
      new Set(["physical", "virtual"]),
      (value) => value.toLowerCase(),
    ),
  };
}

export function mountIsSystemRoot(mount) {
  return (
    mount === "/" || mount === "/System/Volumes" || mount.startsWith("/System/Volumes/")
  );
}

function rootReadAccess(mount) {
  let directory;
  try {
    directory = opendirSync(mount);
    directory.readSync();
    return "ready";
  } catch (error) {
    return error.code === "EACCES" || error.code === "EPERM"
      ? "permission_denied"
      : "unavailable";
  } finally {
    if (directory) directory.closeSync();
  }
}

// Mirrors the current Rust source-admission policy. System-root observations
// are reported separately, not silently excluded, so real evidence can reveal
// an admission defect. No identifier, mount string or capacity is returned.
export function admission(info, listIdentifier, listMount, probeRoot = rootReadAccess) {
  const observations = {
    mountIsSystemRoot: mountIsSystemRoot(listMount),
    mountIsAbsolute: path.posix.isAbsolute(listMount),
    identifierMatchesList: info.DeviceIdentifier === listIdentifier,
    mountMatchesList: info.MountPoint === listMount,
    hasUnsignedCapacity: Number.isInteger(info.TotalSize) && info.TotalSize >= 0,
    hasIdentifierEvidence: typeof info.DeviceIdentifier === "string",
    hasInternalEvidence: typeof info.Internal === "boolean",
    hasRemovableOrExternalEvidence: [
      "RemovableMediaOrExternalDevice",
      "RemovableMediaOrExternal",
      "RemovableMedia",
      "Ejectable",
    ].some((key) => info[key] === true),
    explicitlyVirtual:
      typeof info.VirtualOrPhysical === "string" &&
      info.VirtualOrPhysical.toLowerCase() === "virtual",
  };
  let reason;
  if (!observations.hasUnsignedCapacity) reason = "missingCapacity";
  else if (!observations.hasIdentifierEvidence) reason = "missingIdentifierEvidence";
  else if (!observations.hasInternalEvidence) reason = "missingInternalEvidence";
  else if (!observations.identifierMatchesList) reason = "identifierChanged";
  else if (!observations.mountMatchesList) reason = "mountChanged";
  else if (observations.explicitlyVirtual) reason = "explicitVirtual";
  else if (!observations.hasRemovableOrExternalEvidence)
    reason = "notRemovableOrExternal";
  else if (info.Internal) reason = "internal";
  else if (!observations.mountIsAbsolute) reason = "relativeMount";
  else {
    const readAccess = probeRoot(listMount);
    reason =
      readAccess === "ready"
        ? "admitted"
        : readAccess === "permission_denied"
          ? "rootPermissionDenied"
          : "rootUnavailable";
  }
  return { observations, reason };
}

export function collectDiscoveryDiagnostics({
  run = native,
  probeRoot = rootReadAccess,
} = {}) {
  const report = {
    complete: true,
    counts: {
      mountedEntries: 0,
      duplicateMounts: 0,
      examinedCandidates: 0,
      admittedCandidates: 0,
      admittedSystemRoots: 0,
    },
    reasonCounts: {},
    candidates: [],
  };
  const countReason = (reason) => {
    report.reasonCounts[reason] = (report.reasonCounts[reason] ?? 0) + 1;
  };
  const root = plist(["list", "-plist"], run);
  if (!root) {
    report.complete = false;
    countReason("listUnavailable");
    return report;
  }
  const seenMounts = new Set();
  for (const entry of diskEntries(root)) {
    if (typeof entry.MountPoint !== "string") continue;
    report.counts.mountedEntries++;
    if (seenMounts.has(entry.MountPoint)) {
      report.counts.duplicateMounts++;
      continue;
    }
    seenMounts.add(entry.MountPoint);
    report.counts.examinedCandidates++;
    const identifier = entry.DeviceIdentifier;
    if (typeof identifier !== "string" || !/^disk[A-Za-z0-9]*$/.test(identifier)) {
      countReason("invalidListIdentifier");
      continue;
    }
    const info = plist(["info", "-plist", identifier], run);
    if (!info) {
      report.complete = false;
      countReason("infoUnavailable");
      continue;
    }
    const decision = admission(info, identifier, entry.MountPoint, probeRoot);
    report.candidates.push({ ...safeProperties(info), ...decision });
    countReason(decision.reason);
    if (decision.reason === "admitted") {
      report.counts.admittedCandidates++;
      if (decision.observations.mountIsSystemRoot) report.counts.admittedSystemRoots++;
    }
  }
  return report;
}

export function assertOutputOutsideRepository(output, checkout = repository) {
  const resolved = path.resolve(output);
  const relative = path.relative(realpathSync(checkout), resolved);
  if (
    !relative ||
    (!relative.startsWith(`..${path.sep}`) &&
      relative !== ".." &&
      !path.isAbsolute(relative))
  ) {
    throw new Error("Diagnostic output must stay outside the checkout.");
  }
  // Resolve existing parent symlinks before creating an output directory.
  let parent = path.dirname(resolved);
  const missing = [];
  while (true) {
    try {
      lstatSync(parent);
      break;
    } catch (error) {
      if (error.code !== "ENOENT")
        // eslint-disable-next-line preserve-caught-error -- Native causes can contain private paths; expose only fixed sanitized text.
        throw new Error("Diagnostic output directory is unavailable.");
      missing.unshift(path.basename(parent));
      parent = path.dirname(parent);
    }
  }
  const canonical = path.join(
    realpathSync(parent),
    ...missing,
    path.basename(resolved),
  );
  const canonicalRelative = path.relative(realpathSync(checkout), canonical);
  if (
    !canonicalRelative ||
    (!canonicalRelative.startsWith(`..${path.sep}`) &&
      canonicalRelative !== ".." &&
      !path.isAbsolute(canonicalRelative))
  ) {
    throw new Error("Diagnostic output must stay outside the checkout.");
  }
  return resolved;
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  try {
    if (process.platform !== "darwin") throw new Error("Native macOS is required.");
    const output = assertOutputOutsideRepository(
      process.argv[2] ?? path.join(os.tmpdir(), "mit-macos-discovery-diagnostics.json"),
    );
    const report = collectDiscoveryDiagnostics();
    const json = `${JSON.stringify(report, null, 2)}\n`;
    mkdirSync(path.dirname(output), { recursive: true });
    writeFileSync(output, json, { flag: "wx" });
    process.stdout.write(json);
    if (!report.complete) process.exitCode = 1;
  } catch {
    // Fixed failure text only: never print a Node/native error containing a
    // requested path, invocation arguments, plist output or volume details.
    process.stdout.write('{"complete":false,"reasonCounts":{"diagnosticFailure":1}}\n');
    process.exitCode = 1;
  }
}

import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { mkdtempSync, rmdirSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  admission,
  assertOutputOutsideRepository,
  collectDiscoveryDiagnostics,
  safeProperties,
} from "./macos-discovery-diagnostics.mjs";

test("diagnostics retain schema and booleans while excluding private values", () => {
  const info = {
    VolumeName: "PRIVATE_VOLUME_NAME",
    MountPoint: "/Volumes/PRIVATE_MOUNT_NAME",
    DeviceIdentifier: "disk987s2",
    VolumeUUID: "PRIVATE_VOLUME_UUID",
    DeviceSerial: "PRIVATE_DEVICE_SERIAL",
    TotalSize: 4398046511104,
    Internal: false,
    WritableVolume: false,
    FilesystemType: "exfat",
    BusProtocol: "USB",
    VirtualOrPhysical: "Physical",
    MediaName: "PRIVATE_MEDIA_NAME",
  };
  const result = safeProperties(info);
  assert.equal(result.filesystemType, "exfat");
  assert.equal(result.busProtocol, "USB");
  assert.equal(result.virtualOrPhysical, "physical");
  assert.deepEqual(result.booleanProperties, {
    Internal: false,
    WritableVolume: false,
  });
  assert.ok(result.propertyKeys.includes("VolumeUUID"));
  const serialized = JSON.stringify(result);
  for (const value of Object.values(info).filter(
    (value) => typeof value === "string",
  )) {
    if (
      value.startsWith("PRIVATE_") ||
      value.startsWith("/Volumes/") ||
      value.startsWith("disk")
    )
      assert.ok(!serialized.includes(value));
  }
  assert.ok(!serialized.includes("4398046511104"));
  assert.equal(
    safeProperties({ FilesystemType: "/PRIVATE/ENUM", BusProtocol: "PRIVATE_PROTOCOL" })
      .filesystemType,
    "other",
  );
});

test("admission mirrors source guards without requiring a writable source", () => {
  const info = {
    TotalSize: 1024,
    DeviceIdentifier: "disk4s1",
    MountPoint: "/Volumes/PRIVATE_CARD",
    Internal: false,
    RemovableMediaOrExternalDevice: true,
    WritableVolume: false,
  };
  assert.equal(
    admission(info, "disk4s1", info.MountPoint, () => "ready").reason,
    "admitted",
  );
  assert.equal(
    admission(info, "disk4s1", info.MountPoint, () => "permission_denied").reason,
    "rootPermissionDenied",
  );
  assert.equal(
    admission({ ...info, VirtualOrPhysical: "Virtual" }, "disk4s1", info.MountPoint)
      .reason,
    "explicitVirtual",
  );
  assert.equal(
    admission({ ...info, BusProtocol: "Disk Image" }, "disk4s1", info.MountPoint)
      .reason,
    "diskImageProtocol",
  );
  assert.equal(
    admission({ ...info, Internal: true }, "disk4s1", info.MountPoint).reason,
    "internal",
  );
  assert.equal(admission(info, "disk5s1", info.MountPoint).reason, "identifierChanged");
  assert.equal(
    admission(info, "disk4s1", "/Volumes/PRIVATE_CHANGED").reason,
    "mountChanged",
  );
  const withoutInternal = { ...info };
  delete withoutInternal.Internal;
  assert.equal(
    admission(withoutInternal, "disk4s1", info.MountPoint).reason,
    "missingInternalEvidence",
  );
});

test("collector reports an admitted system root rather than assuming an empty runner", () => {
  const list = {
    AllDisksAndPartitions: [
      {
        DeviceIdentifier: "disk4",
        Partitions: [
          { DeviceIdentifier: "disk4s1", MountPoint: "/" },
          { DeviceIdentifier: "disk4s2", MountPoint: "/Volumes/PRIVATE_CARD" },
          { DeviceIdentifier: "disk4s2", MountPoint: "/Volumes/PRIVATE_CARD" },
        ],
      },
    ],
  };
  const run = (command, args, input) => {
    if (command === "/usr/bin/plutil") {
      assert.deepEqual(args, ["-convert", "json", "-o", "-", "-"]);
      return { okay: true, stdout: input };
    }
    assert.equal(command, "/usr/sbin/diskutil");
    assert.ok(args[0] === "list" || args[0] === "info", "only read-only utility verbs");
    const value =
      args[0] === "list"
        ? list
        : {
            DeviceIdentifier: args[2],
            MountPoint: args[2] === "disk4s1" ? "/" : "/Volumes/PRIVATE_CARD",
            TotalSize: 1024,
            Internal: false,
            RemovableMediaOrExternalDevice: true,
            FilesystemType: "apfs",
            BusProtocol: "USB",
            VolumeName: "PRIVATE_VOLUME_NAME",
            VolumeUUID: "PRIVATE_UUID",
          };
    return { okay: true, stdout: Buffer.from(JSON.stringify(value)) };
  };
  const result = collectDiscoveryDiagnostics({ run, probeRoot: () => "ready" });
  assert.equal(result.counts.examinedCandidates, 2);
  assert.equal(result.counts.duplicateMounts, 1);
  assert.equal(result.counts.admittedCandidates, 2);
  assert.equal(result.counts.admittedSystemRoots, 1);
  assert.equal(result.candidates[0].observations.mountIsSystemRoot, true);
  const serialized = JSON.stringify(result);
  assert.ok(!serialized.includes("PRIVATE_"));
  assert.ok(!serialized.includes("disk4"));
  assert.ok(!serialized.includes("/Volumes/"));
});

test("native failure produces fixed reasons and never copied exception text", () => {
  const report = collectDiscoveryDiagnostics({
    run: () => {
      throw new Error("PRIVATE_MOUNT /Volumes/secret disk9");
    },
  });
  assert.equal(report.complete, false);
  assert.deepEqual(report.reasonCounts, { listUnavailable: 1 });
  assert.ok(!JSON.stringify(report).includes("PRIVATE_MOUNT"));
});

test("diagnostic artifact cannot be written inside the checkout", () => {
  assert.throws(
    () => assertOutputOutsideRepository(path.resolve("discovery.json")),
    /outside/,
  );
  const root = mkdtempSync(path.join(os.tmpdir(), "mit-safe-discovery-"));
  try {
    const output = path.join(root, "subdirectory", "discovery.json");
    assert.equal(assertOutputOutsideRepository(output), output);
  } finally {
    // Only the exact newly created task-owned empty directory is removed.
    rmdirSync(root);
  }
});

import assert from "node:assert/strict";
import test from "node:test";
import { releaseMetadata, validateVersions } from "./release-metadata.mjs";

const sourceCommit = "a".repeat(40);
const base = { sourceCommit, version: "0.1.0" };

test("nightly tag uses Singapore calendar date and exact source snapshot", () => {
  const metadata = releaseMetadata({
    ...base,
    channel: "nightly",
    date: new Date("2026-10-09T18:17:00Z"),
  });
  assert.equal(metadata.tag, "nightly-20261010-aaaaaaaaaaaa");
  assert.equal(metadata.prerelease, true);
  assert.equal(metadata.sourceCommit, sourceCommit);
});
test("stable and prerelease tags follow checked-in SemVer", () => {
  assert.equal(
    releaseMetadata({ ...base, channel: "release", tag: "v0.1.0" }).prerelease,
    false,
  );
  assert.equal(
    releaseMetadata({
      ...base,
      version: "1.0.0-rc.1",
      channel: "release",
      tag: "v1.0.0-rc.1",
    }).prerelease,
    true,
  );
});
test("wrong tags and malformed source/version are rejected", () => {
  for (const override of [
    { tag: "v0.2.0" },
    { tag: "main" },
    { sourceCommit: "main" },
    { version: "01.0.0" },
    { version: "1.0.0-01" },
    { version: "1.0.0+build.1" },
    { channel: "unknown" },
  ]) {
    assert.throws(() =>
      releaseMetadata({ ...base, channel: "release", tag: "v0.1.0", ...override }),
    );
  }
});
test("all checked-in application versions must agree", () => {
  const versions = {
    packageVersion: "0.1.0",
    lockVersion: "0.1.0",
    lockedPackageVersion: "0.1.0",
    tauriVersion: "0.1.0",
    cargoVersion: "0.1.0",
  };
  assert.equal(validateVersions(versions), "0.1.0");
  for (const field of Object.keys(versions)) {
    assert.throws(() => validateVersions({ ...versions, [field]: "0.2.0" }));
    assert.throws(() => validateVersions({ ...versions, [field]: undefined }));
  }
});

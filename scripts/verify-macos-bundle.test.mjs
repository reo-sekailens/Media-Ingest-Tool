import assert from "node:assert/strict";
import path from "node:path";
import process from "node:process";
import os from "node:os";
import { mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import test from "node:test";
import {
  assertPlist,
  assertArchitecture,
  assertBundledDependency,
  assertSignature,
  contained,
  deploymentVersions,
  linkedLibraries,
  isSystemLibrary,
  PRIVACY_KEYS,
  verifyBundle,
  versionAtMost,
} from "./verify-macos-bundle.mjs";

const config = { identifier: "com.mediaingest.tool", version: "0.1.0" };
const macConfig = { bundle: { macOS: { minimumSystemVersion: "13.3" } } };
const plist = {
  CFBundleIdentifier: config.identifier,
  CFBundleShortVersionString: config.version,
  CFBundlePackageType: "APPL",
  CFBundleExecutable: "media-ingest-tool",
  CFBundleIconFile: "icon.icns",
  LSMinimumSystemVersion: "13.3",
  ...Object.fromEntries(
    PRIVACY_KEYS.map((key) => [key, "Read source or destination media."]),
  ),
};

test("minimum version comparison does not confuse 13.10 with 13.3", () => {
  assert.equal(versionAtMost("13.3.0", "13.3"), true);
  assert.equal(versionAtMost("13.2.1", "13.3"), true);
  assert.equal(versionAtMost("13.10", "13.3"), false);
  assert.equal(versionAtMost("14.0", "13.3"), false);
  assert.throws(() => versionAtMost("13", "13.3"));
  assert.throws(() => versionAtMost("not-a-version", "13.3"));
});

test("reads modern and legacy Mach-O deployment targets and rejects non-mac platforms", () => {
  const modern = `binary:\nLoad command 1\n cmd LC_BUILD_VERSION\n platform 1\n minos 13.3\n sdk 26.0\nLoad command 2\n cmd LC_LOAD_DYLIB`;
  assert.deepEqual(deploymentVersions(modern), ["13.3"]);
  assert.deepEqual(deploymentVersions(modern.replace("platform 1", "platform macOS")), [
    "13.3",
  ]);
  assert.deepEqual(
    deploymentVersions(
      "binary:\nLoad command 0\n cmd LC_VERSION_MIN_MACOSX\n version 10.13\n sdk 15.0",
    ),
    ["10.13"],
  );
  assert.throws(
    () => deploymentVersions(modern.replace("platform 1", "platform 2")),
    /other than macOS/,
  );
  assert.throws(() => deploymentVersions(modern.replace("minos 13.3", "")), /Missing/);
  assert.throws(
    () => deploymentVersions("binary:\nLoad command 0\n cmd LC_LOAD_DYLIB"),
    /Missing/,
  );
});

test("library parser preserves spaces and rejects unrecognized output", () => {
  assert.deepEqual(
    linkedLibraries(
      `binary:\n\t/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit (compatibility version 1.0.0, current version 619.1.26)\n\t@rpath/My Library.dylib (compatibility version 1.0.0, current version 1.0.0)`,
    ),
    [
      "/System/Library/Frameworks/WebKit.framework/Versions/A/WebKit",
      "@rpath/My Library.dylib",
    ],
  );
  assert.throws(() => linkedLibraries("binary:\n unexpected output"));
});

test("plist gate requires each privacy string, exact identity/minimum, and local executable", () => {
  assert.doesNotThrow(() => assertPlist(plist, config, macConfig));
  for (const key of PRIVACY_KEYS) {
    assert.throws(
      () => assertPlist({ ...plist, [key]: " " }, config, macConfig),
      /privacy/,
    );
    assert.throws(
      () => assertPlist({ ...plist, [key]: undefined }, config, macConfig),
      /privacy/,
    );
  }
  for (const change of [
    { CFBundleIdentifier: "wrong.id" },
    { CFBundleShortVersionString: "0.2.0" },
    { CFBundlePackageType: "BNDL" },
    { LSMinimumSystemVersion: "13.0" },
    { CFBundleExecutable: "../../outside" },
    { CFBundleIconFile: "" },
  ])
    assert.throws(() => assertPlist({ ...plist, ...change }, config, macConfig));
});

test("signature gate distinguishes valid ad-hoc metadata from Developer ID and missing runtime", () => {
  const adhoc =
    "CodeDirectory v=20500 size=123 flags=0x10002(adhoc,runtime) hashes=10+7 location=embedded\nSignature=adhoc\nTeamIdentifier=not set";
  assert.doesNotThrow(() => assertSignature(adhoc, true));
  assert.throws(
    () => assertSignature(adhoc.replace("adhoc,runtime", "adhoc"), true),
    /runtime/,
  );
  assert.throws(
    () =>
      assertSignature(
        "Signature=adhoc\nAuthority=Developer ID Application: Test",
        false,
      ),
    /ad-hoc/,
  );
  assert.throws(() => assertSignature("Signature size=9000", false), /ad-hoc/);
});

test("path containment excludes prefix siblings, parents, and different absolute roots", () => {
  const root = path.resolve("fixture", "App.app");
  assert.equal(contained(root, path.join(root, "Contents", "MacOS", "app")), true);
  assert.equal(contained(root, root), true);
  assert.equal(contained(root, `${root}-other`), false);
  assert.equal(contained(root, path.dirname(root)), false);
});

test("architecture gate rejects wrong native architecture and accidental universal main executable", () => {
  assert.doesNotThrow(() => assertArchitecture(["arm64"], "arm64", true));
  assert.doesNotThrow(() => assertArchitecture(["x86_64", "arm64"], "arm64", false));
  assert.throws(() => assertArchitecture(["x86_64"], "arm64", true));
  assert.throws(() => assertArchitecture(["x86_64", "arm64"], "arm64", true));
});

test("dependency gate allows real bundle files while excluding host-installed and unresolved libraries", async (t) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "media-ingest-library-test-")),
  );
  t.after(() => rm(root, { recursive: true, force: true }));
  const bundle = path.join(root, "App.app");
  const library = path.join(bundle, "Contents", "Frameworks", "Example.dylib");
  await mkdir(path.dirname(library), { recursive: true });
  await writeFile(library, "fixture");
  await assert.doesNotReject(
    assertBundledDependency("@rpath/Example.dylib", [library], bundle),
  );
  await assert.rejects(
    assertBundledDependency(
      "/opt/homebrew/lib/Example.dylib",
      ["/opt/homebrew/lib/Example.dylib"],
      bundle,
    ),
  );
  await assert.rejects(
    assertBundledDependency(
      "@rpath/Missing.dylib",
      [path.join(bundle, "Missing.dylib")],
      bundle,
    ),
  );
  assert.equal(isSystemLibrary("/usr/lib/libSystem.B.dylib"), true);
  assert.equal(
    isSystemLibrary("/System/Library/Frameworks/WebKit.framework/WebKit"),
    true,
  );
  assert.equal(isSystemLibrary("/usr/lib/../../opt/homebrew/lib/Example.dylib"), false);
});

test(
  "Windows/Linux cannot fabricate native macOS package evidence",
  { skip: process.platform === "darwin" },
  async () => {
    await assert.rejects(
      verifyBundle("missing", "aarch64-apple-darwin", "unused.json"),
      /requires macOS/,
    );
  },
);

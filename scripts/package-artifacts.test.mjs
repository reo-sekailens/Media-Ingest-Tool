import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { promisify } from "node:util";
import { TARGET_FORMATS } from "./package-artifacts.mjs";

const execute = promisify(execFile);
const script = fileURLToPath(new URL("./package-artifacts.mjs", import.meta.url));
const sourceCommit = "1234567890abcdef1234567890abcdef12345678";
const targets = Object.keys(TARGET_FORMATS);

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "media-ingest-packages-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

function cli(args, commit = sourceCommit) {
  return execute(process.execPath, [script, ...args], {
    env: { ...process.env, GITHUB_SHA: commit },
    timeout: 15000,
  });
}

async function bundle(root, target) {
  const directory = path.join(root, `bundle-${target}`);
  await mkdir(path.join(directory, "nested packages"), { recursive: true });
  for (const extension of TARGET_FORMATS[target]) {
    await writeFile(
      path.join(directory, "nested packages", `Media Ingest Tool_0.1.0${extension}`),
      `package bytes for ${target}${extension}`,
    );
  }
  return directory;
}

async function downloadedPackages(root) {
  const download = path.join(root, "download");
  await mkdir(download);
  for (const target of targets) {
    await cli([
      "stage",
      await bundle(root, target),
      path.join(download, `packages-${target}`),
      target,
    ]);
  }
  return download;
}

async function alterManifest(download, target, change) {
  const file = path.join(download, `packages-${target}`, "artifact-manifest.json");
  const manifest = JSON.parse(await readFile(file, "utf8"));
  change(manifest);
  await writeFile(file, JSON.stringify(manifest));
}

test("stage requires every package format and writes known digests for all five targets", async (t) => {
  const root = await fixture(t);
  for (const target of targets) {
    const output = path.join(root, `staged-${target}`);
    await cli(["stage", await bundle(root, target), output, target]);
    const manifest = JSON.parse(
      await readFile(path.join(output, "artifact-manifest.json"), "utf8"),
    );
    assert.equal(manifest.sourceCommit, sourceCommit);
    assert.equal(manifest.target, target);
    assert.equal(manifest.schemaVersion, 1);
    assert.deepEqual(Object.keys(manifest.versions).sort(), [
      "application",
      "node",
      "rust",
    ]);
    assert.equal(manifest.artifacts.length, TARGET_FORMATS[target].length);
    const checksums = await readFile(path.join(output, "SHA256SUMS"), "utf8");
    for (const artifact of manifest.artifacts) {
      assert.match(artifact.file, /^[a-zA-Z0-9._-]+$/);
      assert.ok(artifact.file.startsWith(`${target}-`));
      const bytes = await readFile(path.join(output, artifact.file));
      assert.equal(artifact.bytes, bytes.length);
      assert.equal(artifact.sha256, createHash("sha256").update(bytes).digest("hex"));
      assert.ok(checksums.includes(`${artifact.sha256}  ${artifact.file}\n`));
    }
  }
});

test("stage rejects missing required formats before writing outputs", async (t) => {
  const root = await fixture(t);
  const input = await bundle(root, targets[0]);
  await rm(path.join(input, "nested packages", "Media Ingest Tool_0.1.0.msi"));
  const output = path.join(root, "output");
  await assert.rejects(
    cli(["stage", input, output, targets[0]]),
    /Missing required .msi/,
  );
  await assert.rejects(readdir(output), { code: "ENOENT" });
});

test("stage rejects malformed source commits and unsupported targets", async (t) => {
  const root = await fixture(t);
  const input = await bundle(root, targets[0]);
  for (const commit of ["", "main", "a".repeat(39), "g".repeat(40)]) {
    await assert.rejects(
      cli(["stage", input, path.join(root, "output"), targets[0]], commit),
      /GITHUB_SHA/,
    );
  }
  await assert.rejects(
    cli(["stage", input, path.join(root, "output"), "unsupported"]),
    /Unsupported package target/,
  );
});

test("stage detects normalized filename collisions and preserves existing outputs", async (t) => {
  const root = await fixture(t);
  const input = await bundle(root, targets[0]);
  await writeFile(path.join(input, "a b.exe"), "one");
  await writeFile(path.join(input, "a_b.exe"), "two");
  const output = path.join(root, "output");
  await assert.rejects(cli(["stage", input, output, targets[0]]), /filename collision/);
  await rm(path.join(input, "a_b.exe"));
  await mkdir(output);
  await writeFile(path.join(output, "keep.txt"), "preserve this");
  await assert.rejects(
    cli(["stage", input, output, targets[0]]),
    /Output directory must be empty/,
  );
  assert.equal(await readFile(path.join(output, "keep.txt"), "utf8"), "preserve this");
});

test("stage rejects source and destination symlinks and overlapping directories", async (t) => {
  const root = await fixture(t);
  const input = await bundle(root, targets[0]);
  const linkedInput = path.join(root, "linked-input");
  await symlink(input, linkedInput, process.platform === "win32" ? "junction" : "dir");
  await assert.rejects(
    cli(["stage", linkedInput, path.join(root, "output"), targets[0]]),
    /symlink/,
  );
  const linkedOutput = path.join(root, "linked-output");
  const output = path.join(root, "empty-output");
  await mkdir(output);
  await symlink(
    output,
    linkedOutput,
    process.platform === "win32" ? "junction" : "dir",
  );
  await assert.rejects(cli(["stage", input, linkedOutput, targets[0]]), /symlink/);
  await assert.rejects(
    cli(["stage", input, path.join(input, "output"), targets[0]]),
    /must not overlap/,
  );
  await assert.rejects(
    cli(["stage", input, path.join(input, "new", "output"), targets[0]]),
    /must not overlap/,
  );
  await assert.rejects(readdir(path.join(input, "new")), { code: "ENOENT" });
  await assert.rejects(cli(["stage", input, root, targets[0]]), /must not overlap/);
  await symlink(
    output,
    path.join(input, "linked-tree"),
    process.platform === "win32" ? "junction" : "dir",
  );
  await assert.rejects(
    cli(["stage", input, path.join(root, "other-output"), targets[0]]),
    /contains a symlink/,
  );
});

test("stage excludes macOS application resources from installer staging", async (t) => {
  const root = await fixture(t);
  const target = "x86_64-apple-darwin";
  const input = await bundle(root, target);
  await mkdir(path.join(input, "Media Ingest Tool.app", "Contents"), {
    recursive: true,
  });
  await writeFile(
    path.join(input, "Media Ingest Tool.app", "Contents", "not-an-installer.dmg"),
    "resource",
  );
  const output = path.join(root, "output");
  await cli(["stage", input, output, target]);
  const manifest = JSON.parse(
    await readFile(path.join(output, "artifact-manifest.json"), "utf8"),
  );
  assert.equal(manifest.artifacts.length, 1);
});

test("collect verifies all targets and produces a complete global manifest and checksums", async (t) => {
  const root = await fixture(t);
  const download = await downloadedPackages(root);
  const output = path.join(root, "release");
  await cli(["collect", download, output]);
  const manifest = JSON.parse(
    await readFile(path.join(output, "artifact-manifest.json"), "utf8"),
  );
  assert.equal(manifest.sourceCommit, sourceCommit);
  assert.deepEqual(manifest.targets, targets);
  assert.equal(manifest.artifacts.length, 10);
  const sums = await readFile(path.join(output, "SHA256SUMS"), "utf8");
  assert.equal(sums.trimEnd().split("\n").length, 10);
  assert.equal((await readdir(output)).length, 12);
  for (const artifact of manifest.artifacts) {
    const contents = await readFile(path.join(output, artifact.file));
    assert.equal(artifact.sha256, createHash("sha256").update(contents).digest("hex"));
    assert.ok(TARGET_FORMATS[artifact.target].includes(path.extname(artifact.file)));
  }
});

test("collect rejects a missing target or a different source commit", async (t) => {
  const root = await fixture(t);
  const download = await downloadedPackages(root);
  const output = path.join(root, "release");
  await alterManifest(download, targets[0], (manifest) => {
    manifest.sourceCommit = "f".repeat(40);
  });
  await assert.rejects(cli(["collect", download, output]), /provenance or versions/);
  await assert.rejects(readdir(output), { code: "ENOENT" });
  await rm(path.join(download, `packages-${targets[0]}`), { recursive: true });
  await assert.rejects(cli(["collect", download, output]), /all five/);
});

test("collect rejects tampered artifact bytes, manifest sizes, and toolchain policy", async (t) => {
  const root = await fixture(t);
  const download = await downloadedPackages(root);
  const target = targets[0];
  const directory = path.join(download, `packages-${target}`);
  const manifest = JSON.parse(
    await readFile(path.join(directory, "artifact-manifest.json"), "utf8"),
  );
  const artifact = path.join(directory, manifest.artifacts[0].file);
  const original = await readFile(artifact);
  await writeFile(artifact, Buffer.alloc(original.length, 120));
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /digest or size mismatch/,
  );
  await writeFile(artifact, original);
  await alterManifest(download, target, (value) => {
    value.artifacts[0].bytes += 1;
  });
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /digest or size mismatch/,
  );
  await alterManifest(download, target, (value) => {
    value.artifacts[0].bytes -= 1;
    value.versions.rust = "0.0.0";
  });
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /provenance or versions/,
  );
});

test("collect rejects unsafe manifest paths, duplicate entries, missing formats, and extra files", async (t) => {
  const root = await fixture(t);
  const download = await downloadedPackages(root);
  const target = targets[0];
  const directory = path.join(download, `packages-${target}`);
  const file = path.join(directory, "artifact-manifest.json");
  const original = await readFile(file, "utf8");
  await alterManifest(download, target, (manifest) => {
    manifest.artifacts[0].file = "../escape.exe";
  });
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /Unsafe, incomplete/,
  );
  await writeFile(file, original);
  await alterManifest(download, target, (manifest) => {
    manifest.artifacts.push(manifest.artifacts[0]);
  });
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /Unsafe, incomplete/,
  );
  await writeFile(file, original);
  await alterManifest(download, target, (manifest) => {
    manifest.artifacts = manifest.artifacts.filter(
      (artifact) => path.extname(artifact.file) !== ".msi",
    );
  });
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /Missing required .msi/,
  );
  await writeFile(file, original);
  await writeFile(path.join(directory, "unlisted.exe"), "not covered by manifest");
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /unlisted or missing files/,
  );
});

test("collect rejects checksum changes and symlinked target directories", async (t) => {
  const root = await fixture(t);
  const download = await downloadedPackages(root);
  const directory = path.join(download, `packages-${targets[0]}`);
  const sumsFile = path.join(directory, "SHA256SUMS");
  const original = await readFile(sumsFile, "utf8");
  await writeFile(sumsFile, original.replace(/^[a-f0-9]{64}/, "0".repeat(64)));
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /Unsafe, incomplete/,
  );
  await rm(directory, { recursive: true });
  const substitute = path.join(root, "substitute");
  await mkdir(substitute);
  await symlink(
    substitute,
    directory,
    process.platform === "win32" ? "junction" : "dir",
  );
  await assert.rejects(
    cli(["collect", download, path.join(root, "release")]),
    /symlink/,
  );
});

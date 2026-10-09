import { createHash } from "node:crypto";
import { constants, createWriteStream } from "node:fs";
import {
  lstat,
  mkdir,
  open,
  readFile,
  readdir,
  realpath,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { Transform } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath, URL } from "node:url";

export const TARGET_FORMATS = Object.freeze({
  "x86_64-pc-windows-msvc": [".exe", ".msi"],
  "x86_64-unknown-linux-gnu": [".deb", ".rpm", ".AppImage"],
  "aarch64-unknown-linux-gnu": [".deb", ".rpm", ".AppImage"],
  "x86_64-apple-darwin": [".dmg"],
  "aarch64-apple-darwin": [".dmg"],
});

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const safeName = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,199}$/;
const checksumPattern = /^[a-f0-9]{64}$/;
const metadataLimit = 1024 * 1024;

function sourceCommitFromEnvironment() {
  const commit = process.env.GITHUB_SHA;
  if (!commit || !/^[a-f0-9]{40}$/i.test(commit)) {
    throw new Error(
      "GITHUB_SHA must identify the immutable 40-character source commit.",
    );
  }
  return commit.toLowerCase();
}

async function repositoryVersions() {
  const [packageJson, tauriJson, cargo, node, rust] = await Promise.all([
    readFile(path.join(repositoryRoot, "package.json"), "utf8"),
    readFile(path.join(repositoryRoot, "src-tauri/tauri.conf.json"), "utf8"),
    readFile(path.join(repositoryRoot, "src-tauri/Cargo.toml"), "utf8"),
    readFile(path.join(repositoryRoot, ".node-version"), "utf8"),
    readFile(path.join(repositoryRoot, "rust-toolchain.toml"), "utf8"),
  ]);
  const application = JSON.parse(packageJson).version;
  const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  const rustVersion = rust.match(/^channel\s*=\s*"([^"]+)"/m)?.[1];
  const versions = { application, node: node.trim(), rust: rustVersion };
  if (
    Object.values(versions).some(
      (version) =>
        typeof version !== "string" ||
        !/^\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?$/.test(version),
    ) ||
    application !== JSON.parse(tauriJson).version ||
    application !== cargoVersion
  ) {
    throw new Error(
      "Application and pinned toolchain versions are missing or inconsistent.",
    );
  }
  return versions;
}

async function checkedDirectory(directory) {
  const resolved = path.resolve(directory);
  const info = await lstat(resolved);
  if (!info.isDirectory() || info.isSymbolicLink()) {
    throw new Error(`Expected a real directory, not a symlink: ${resolved}`);
  }
  return realpath(resolved);
}

async function checkedFile(file) {
  const info = await lstat(file);
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error(`Expected a regular file, not a symlink: ${file}`);
  }
  if (!Number.isSafeInteger(info.size))
    throw new Error("Artifact size exceeds safe accounting limits.");
  return info;
}

function sameObservation(before, after) {
  return (
    after.isFile() &&
    !after.isSymbolicLink() &&
    before.dev === after.dev &&
    before.ino === after.ino &&
    before.size === after.size &&
    before.mtimeMs === after.mtimeMs &&
    before.ctimeMs === after.ctimeMs
  );
}

// Hold one read handle, account for every streamed byte, and reject a changed
// source. Copying uses exclusive creation so an existing output is never replaced.
async function streamArtifact(source, destination) {
  const before = await checkedFile(source);
  const handle = await open(source, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    if (!sameObservation(before, await handle.stat()))
      throw new Error("Artifact changed before reading.");
    const digest = createHash("sha256");
    let bytes = 0;
    const reader = handle.createReadStream({ autoClose: false });
    if (destination) {
      const hashing = new Transform({
        transform(chunk, _encoding, callback) {
          digest.update(chunk);
          bytes += chunk.length;
          callback(null, chunk);
        },
      });
      await pipeline(reader, hashing, createWriteStream(destination, { flags: "wx" }));
    } else {
      for await (const chunk of reader) {
        digest.update(chunk);
        bytes += chunk.length;
      }
    }
    if (
      bytes !== before.size ||
      !sameObservation(before, await handle.stat()) ||
      !sameObservation(before, await checkedFile(source))
    ) {
      throw new Error("Artifact changed during reading.");
    }
    return { bytes, sha256: digest.digest("hex") };
  } finally {
    await handle.close();
  }
}

async function bundleFiles(root, directory = root) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    if (
      entry.name.includes("\\") ||
      Array.from(entry.name).some((character) => {
        const code = character.codePointAt(0);
        return code < 32 || code === 127;
      })
    )
      throw new Error("Unsafe artifact source path.");
    const file = path.join(directory, entry.name);
    const info = await lstat(file);
    if (info.isSymbolicLink()) throw new Error(`Bundle contains a symlink: ${file}`);
    if (info.isDirectory()) {
      // An application bundle is not a distribution package; its internal
      // framework links and resources do not belong in installer staging.
      if (!entry.name.endsWith(".app")) files.push(...(await bundleFiles(root, file)));
    } else if (info.isFile()) {
      files.push(file);
    } else {
      throw new Error(`Bundle contains a non-regular entry: ${file}`);
    }
  }
  return files;
}

function packagedName(target, source) {
  const original = path.basename(source);
  const extension = path.extname(original);
  const stem = original
    .slice(0, -extension.length)
    .normalize("NFKC")
    .replace(/[^a-zA-Z0-9._-]+/g, "_");
  const file = `${target}-${stem}${extension}`;
  if (!safeName.test(file))
    throw new Error("Artifact filename cannot be safely staged.");
  return file;
}

function requireFormats(target, artifacts) {
  for (const extension of TARGET_FORMATS[target]) {
    if (!artifacts.some(({ file }) => path.extname(file) === extension)) {
      throw new Error(`Missing required ${extension} package for ${target}.`);
    }
  }
}

async function emptyOutput(outputDirectory, inputRoot) {
  const output = path.resolve(outputDirectory);
  let existingParent = path.dirname(output);
  for (;;) {
    try {
      await lstat(existingParent);
      break;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      const parent = path.dirname(existingParent);
      if (parent === existingParent) throw error;
      existingParent = parent;
    }
  }
  const resolvedOutput = path.join(
    await checkedDirectory(existingParent),
    path.relative(existingParent, output),
  );
  const relative = path.relative(inputRoot, resolvedOutput);
  const reverse = path.relative(resolvedOutput, inputRoot);
  const contained = (value) =>
    value === "" ||
    (!value.startsWith(`..${path.sep}`) && value !== ".." && !path.isAbsolute(value));
  if (contained(relative) || contained(reverse))
    throw new Error("Input and output directories must not overlap.");
  await mkdir(path.dirname(output), { recursive: true });
  try {
    await mkdir(output);
  } catch (error) {
    if (error.code !== "EEXIST") throw error;
  }
  const canonical = await checkedDirectory(output);
  if (canonical !== resolvedOutput || (await readdir(canonical)).length !== 0) {
    throw new Error(
      "Output directory must be empty and must not redirect through a symlink.",
    );
  }
  return canonical;
}

async function writeMetadata(output, manifest) {
  await writeFile(
    path.join(output, "artifact-manifest.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
    { flag: "wx" },
  );
  await writeFile(
    path.join(output, "SHA256SUMS"),
    `${manifest.artifacts.map(({ sha256, file }) => `${sha256}  ${file}`).join("\n")}\n`,
    { flag: "wx" },
  );
}

async function stage(bundleDirectory, outputDirectory, target, sourceCommit) {
  if (!Object.hasOwn(TARGET_FORMATS, target))
    throw new Error(`Unsupported package target: ${target}`);
  const root = await checkedDirectory(bundleDirectory);
  const versions = await repositoryVersions();
  const sources = (await bundleFiles(root))
    .filter((file) => TARGET_FORMATS[target].includes(path.extname(file)))
    .sort();
  const names = new Set();
  const planned = sources.map((source) => {
    const file = packagedName(target, source);
    if (names.has(file.toLowerCase()))
      throw new Error(`Artifact filename collision: ${file}`);
    names.add(file.toLowerCase());
    return { source, file };
  });
  requireFormats(target, planned);
  const output = await emptyOutput(outputDirectory, root);
  const artifacts = [];
  for (const { source, file } of planned) {
    const result = await streamArtifact(source, path.join(output, file));
    if (result.bytes === 0) throw new Error(`Empty package artifact: ${file}`);
    artifacts.push({ file, ...result });
  }
  await writeMetadata(output, {
    schemaVersion: 1,
    sourceCommit,
    target,
    versions,
    artifacts,
  });
}

async function metadataText(file) {
  const info = await checkedFile(file);
  if (info.size > metadataLimit)
    throw new Error("Package metadata exceeds its size limit.");
  return readFile(file, "utf8");
}

async function checkedPackage(root, target, sourceCommit, versions) {
  const directory = await checkedDirectory(path.join(root, `packages-${target}`));
  const manifest = JSON.parse(
    await metadataText(path.join(directory, "artifact-manifest.json")),
  );
  if (
    manifest.schemaVersion !== 1 ||
    manifest.sourceCommit !== sourceCommit ||
    manifest.target !== target ||
    !manifest.versions ||
    Object.keys(versions).some((key) => manifest.versions[key] !== versions[key]) ||
    !Array.isArray(manifest.artifacts) ||
    manifest.artifacts.length === 0
  ) {
    throw new Error(`Package manifest provenance or versions do not match: ${target}`);
  }
  const checksums = new Map();
  const lines = (await metadataText(path.join(directory, "SHA256SUMS")))
    .trimEnd()
    .split(/\r?\n/);
  for (const line of lines) {
    const match = line.match(/^([a-f0-9]{64}) {2}([a-zA-Z0-9][a-zA-Z0-9._-]{0,199})$/);
    if (!match || checksums.has(match[2]))
      throw new Error(`Invalid or duplicate checksum entry: ${target}`);
    checksums.set(match[2], match[1]);
  }
  const names = new Set();
  for (const artifact of manifest.artifacts) {
    if (
      !artifact ||
      typeof artifact.file !== "string" ||
      !safeName.test(artifact.file) ||
      !artifact.file.startsWith(`${target}-`) ||
      !TARGET_FORMATS[target].includes(path.extname(artifact.file)) ||
      !Number.isSafeInteger(artifact.bytes) ||
      artifact.bytes <= 0 ||
      typeof artifact.sha256 !== "string" ||
      !checksumPattern.test(artifact.sha256) ||
      names.has(artifact.file.toLowerCase()) ||
      checksums.get(artifact.file) !== artifact.sha256
    ) {
      throw new Error(`Unsafe, incomplete, or inconsistent artifact entry: ${target}`);
    }
    names.add(artifact.file.toLowerCase());
    const actual = await streamArtifact(path.join(directory, artifact.file));
    if (actual.bytes !== artifact.bytes || actual.sha256 !== artifact.sha256) {
      throw new Error(`Artifact digest or size mismatch: ${artifact.file}`);
    }
  }
  requireFormats(target, manifest.artifacts);
  const entries = await readdir(directory);
  if (
    checksums.size !== manifest.artifacts.length ||
    entries.length !== manifest.artifacts.length + 2 ||
    entries.some(
      (entry) =>
        entry !== "artifact-manifest.json" &&
        entry !== "SHA256SUMS" &&
        !names.has(entry.toLowerCase()),
    )
  ) {
    throw new Error(`Package directory contains unlisted or missing files: ${target}`);
  }
  return manifest.artifacts.map((artifact) => ({
    ...artifact,
    target,
    source: path.join(directory, artifact.file),
  }));
}

async function collect(downloadDirectory, outputDirectory, sourceCommit) {
  const root = await checkedDirectory(downloadDirectory);
  const targets = Object.keys(TARGET_FORMATS);
  const expected = new Set(targets.map((target) => `packages-${target}`));
  const entries = await readdir(root);
  if (
    entries.length !== expected.size ||
    entries.some((entry) => !expected.has(entry))
  ) {
    throw new Error(
      "Collection requires exactly all five packages-<target> directories.",
    );
  }
  const versions = await repositoryVersions();
  const planned = [];
  for (const target of targets)
    planned.push(...(await checkedPackage(root, target, sourceCommit, versions)));
  planned.sort((left, right) => left.file.localeCompare(right.file, "en"));
  const output = await emptyOutput(outputDirectory, root);
  const artifacts = [];
  for (const { source, ...artifact } of planned) {
    const actual = await streamArtifact(source, path.join(output, artifact.file));
    if (actual.bytes !== artifact.bytes || actual.sha256 !== artifact.sha256) {
      throw new Error(`Artifact changed after validation: ${artifact.file}`);
    }
    artifacts.push(artifact);
  }
  await writeMetadata(output, {
    schemaVersion: 1,
    sourceCommit,
    targets,
    versions,
    artifacts,
  });
}

async function main() {
  const [command, input, output, target] = process.argv.slice(2);
  const sourceCommit = sourceCommitFromEnvironment();
  if (command === "stage" && input && output && target && process.argv.length === 6) {
    await stage(input, output, target, sourceCommit);
  } else if (command === "collect" && input && output && process.argv.length === 5) {
    await collect(input, output, sourceCommit);
  } else {
    throw new Error(
      "Usage: package-artifacts.mjs stage <bundle-dir> <output-dir> <target> | collect <download-dir> <output-dir>",
    );
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}

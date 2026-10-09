import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";

const [bundleDirectory] = process.argv.slice(2);
if (!bundleDirectory || process.argv.length !== 3) {
  throw new Error("Usage: node scripts/write-dmg-manifest.mjs <bundle-directory>");
}

const sourceCommit = process.env.GITHUB_SHA;
if (!sourceCommit || !/^[a-f0-9]{40}$/i.test(sourceCommit)) {
  throw new Error("GITHUB_SHA must identify the 40-character source commit.");
}

const bundleRoot = path.resolve(bundleDirectory);
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));

async function findDmgs(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory() && !entry.name.endsWith(".app")) {
      files.push(...(await findDmgs(fullPath)));
    } else if (entry.isFile() && entry.name.endsWith(".dmg")) {
      files.push(fullPath);
    }
  }
  return files;
}

const dmgPaths = (await findDmgs(bundleRoot)).sort();
if (dmgPaths.length === 0) {
  throw new Error("No DMG artifacts found; refusing to write an empty manifest.");
}

const artifacts = [];
for (const dmgPath of dmgPaths) {
  const relativePath = path.relative(bundleRoot, dmgPath).split(path.sep).join("/");
  if (/[\r\n\\]/.test(relativePath)) {
    throw new Error("Artifact paths must be representable in SHA256SUMS.");
  }
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(dmgPath)) digest.update(chunk);
  artifacts.push({
    file: relativePath,
    bytes: (await stat(dmgPath)).size,
    sha256: digest.digest("hex"),
  });
}

const nodePolicy = (
  await readFile(path.join(repositoryRoot, ".node-version"), "utf8")
).trim();
const rustPolicy = await readFile(
  path.join(repositoryRoot, "rust-toolchain.toml"),
  "utf8",
);
const rustChannel = rustPolicy.match(/^channel\s*=\s*"([^"]+)"/m)?.[1];
if (!rustChannel) throw new Error("Repository Rust toolchain channel is missing.");

const manifest = {
  sourceCommit,
  target: process.env.ARTIFACT_TARGET ?? null,
  builder: {
    platform: process.platform,
    architecture: process.arch,
    node: process.version,
  },
  toolchainPolicy: { node: nodePolicy, rust: rustChannel },
  artifacts,
};

await writeFile(
  path.join(bundleRoot, "artifact-manifest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
await writeFile(
  path.join(bundleRoot, "SHA256SUMS"),
  `${artifacts.map(({ sha256, file }) => `${sha256}  ${file}`).join("\n")}\n`,
);

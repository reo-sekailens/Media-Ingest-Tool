import { execFileSync } from "node:child_process";
import { appendFile, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, URL } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const versionPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

export function releaseMetadata({
  channel,
  version,
  tag,
  sourceCommit,
  date = new Date(),
}) {
  if (!/^[a-f0-9]{40}$/.test(sourceCommit)) {
    throw new Error("Release source must be a full lowercase commit SHA.");
  }
  const parsedVersion = versionPattern.exec(version);
  if (
    !parsedVersion ||
    parsedVersion[4]?.split(".").some((part) => /^0\d+$/.test(part))
  ) {
    throw new Error("Application version must be SemVer without build metadata.");
  }
  if (channel === "nightly") {
    const day = new Intl.DateTimeFormat("en-CA", {
      timeZone: "Asia/Singapore",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    })
      .format(date)
      .replaceAll("-", "");
    tag = `nightly-${day}-${sourceCommit.slice(0, 12)}`;
    return {
      tag,
      title: `Nightly ${day} (${sourceCommit.slice(0, 12)})`,
      prerelease: true,
      sourceCommit,
    };
  }
  if (channel !== "release" || tag !== `v${version}`) {
    throw new Error(
      "Release tag must exactly match the checked-in application version (vX.Y.Z).",
    );
  }
  return {
    tag,
    title: `Media Ingest Tool ${version}`,
    prerelease: Boolean(parsedVersion[4]),
    sourceCommit,
  };
}

export function validateVersions({
  packageVersion,
  lockVersion,
  lockedPackageVersion,
  tauriVersion,
  cargoVersion,
}) {
  const versions = [
    packageVersion,
    lockVersion,
    lockedPackageVersion,
    tauriVersion,
    cargoVersion,
  ];
  if (versions.some((value) => typeof value !== "string" || value !== packageVersion)) {
    throw new Error(
      "package.json, package-lock.json, Tauri, and Cargo versions must agree.",
    );
  }
  return packageVersion;
}

async function main() {
  const [command, channel, tagArgument] = process.argv.slice(2);
  if (
    !["resolve", "notes"].includes(command) ||
    !["nightly", "release"].includes(channel)
  ) {
    throw new Error(
      "Usage: node scripts/release-metadata.mjs <resolve|notes> <nightly|release> [tag]",
    );
  }
  const [packageText, lockText, tauriText, cargoText] = await Promise.all([
    readFile(path.join(repositoryRoot, "package.json"), "utf8"),
    readFile(path.join(repositoryRoot, "package-lock.json"), "utf8"),
    readFile(path.join(repositoryRoot, "src-tauri/tauri.conf.json"), "utf8"),
    readFile(path.join(repositoryRoot, "src-tauri/Cargo.toml"), "utf8"),
  ]);
  const lock = JSON.parse(lockText);
  const version = validateVersions({
    packageVersion: JSON.parse(packageText).version,
    lockVersion: lock.version,
    lockedPackageVersion: lock.packages?.[""]?.version,
    tauriVersion: JSON.parse(tauriText).version,
    cargoVersion: cargoText.match(/^version\s*=\s*"([^"]+)"/m)?.[1],
  });
  const sourceCommit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  }).trim();
  const metadata = releaseMetadata({
    channel,
    version,
    tag: tagArgument || process.env.RELEASE_TAG,
    sourceCommit,
  });
  if (command === "resolve") {
    const output = `source-ref=${sourceCommit}\ntag=${metadata.tag}\ntitle=${metadata.title}\nprerelease=${metadata.prerelease}\n`;
    if (process.env.GITHUB_OUTPUT) await appendFile(process.env.GITHUB_OUTPUT, output);
    process.stdout.write(output);
    return;
  }
  // A nightly may cross midnight while building; keep the resolved snapshot tag.
  const releaseTag = tagArgument || metadata.tag;
  if (
    channel === "nightly" &&
    !new RegExp(`^nightly-\\d{8}-${sourceCommit.slice(0, 12)}$`).test(releaseTag)
  ) {
    throw new Error("Nightly tag must identify this exact source snapshot.");
  }
  const repository = process.env.GITHUB_REPOSITORY;
  if (!repository || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error("GITHUB_REPOSITORY is required for release notes.");
  }
  const notes =
    `Built from [${sourceCommit.slice(0, 12)}](https://github.com/${repository}/commit/${sourceCommit}).\n\n` +
    `Channel: ${channel}. Application version: ${version}.\n\n` +
    "Packages: Windows x64 (EXE/MSI), macOS Intel and Apple Silicon (DMG), Linux x64 and ARM64 (DEB/RPM/AppImage). Download the package for your OS and CPU.\n\n" +
    "SHA256SUMS and artifact-manifest.json record package hashes, target architectures, toolchain policies, and the exact source commit.\n\n" +
    "Windows installers are unsigned. macOS DMGs use ad-hoc signing and are not Developer ID signed or notarized. Linux packages are unsigned. These builds do not certify clean-machine installation or removable-media hardware workflows.\n\n" +
    (channel === "nightly"
      ? "Nightly builds are prereleases for testing and may contain incomplete features.\n\n"
      : "") +
    `[Current certification evidence](https://github.com/${repository}/blob/${sourceCommit}/memory-bank/certification-matrix.md) and [packaging limitations](https://github.com/${repository}/blob/${sourceCommit}/memory-bank/tasks/TASK011-packaging-release-support.md).\n`;
  await writeFile("RELEASE_NOTES.md", notes);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  await main();

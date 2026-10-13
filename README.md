# Media Ingest Tool

Media Ingest Tool is a local-first desktop application for safely ingesting removable camera media. It uses Tauri 2, Rust, React, TypeScript, Vite, and Tailwind CSS.

The app includes conservative removable-volume discovery, controlled SanDisk reader-slot calibration, bounded verified ingest, BLAKE3 destination readback, SQLite manifest/receipt sealing, camera/time destination sorting, cooperative cancellation, and guarded card setup. Windows has local automated and limited physical-card evidence; this does not certify every reader, card, or platform.

Physical immutable-card identity through every reader, hotplug/reconnect recovery, Linux hardware workflows, and signed release artifacts remain incomplete or uncertified. Windows quick format is available through native target validation and explicit authorization, with a separately confirmed recovery path. macOS formatting remains unavailable pending an authorized native helper; macOS lifecycle and Linux formatting require native-host and hardware verification.

## Development

Prerequisites and platform notes are in [Development guide](docs/development.md).

```powershell
npm ci
npm run check
npm run tauri dev
```

The app has no broad frontend filesystem or shell access. Native functionality is exposed only through narrow typed Rust commands.

## Configure a card

Select a connected card and choose **Configure ingest**. The dialog walks through
Destination, Organization, Automation, and Review. Choose a folder, set the EXIF
or original-tree rule and folder order, then choose whether to remember settings
on the card. **Save configuration** applies the reviewed settings; Cancel or
Escape discards the draft. The workspace shows the saved summary and an
**Edit configuration** action.

**EXIF hour** groups media by capture hour. With Camera time selected, a 13:42
capture with a +08:00 offset goes into
`<camera>/20261013T000000+0800/1300-1400+0800/<filename>`. The start is included
and the end is excluded, so a 14:00 capture goes into `1400-1500+0800`. The last
hour ends at `2400`; midnight begins a new date folder. Custom intervals use the
same range naming. Missing offsets remain explicit, and unreadable capture
metadata uses the source modification timestamp. Existing files and frozen
recovery paths are not renamed by this new-plan naming change.

Choose **Camera time** (default) to keep the recorded capture clock, or
**Computer local time** to convert known capture instants to the computer's
local timezone at the capture date, including daylight saving time. Captures
without an offset keep their recorded clock and use `offset-unknown` in either
mode. Unsupported metadata falls back to the source modification time in UTC;
Computer local time converts that known instant. No capture or fallback time
means time-based planning fails instead of inventing the ingest time.

Before using capture-time folders, synchronize the camera and computer clocks
and check their date, time, timezone, and daylight saving settings. The dialog
and workspace show this warning. The choice is saved with the card's local
profile and applies to preview, manual ingest, and future automatic ingest.
Existing files and recovery plans retain their saved paths.

Card persistence uses the desktop app's existing mutable marker registration.
Automatic ingest and automatic format require persistence and remain explicit
opt-ins for future mounts. Saving does not start a copy. Session-only saving
leaves previously stored card settings available for future mounts. Browser
preview mode saves only session settings and does not write to removable media.

## Continuous verification and packaging

The Quality workflow follows the checked-in Node/Rust policies and lockfiles and checks the frontend, native Rust, and packaging safeguards on Windows, macOS, and Linux. Normal pushes and pull requests also build a macOS app/DMG. Nightly and release workflows run those checks and the following package builds in parallel; publication requires every check and target to pass.

| Platform | Architecture        | Packages           | Build runner        |
| -------- | ------------------- | ------------------ | ------------------- |
| Windows  | x64                 | EXE (NSIS), MSI    | Windows Server 2022 |
| macOS    | Intel x64           | DMG                | macOS 15 Intel      |
| macOS    | Apple Silicon ARM64 | DMG                | macOS 15 ARM64      |
| Linux    | x64                 | DEB, RPM, AppImage | Ubuntu 22.04        |
| Linux    | ARM64               | DEB, RPM, AppImage | Ubuntu 22.04 ARM64  |

**Nightlies:** [Nightly packages](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/workflows/nightly.yml) runs every night at **02:17 Asia/Singapore** (18:17 UTC). It snapshots the latest default-branch commit and publishes an immutable `nightly-YYYYMMDD-<12-character-SHA>` prerelease on the [Releases page](https://github.com/reo-sekailens/Media-Ingest-Tool/releases). It creates no source commits. The same workflow can run manually; an already-published snapshot is never overwritten. GitHub scheduling can be delayed and may disable schedules after 60 days of public-repository inactivity ([GitHub schedule documentation](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#schedule)).

**Versioned releases:** update the versions in `package.json`, `package-lock.json` (including its root package), `src-tauri/Cargo.toml`/`Cargo.lock`, and `src-tauri/tauri.conf.json` together. Commit that version, create a matching `vX.Y.Z` tag, and push it. [Release packages](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/workflows/release.yml) validates the version and publishes the exact tagged commit. Tags such as `v1.0.0-rc.1` create prereleases; stable versions create normal releases. Manual dispatch accepts an existing matching tag. The workflow refuses to replace an existing release or its assets; a failed draft must be inspected and removed before a full rerun. No signing secrets are required for the current unsigned builds.

Every package is named with its target architecture. Downloads include combined `SHA256SUMS` and `artifact-manifest.json` with the exact source commit, application version, and toolchain policies. The publisher checks hashes, sizes, provenance, versions, required formats, and all five targets before creating the release. For example, run `sha256sum --check SHA256SUMS` after downloading every listed package, or compare a selected file with `Get-FileHash -Algorithm SHA256` on Windows.

Windows and Linux packages are unsigned. macOS uses ad-hoc signing, which is not Developer ID signing or notarization. Runner versions and generated packages do not certify runtime or removable-media support. Clean-machine installation, signing/notarization, dependency security audits, SBOM generation, and hardware certification remain in [TASK011](memory-bank/tasks/TASK011-packaging-release-support.md). The existing manual macOS-only installer workflow remains available.

## macOS manual installation

Intel and Apple Silicon use separate native DMGs. The configured minimum is
macOS 13.3, matching the system WebKit needed by Tailwind 4. An Apple Developer
account is not required: packages are ad-hoc signed, and a downloaded app may
need Apple's per-app **Privacy & Security > Open Anyway** exception.

See [Mac installation and permissions](docs/macos-installation.md) and the
[compatibility review and validation boundaries](docs/macos-compatibility.md).
Quality checks cover macOS 15/26 on both architectures, including native core,
disposable APFS/exFAT images, actual WKWebView smoke checks and independently
checked shipping DMG startup. [Final software/UI checks](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38351288268)
pass all 16 jobs; [the production candidate](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38349630929)
passes all four DMG gates with independently verified downloaded hashes. These
results refer to the retained validation snapshots; their source and checks
are integrated into `main` without issuing a versioned release;
physical reader/card, privacy-consent, sleep/wake, and macOS 13.3 runtime
acceptance remain separate. **macOS Quick Format is unavailable.**

Use APFS or case-sensitive APFS destinations on Mac. ExFAT cards are valid
sources, but ExFAT destinations lack the atomic no-overwrite publication
required by the app on tested Mac systems and are rejected safely.

## Evidence-oriented project workflow

The repository includes an evidence-oriented memory bank and project entry points:

- Start with [AGENTS.md](AGENTS.md) for the operating rules.
- Read [memory-bank/README.md](memory-bank/README.md) for the maintained project context.
- Create and track work through [memory-bank/tasks/_index.md](memory-bank/tasks/_index.md).
- Follow the researched [media-ingest delivery roadmap](memory-bank/tasks/ROADMAP.md).
- Record durable choices in [memory-bank/decisions/README.md](memory-bank/decisions/README.md).
- Use the GitHub templates for reproducible issues and pull requests.

Record implementation unknowns and evidence in `memory-bank/projectbrief.md` and `memory-bank/techContext.md`. A build, local test, packaged artifact, and native hardware result are distinct kinds of proof.

Validate the project documentation at any time with:

```powershell
pwsh -NoProfile -File scripts/verify-memory-bank.ps1
```

## License

Licensed under [AGPL-3.0](LICENSE).

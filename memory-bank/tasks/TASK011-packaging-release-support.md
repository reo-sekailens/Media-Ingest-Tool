# TASK011 — Packaging, release, and supported-platform contract

- **Status:** in progress — five-target nightly/versioned package automation is implemented. Hosted build/publication verification, signing/notarization, clean-machine install/start, SBOM, audits, and hardware-certified runtime support remain pending.
- **Depends on:** TASK001, TASK009, TASK010

## Objective

Ship reproducible native desktop artifacts and publish an honest support contract for the exact operating systems, architectures, privileges, and reader/card behaviors proven by TASK010.

## Scope

- Freeze the application name and reverse-domain bundle identifier before persistent app data or signing identities ship.
- Pin Rust and JavaScript dependency ranges and commit the lockfiles.
- Build Windows installer/portable artifacts, signed/notarized macOS universal or architecture-specific artifacts, and Linux packages appropriate to the certified distributions.
- Use platform-specific Tauri configuration files for entitlements, minimum versions, icons, bundling, and any privileged format helper.
- Generate an SBOM, checksums, release notes, license notices, and a mapping from artifact to source commit.
- Keep the app functional offline. Do not add analytics, accounts, cloud storage, or a server as part of packaging.
- If automatic updates are added later, require signed update metadata and a separate opt-in task.

## Acceptance criteria

- Clean machines can install/start the exact packaged artifacts and complete all non-destructive critical workflows.
- Signed/notarized status is stated per artifact; an unsigned local build is never labeled production-ready.
- The support table lists exact OS versions, architectures, required system services, filesystem constraints, elevation behavior, and verified SanDisk models.
- Uninstall behavior explicitly preserves or removes local history according to a documented user choice.
- CI runs formatting, linting, type checks, Rust tests, frontend tests, security audits, package builds, and artifact checksum generation.
- Release notes disclose any `partial` or `blocked` certification row.

## Verification evidence

### 2026-10-10 nightly and release automation

Nightly publication runs at 02:17 Asia/Singapore from one immutable snapshot of
the latest default-branch commit. It creates a dated commit-specific prerelease,
without source commits or moving existing release tags. Manual dispatch is
available. Version-tag pushes and manual existing tags validate matching npm,
lockfile, Cargo, and Tauri versions; SemVer prereleases retain that status.

Reusable builds cover Windows x64 EXE/MSI, macOS Intel/Apple Silicon DMG, and
Linux x64/ARM64 DEB/RPM/AppImage. Quality and packaging run in parallel, while
publication requires every job and target to succeed. Only the publisher receives
write permission. It verifies the full matrix, required formats, hashes, sizes,
source SHA and version provenance before uploading combined manifests/packages.
Published releases/assets are never overwritten. Failed drafts require inspection
and removal before a full rerun; original source commits remain rollback anchors.

The observed CI failures in Windows checkout line endings, macOS plist reader
fixtures/Clippy, and Linux mountinfo lifetime are corrected without changing
hardware policy. Local Node safeguards and four source-derived platform fixtures
pass. Hosted native checks, packages and publication remain unverified. Windows
and Linux packages are unsigned; Mac packages use ad-hoc signing, without
Developer ID signing/notarization. These are package targets, not new runtime or
hardware certification claims. SBOM, audits and clean-machine checks remain open.

- Native CI logs and artifact hashes for every supported OS/architecture pair.
- Install/start/uninstall smoke-test records from clean machines or clean user profiles.
- Signing/notarization verification output and the published supported-platform table.
- Packaged-app critical-path results linked back to TASK010 without substituting source builds for package evidence.

## Latest local artifact

2026-08-28: Commit `a26b1fb35112846fa8881f2de5916b6a04069a8e` built the
unsigned x64 NSIS artifact
`src-tauri/target/release/bundle/nsis/Media Ingest Tool_0.1.0_x64-setup.exe`
(3,233,407 bytes, SHA-256
`D8C551B82F2DD66678CE6A23E261AC90E7B15C86BC43201B09649BDF38777792`). Its
Authenticode status is `NotSigned`. This is a local artifact creation result;
no installer run, clean-machine test, signing, notarization, or release
publication is claimed.

## Research sources

- [Tauri distribution overview](https://v2.tauri.app/distribute/)
- [Tauri platform-specific configuration](https://v2.tauri.app/reference/config/)
- [Tauri Windows code signing](https://v2.tauri.app/distribute/sign/windows/)
- [Tauri macOS code signing and notarization](https://v2.tauri.app/distribute/sign/macos/)
- [Tauri Linux package signing](https://v2.tauri.app/distribute/sign/linux/)
- [Tauri macOS application bundle](https://v2.tauri.app/distribute/macos-application-bundle/)
- [Apple macOS notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

# TASK011 — Packaging, release, and supported-platform contract

- **Status:** in progress — local unsigned Windows x64 NSIS packaging completed on 2026-08-23. The setup executable and native executable hashes are recorded in progress evidence; signing, clean-machine install/start proof, macOS/Linux packages, SBOM, and hardware-certification-dependent release support remain pending.
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

- Native CI logs and artifact hashes for every supported OS/architecture pair.
- Install/start/uninstall smoke-test records from clean machines or clean user profiles.
- Signing/notarization verification output and the published supported-platform table.
- Packaged-app critical-path results linked back to TASK010 without substituting source builds for package evidence.

## Research sources

- [Tauri distribution overview](https://v2.tauri.app/distribute/)
- [Tauri platform-specific configuration](https://v2.tauri.app/reference/config/)
- [Tauri Windows code signing](https://v2.tauri.app/distribute/sign/windows/)
- [Tauri macOS code signing and notarization](https://v2.tauri.app/distribute/sign/macos/)
- [Tauri Linux package signing](https://v2.tauri.app/distribute/sign/linux/)

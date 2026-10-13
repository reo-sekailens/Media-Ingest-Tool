# macOS compatibility and validation

Review date: 2026-10-13. Targets: Intel `x86_64-apple-darwin` and Apple Silicon
`aarch64-apple-darwin`. Distribution: architecture-specific ad-hoc-signed DMGs,
manually copied to Applications, without an Apple Developer account.

The configured minimum is macOS 13.3. macOS 15 and 26 native runners exercise
both architectures. A declared minimum, static documentation review, native
runner test, packaged launch and physical-card result are different evidence.
The 13.3 runtime and physical hardware remain acceptance gaps.

## Review coverage

The source-derived review covers the project's relevant platform contracts,
with current primary documentation and pinned dependency source where it matters:

| Area                                                                                                                          | Source and evidence record                                                                                                                          |
| ----------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Tauri prerequisites, architecture targets, bundle configuration, system WebKit, signing and manual distribution               | [Package validation](macos-package-validation.md), [installation](macos-installation.md), platform configuration and locked Tauri/Rust dependencies |
| Disk Arbitration/CoreFoundation ABI, callbacks, run-loop ownership, diskutil discovery/eject, identity and privileges         | [Native review](macos-native-review.md), installed runner `man diskutil` artifacts, native API tests                                                |
| APFS/exFAT names, collisions, verified streaming/readback, cancellation/recovery, immutable publication, SQLite and full-sync | [Runtime review](macos-runtime-review.md), core tests and disposable filesystem images                                                              |
| Privacy permissions, accessory approval, folder picker, local data/logs, sleep/wake and formatting limits                     | [Installation](macos-installation.md), [runtime review](macos-runtime-review.md), [native review](macos-native-review.md)                           |
| Actual WKWebView loading, real read-only IPC, empty state, refresh and screenshots                                            | [Native UI validation](macos-ui-validation.md), isolated debug-only harness                                                                         |

This is a bounded review of the actual implementation. It does not claim to have
read every page on the internet or certify future documentation, untested OS
versions, every M-series chip model, peripherals, storage servers, or camera formats.

## Implemented compatibility fixes

- Corrected the five-argument Disk Arbitration description-change registration,
  CoreFoundation Boolean ABI and retained run-loop lifetime across shutdown.
- Native discovery requires matching fresh device/mount identifiers and explicit
  internal/external evidence. Mounted read-only cards can be admitted based on
  readable roots; inability to write a marker does not imply inability to ingest.
- Safe eject uses the native utility's supported argument shape and OS-neutral
  errors. macOS format remains fail-closed pending a privileged native helper.
- Raised the minimum from 13.0 to 13.3 to match Tailwind 4's Safari 16.4 baseline.
  Ad-hoc signing is configured for local builds as well as CI. Added accurate
  removable-volume and destination-folder privacy descriptions.
- Media and receipt publication use macOS `renamex_np(RENAME_EXCL)` to refuse an
  occupied destination atomically. Collisions preserve the verified partial;
  unsupported filesystem operations fail instead of falling back to overwrite.
- Inventory skips only exact root `.fseventsd`, `.Spotlight-V100`, `.Trashes`,
  Windows service metadata and the app marker. Ordinary dotfiles/nested names
  remain media, and inventory/planning/content witnesses share the enumeration.
- File-backed macOS SQLite connections require WAL, FULL synchronous and
  `fullfsync=ON`, including after reopen. This does not certify power-loss safety
  on physical drives or parent-directory persistence after publication.
- Quality checks now cover native Intel/ARM64 on macOS 15/26. DMG verification
  checks copies, signatures, Mach-O architecture/deployment/dependencies, plist,
  icons, privacy strings and fresh-profile LaunchServices startup.
- Native UI automation uses a dedicated debug feature and separate application
  identifier. A release build with that feature is rejected at compile time;
  production package jobs never enable it. No app-facing shell/filesystem
  permission or hardware mock was added.

## Validation record

Local checks and hosted exact-source evidence will be recorded here after the
integration snapshot completes. Parser/fixture tests on Windows are not Mac
runtime or installation evidence. Native UI tests run the real Tauri app with
an embedded test driver; shipping DMG startup is checked independently.

## Remaining acceptance boundaries

- Downloaded-app quarantine/Gatekeeper **Open Anyway**, Allow/Deny privacy prompts,
  native folder-picker selection and native keyboard events need an installed Mac
  session. The synthetic WebDriver keyboard implementation cannot establish real
  keyboard activation, and a generated runner package does not exercise download
  quarantine.
- Real reader/card identity, write-protection, unplug/replug/mount reuse, multiple
  cards, sleep/wake during operations, throughput and safe eject need physical
  media. Virtual disk-image tests exercise filesystem/core behavior only.
- macOS 13.3 is the configured baseline; current hosted runners cannot prove that
  oldest runtime. Binary deployment inspection catches a newer load-command
  minimum but cannot prove every symbol or OS behavior on 13.3.
- `renamex_np` support, full-sync behavior and directory-entry durability depend
  on the actual destination filesystem/device. Unsupported operations stay errors.
  Network storage and power loss are not certified.
- Full native IOMedia topology/card hardware identity, stronger inode/handle
  binding against external mutation,
  power notifications and privileged macOS formatting remain separately scoped
  follow-up work in the task/certification records.

The original working changes remain preserved. Validation snapshots are isolated
from the default branch and normal nightly publication until explicitly integrated.

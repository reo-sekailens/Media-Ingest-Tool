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
  Disk images are excluded by their native bus protocol even when macOS omits
  the virtual/physical field.
- Safe eject uses the native utility's supported argument shape and OS-neutral
  errors. macOS format remains fail-closed pending a privileged native helper.
- Raised the minimum from 13.0 to 13.3 to match Tailwind 4's Safari 16.4 baseline.
  Ad-hoc signing is configured for local builds as well as CI. Added accurate
  removable-volume and destination-folder privacy descriptions.
- Media and receipt publication use macOS `renamex_np(RENAME_EXCL)` to refuse an
  occupied destination atomically. Collisions preserve the verified partial;
  unsupported filesystem operations fail instead of falling back to overwrite.
  Native ExFAT images support full file flush but reject this exclusive rename.
  ExFAT sources can be copied to APFS destinations; unsupported destinations
  receive an actionable APFS-destination error and retain source/partial data.
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

Production candidate: `9e4c4fadc1c615677fd61c0cf6c68c78fea30c85`,
[installer/core evidence](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38349630929).
All four production app/DMG gates passed, including signature verification,
architecture/deployment/dependencies, copy from a read-only DMG, detach and
fresh-profile LaunchServices startup. All four downloaded DMGs independently
matched their manifests, checksums, validation reports and source provenance.

| Native target | Actual OS | Core Rust  | Mounted-filesystem checks | Shipping DMG/startup | Native UI                    |
| ------------- | --------- | ---------- | ------------------------- | -------------------- | ---------------------------- |
| Intel         | 15.7.9    | 142 passed | 21 passed                 | passed               | 5 passed, 1 conditional skip |
| Apple Silicon | 15.7.9    | 142 passed | 21 passed                 | passed               | 5 passed, 1 conditional skip |
| Intel         | 26.6.1    | 142 passed | 21 passed                 | passed               | 5 passed, 1 conditional skip |
| Apple Silicon | 26.6.2    | 142 passed | 21 passed                 | passed               | 5 passed, 1 conditional skip |

Core suites skipped five physical-hardware cases and two disposable-image cases;
the latter were explicitly executed by the filesystem script. Each of the 21
image invocations actually ran one test: nine on APFS, nine on case-sensitive
APFS, and three ExFAT checks. Those exercise normal copy/readback/publication,
collisions, cancellation/recovery, receipts, metadata exclusions, ExFAT-to-APFS
production completion/SQLite reopen/marker upgrade, and safe ExFAT-destination
refusal. They do not certify physical storage or power loss.

Native UI evidence uses the harness correction in `0984bc7`,
[final Quality run](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38351288268).
All 16 software/UI jobs passed; each of four Mac hosts again passed 142 core
tests and 21 mounted-image invocations. Distribution was intentionally skipped
in this check-only run because production packages had already been verified.
The production app, configuration, dependency locks and frontend sources are
identical to the verified `9e4c4fa` installers; only the adapter, its contracts
and documentation changed. All four lanes passed on their first attempt,
checking real WKWebView rendering, read-only IPC/history, refresh, empty state
and native screenshots. Media panels were explicitly skipped with zero admitted
media. The debug-only driver is absent from the shipping builds.

The initial Mac26 ARM test adapter used legacy `execute/sync`, which timed out
before product IPC. Failure evidence was preserved outside the checkout. The
corrected adapter follows the pinned official embedded `/wdio/eval` route and
its Mac26 safeguards, with unchanged deadlines and no automatic harness retry.
The identical test binary then passed. See [the mechanism](macos-ui-validation.md).

Frontend checks include 100 tests, TypeScript, ESLint, formatting and production
builds across all six quality hosts. Packaging/diagnostic scripts have 30 portable
checks and the native transport has 10 contracts. Windows strict Rust/format
checks and repository memory validation also pass. Fixtures on Windows are not
Mac runtime evidence; shipping startup and instrumented UI remain separate.

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
  ExFAT destinations are blocked by their missing exclusive-rename capability;
  use APFS or case-sensitive APFS on macOS. Network storage and power loss are
  not certified.
- Full native IOMedia topology/card hardware identity, stronger inode/handle
  binding against external mutation,
  power notifications and privileged macOS formatting remain separately scoped
  follow-up work in the task/certification records.

The user requested that all working changes be committed to `main`. Production
inputs and test harness match the retained validation snapshots; documentation
records this integration. The snapshot branch remains available for provenance.
No versioned release is issued by this source integration; existing nightly
automation selects the latest default-branch source on its normal schedule.

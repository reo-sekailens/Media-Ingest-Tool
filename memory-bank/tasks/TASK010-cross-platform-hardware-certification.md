# TASK010 — Cross-platform, filesystem, and hardware certification

- **Status:** planned
- **Depends on:** TASK002, TASK003, TASK004, TASK005, TASK006, TASK008, TASK009
- **Unlocks:** TASK011

## Objective

Prove behavior with automated fixtures and real sacrificial media on every declared platform, while keeping source/build, simulated, native-hardware, packaged, and destructive-format evidence distinct.

## Required matrix

- Windows 10/11, macOS on Intel and Apple silicon where supported, and Linux with udev/UDisks2; exact minimum versions and CPU architectures must be frozen in TASK011.
- Source filesystems: FAT16 where applicable, FAT32, exFAT, read-only media, corrupt/dirty media, and unsupported filesystems.
- Destinations: NTFS, APFS, exFAT, and ext4 plus a documented network-volume policy.
- Readers: built-in SD slot, generic single-slot USB reader, SanDisk Professional SD/microSD models named in TASK006, PRO-DOCK if in scope, and two identical readers connected simultaneously.
- Cards: at least two SD and two microSD cards, including two cards with the same vendor/product/capacity; only sacrificial cards may be formatted.
- Failures: source/destination unplug, read error, write error, full destination, permission denial, sleep/wake, app crash, reboot, hash mismatch, metadata corruption, and duplicate paths.

## Performance method

- Record card, reader, host controller/topology, source/destination filesystem, destination medium, file-size distribution, thermal state, concurrency, and digest mode.
- Benchmark large sequential files, many small files, mixed camera trees, two simultaneous sources, and sources sharing one USB controller.
- Compare bounded concurrency values rather than assuming maximum threads are fastest. Define guardrails for UI responsiveness, memory use, and error rate as well as throughput.
- Use full destination re-read verification in the published end-to-end number. Do not report copy-only throughput as verified ingest throughput.

## Acceptance criteria

- The certification matrix records each feature/platform as `certified`, `partial`, `blocked`, or `not applicable` with dated evidence.
- Golden source trees produce identical manifests and destination digests across supported platforms.
- Hardware identities remain stable across reconnect, reboot, port changes, and slot permutations only where the documented identity source promises it; otherwise confidence is downgraded.
- SanDisk slot labels pass the exact insertion matrix in TASK006; unsupported variants remain generic.
- Quick-format tests prove the expected filesystem/partition result and that protected disks remain untouched.
- Packaged native builds repeat the critical discover → ingest → verify → format path before release.

## Evidence artifacts

- Sanitized device-observation snapshots, manifests, benchmark CSV/JSON, test logs, native screenshots, and package hashes.
- No serial number or host path is published without explicit sanitization.

## Current evidence boundary

- The cross-platform GitHub Actions Rust matrix now installs the Linux system
  D-Bus development headers required by the direct UDisks provider before
  running `cargo fmt`, clippy, and tests. This prepares the Linux source gate
  for a remote run; it is not Linux runtime, D-Bus, filesystem, or hardware
  certification.

- Windows hardware evidence currently covers a controlled three-file core ingest from a sacrificial full-size SD card through the observed SanDisk PRO-READER to two local dummy destinations, with fresh post-run SHA-256 comparisons. It does not cover the Tauri IPC lifecycle, marker timing, format authorization/provider, a microSD card, another reader, or any non-Windows platform.
- 2026-08-24: both opt-in hardware probes passed from `D:\\DCIM\\100DUMMY` to unique test-only folders under `F:\\dummy\\1` and `F:\\dummy\\2`. Each copied all 3 files (1,310,762 bytes); independent PowerShell SHA-256 comparison found 0 mismatches. The probe deliberately targets the controlled fixture subdirectory because additional non-fixture files are present at the card root.
- 2026-08-26: native desktop UI on Windows scanned and previewed a three-file 3,670,016-byte controlled fixture from the PRO-READER's mounted LUN 1 card, then completed verified ingest to an isolated `F:` destination. Receipt `92ff1f3c-e591-447f-901d-82d63e5dc441.json` sealed successfully, source marker creation was reported, and independent SHA-256 comparison found 0 mismatches. The UI correctly kept quick format unavailable for the reader's unresolved card identity; no destructive operation occurred. LUN 0 was no-media, so this does not establish a physical slot mapping or two-card behavior.
- 2026-08-26: both opt-in D: fixture hardware probes passed again to their unique isolated `F:\dummy\1` and `F:\dummy\2` output folders (3 files / 1,310,762 bytes each); independent SHA-256 checks found 0 mismatches. The current packaged Windows app also registered the marked M: certification card with auto-ingest enabled and auto-format disabled. On a fresh app observation, run `9df43fc8-0179-4b02-86db-940e424fc4f9` copied all three M: files to `F:\media-ingest-auto-cert-20260826`, sealed its receipt, and an independent SHA-256 comparison found 0 mismatches. Restart observation is not a physical removal/reinsert certificate.
- 2026-08-26: after the final packaged executable rebuild, another fresh app observation started and completed the registered M: auto-ingest as run `1c6f56b8-c21a-4c63-a4bd-f018ff1debf6`. Its local ledger records `M:\` to `F:\media-ingest-auto-cert-20260826`, generation 2, and completed state; the sealed manifest contains the controlled three-file / 3,670,016-byte plan. Independent SHA-256 comparisons again found 0 mismatches. This proves the current package's observed-start behavior, not a physical remount.
- 2026-08-26: after schema-v11 added persisted automatic-run state and an atomic per-generation duplicate guard, a freshly rebuilt unsigned NSIS package completed one migration-era auto run, `738d6152-6f13-4009-badc-aa2f280a4b7b`, then on restart made no additional run. The local ledger remained at five runs and the selected M: UI rendered “Registered card already auto-ingested for this mount.” Its three copied files independently SHA-256 matched the controlled source. This certifies restart suppression in the package, not physical removal/reinsertion.
- 2026-08-26: the same rebuilt package manually ingested the selected M: card to fresh `F:\media-ingest-manual-v11-cert-20260826`. Run `336875d5-6b91-4f7f-8717-3ea0ab7afee6` completed with `auto_ingest_triggered = 0`, sealed 3 files / 3,670,016 bytes, and independently SHA-256 matched every source file. This is the packaged manual critical path; two-card, interruption, filesystem, and cross-platform matrix evidence remains open.
- 2026-08-26: the two opt-in probes were run once more against the current core. They wrote isolated folders `copy-1-6e479e7d-3538-4877-81af-4a927bf427ac` and `copy-2-88ee2333-0e7d-4d3b-a2c0-e5e7f2f77db2`; each independently SHA-256 matched all three controlled D: files, with 0 mismatches. This is direct Windows filesystem/core evidence, not an IPC or packaged-app result.
- 2026-08-26: the opt-in `hardware_two_card_concurrent_verified_ingest_probe` ran the controlled D: SD fixture and M: microSD fixture concurrently into separate unique folders under `F:\media-ingest-hardware-tests`. It sealed independent receipts for 3 files / 1,310,762 bytes and 3 files / 3,670,016 bytes respectively. Fresh SHA-256 comparison found 0 mismatches and 0 extra files for each destination. This is controlled core concurrency evidence only; it does not prove concurrent desktop IPC, physical remount, recovery, another reader, or another OS.
- 2026-08-26: the freshly rebuilt unsigned Windows package completed two concurrent manual desktop-IPC ingests from the calibrated full-size SD and microSD cards into separate folders below `F:\media-ingest-packaged-concurrent-efb6cec8-0817-4a7f-aafb-6243fb07e2c1`. The microSD was actively copying when the SD start was accepted. Sealed receipt `4f4ceb3b-5ed9-4e8d-ac79-43486619700a` contains 5 files / 2,688,024,576 bytes, and `235d92c7-6f73-4bb0-936c-753b87a8f093` contains 19 files / 6,465,617 bytes. Independent SHA-256 re-read found 5/5 and 19/19 matching payloads with 0 missing, extra, or mismatched files after excluding the marker and Windows system-volume metadata. This establishes Windows packaged two-card IPC concurrency, not interruption/recovery, physical remount, another reader, or another OS.

## Research sources

- [Tauri testing guidance](https://v2.tauri.app/develop/tests/)
- [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- [SD Association formatter user manual](https://www.sdcard.org/pdf/SD_CardFormatterUserManualEN.pdf)

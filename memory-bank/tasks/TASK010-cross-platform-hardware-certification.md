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

- Windows hardware evidence currently covers a controlled three-file core ingest from a sacrificial full-size SD card through the observed SanDisk PRO-READER to two local dummy destinations, with fresh post-run SHA-256 comparisons. It does not cover the Tauri IPC lifecycle, marker timing, format authorization/provider, a microSD card, another reader, or any non-Windows platform.
- 2026-08-24: both opt-in hardware probes passed from `D:\\DCIM\\100DUMMY` to unique test-only folders under `F:\\dummy\\1` and `F:\\dummy\\2`. Each copied all 3 files (1,310,762 bytes); independent PowerShell SHA-256 comparison found 0 mismatches. The probe deliberately targets the controlled fixture subdirectory because additional non-fixture files are present at the card root.

## Research sources

- [Tauri testing guidance](https://v2.tauri.app/develop/tests/)
- [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/)
- [SD Association formatter user manual](https://www.sdcard.org/pdf/SD_CardFormatterUserManualEN.pdf)

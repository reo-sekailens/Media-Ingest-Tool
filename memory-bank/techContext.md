# Technical Context

## Repository baseline

The repository now has a Tauri 2 desktop source tree with a React/TypeScript/Vite/Tailwind frontend and Rust native crate. Local Windows foundation checks pass; this is not yet packaged, hardware-certified, or cross-platform runtime evidence.

## Tooling

- Tauri 2.11.1, React 19.2.8, TypeScript 5.8.3, Vite 7.3.6, Tailwind 4.3.3, Vitest 4.1.11, Node 24.14.0 policy, and Rust 1.98.0 policy.
- Local quality gate: Prettier, ESLint, strict TypeScript, Vitest, Vite production build, Rust fmt, Clippy with warnings denied, Rust tests, and the memory-bank verifier.
- CI defines native Windows, macOS, and Linux job matrices, but no remote run has been observed yet.

## Environments

- Target desktop families: Windows, macOS, and Linux.
- Exact supported OS versions, CPU architectures, system services, package formats, and native hardware matrix remain a TASK010/TASK011 certification decision.
- SanDisk Professional documentation lists Windows and macOS for the requested readers; Linux behavior must remain uncertified until tested on physical hardware.

## Integration inventory

- Tauri command/event and capability system for the trusted Rust/webview boundary.
- Windows Storage Management, device-interface, volume/disk mapping, and notification APIs.
- macOS Disk Arbitration plus IOKit/IORegistry storage and USB topology evidence.
- Linux udev/sysfs for block hotplug and identity evidence, with UDisks2 for desktop mount/eject/format authorization.
- CIPA Exif metadata for make/model/body serial/capture time where present; format-specific fallbacks remain explicit.
- BLAKE3 for high-throughput full-file integrity digests.
- SQLite for device profiles, session checkpoints, and receipts.

## Research constraints

- `sysinfo` may be useful for presentation-level capacity/mount facts but is not sufficient as the identity or hotplug authority.
- Most ordinary file operations are blocking on desktop operating systems. Use bounded dedicated/blocking workers rather than unbounded async tasks.
- OS-native fast-copy calls are not automatically a verification strategy. Verified ingest must hash source bytes and independently re-read the destination.
- SD CID is the preferred immutable card identifier when exposed, but USB mass-storage bridges commonly determine what identity reaches the host; missing CID is a product-visible limitation, not a value to synthesize.

Update this file when implementation or configuration establishes verified technical facts. See [System patterns](systemPatterns.md) and [Certification matrix](certification-matrix.md).

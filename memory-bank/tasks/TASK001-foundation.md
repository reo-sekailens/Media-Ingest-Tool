# TASK001 — Desktop foundation and cross-platform contracts

- **Status:** In progress — Windows local foundation verified; macOS/Linux native CI and runtime evidence remain pending.
- **Owner:** Unassigned
- **Priority:** P0
- **Depends on:** None
- **Blocks:** All implementation tasks, including TASK002 device discovery and identity

## Objective

Establish a Tauri 2 desktop application foundation using Rust for trusted/native work and React + TypeScript + Tailwind CSS for the UI. The foundation must make Windows, macOS, and Linux first-class targets and define stable contracts for device discovery, ingest, verification, sorting, and format operations before feature implementation begins.

This is an implementation task for a later phase. This planning record does not authorize application code in the current research-only phase.

## Product and platform boundary

- Desktop only: Windows, macOS, and Linux. Android and iOS are out of scope because the requested workflow operates on storage attached to a desktop host.
- Local first: enumerate devices, copy files, verify bytes, remember per-device configuration, and format media on the host. No cloud service is required for the core workflow.
- Rust owns all native device access, filesystem traversal, transfer, verification, persistence, and destructive operations. React renders state and submits typed user intent; it never receives an unrestricted native filesystem API.
- Device discovery must run without elevation where the OS exposes sufficient information. Privileged steps, especially format/eject or low-level enrichment, must be isolated, explicit, and never required merely to open the app.
- “Supported” means a platform has a native implementation and evidence on real hardware, not merely that it compiles. Tauri documents desktop capability targets as `linux`, `macOS`, and `windows`; the release matrix must record CPU architecture and minimum OS/distro assumptions separately ([Tauri capabilities](https://v2.tauri.app/security/capabilities/), [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

## Required architecture

### Process boundary

Use a static React/Vite frontend hosted by the Tauri webview. Tauri describes the frontend as static web content and exposes Rust through commands ([Tauri frontend configuration](https://v2.tauri.app/start/frontend/), [calling Rust from the frontend](https://v2.tauri.app/develop/calling-rust/)).

- Request/response operations use narrowly scoped Tauri commands.
- Ordered, potentially frequent device/job progress uses typed Tauri channels. Tauri documents channels as ordered and optimized for streaming, while its event system is not intended for high-throughput data ([calling the frontend from Rust](https://v2.tauri.app/develop/calling-frontend/)).
- Define serializable Rust DTOs with a TypeScript contract generation or contract-checking step so discriminated unions and field nullability cannot drift silently.
- Keep native platform implementations behind shared Rust traits/interfaces. Platform-specific modules may use FFI, but common orchestration and tests must not depend on OS path syntax.
- Long-running work must be cancellable and must not block Tauri's UI thread. Job state is owned in Rust and survives window rerenders.

### Planned module boundaries

The implementation may refine names, but it must preserve these ownership boundaries:

| Boundary   | Responsibility                                                                                |
| ---------- | --------------------------------------------------------------------------------------------- |
| `device`   | Snapshot, hotplug reconciliation, physical-reader/media/volume graph, identity confidence     |
| `ingest`   | Safe source enumeration, copy planning, bounded parallel transfer, cancellation, resumability |
| `verify`   | Size/digest verification and durable evidence                                                 |
| `organize` | Camera extraction and per-camera/per-day/custom-time destination planning                     |
| `format`   | Preflight, confirmation receipt, OS-native unmount/format/eject workflow                      |
| `reader`   | Reader and slot topology, including device-specific SanDisk Professional mapping              |
| `settings` | Per-medium destination and organization rules, stored locally                                 |
| `ipc`      | Minimal Tauri commands/channels and shared DTO contract                                       |
| React UI   | Presentation, accessibility, selection, user confirmation, progress and errors                |

### Security invariants

- Configure explicit Tauri 2 capabilities for the main window and only the commands/plugins it needs. Tauri states that capabilities constrain which windows/webviews may invoke native functionality and that capability overlap merges privileges ([Tauri capabilities](https://v2.tauri.app/security/capabilities/), [Tauri permissions](https://v2.tauri.app/security/permissions/)).
- Do not grant broad frontend filesystem access. A destination selected through an OS dialog becomes an opaque, Rust-validated path/configuration record.
- Canonicalize and validate source and destination roots in Rust. Reject source/destination overlap, traversal, symlink/reparse-point escapes where applicable, device nodes, sockets, and other non-regular files.
- Make copy and format separate permissions and commands. Formatting requires a fresh typed confirmation bound to the currently resolved medium identity, target device, filesystem choice, and an expiry time.
- Never expose raw pointers/handles, unrestricted shell execution, secrets, or arbitrary native command names over IPC.
- Logs and fixtures must not contain full real media contents or personal metadata. Stable identifiers may be hashed for routine telemetry/UI keys, but the raw identifier source and confidence remain available in local diagnostics.

## Toolchain and project setup work

1. Scaffold Tauri 2 with a Vite React TypeScript frontend. Use strict TypeScript; the `strict` option enables the strict family of checks ([TypeScript `strict`](https://www.typescriptlang.org/tsconfig/strict)).
2. Configure Tailwind CSS through its current first-party Vite integration and import path. Tailwind v4 recommends `@tailwindcss/vite` for Vite projects ([Tailwind v4 Vite guidance](https://tailwindcss.com/docs/upgrade-guide), [Tailwind functions and directives](https://tailwindcss.com/docs/functions-and-directives)).
3. Add formatting, lint, typecheck, unit-test, Rust format/lint/test, and production-build scripts with one documented aggregate verification command.
4. Pin a Node LTS line and Rust toolchain policy; commit lockfiles. Record exact versions when the scaffold is created rather than guessing in this plan.
5. Create target-aware CI jobs on native Windows, macOS, and Linux runners. Cross-compilation alone is not native API evidence.
6. Add typed error codes and a common result envelope that preserve actionable OS error context without leaking sensitive paths by default.
7. Establish test seam/fake implementations for device snapshots, hotplug streams, file trees, transfer faults, verification mismatches, and destructive-operation authorization.
8. Document local development prerequisites for all three OSes. Tauri currently requires WebView2 and Microsoft C++ Build Tools on Windows, Xcode/Command Line Tools on macOS, and distribution-specific WebKitGTK/system packages on Linux ([Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).
9. Define the packaging matrix, signing/notarization blockers, and native smoke-test checklist without claiming unsigned build evidence as production evidence ([Tauri distribution overview](https://v2.tauri.app/distribute/)).

## Acceptance criteria

- [ ] Repository contains a Tauri 2 desktop scaffold using Rust, React, TypeScript, Vite, and Tailwind CSS; dependency versions and lockfiles are committed.
- [ ] Desktop target matrix explicitly covers Windows, macOS, and Linux and records each architecture/minimum-version decision with rationale.
- [ ] Rust is authoritative for every native/storage/destructive operation, and the frontend has no broad filesystem or shell capability.
- [ ] Shared DTOs represent device state, identity confidence, transfer/verification status, organization plans, and format preflight without using OS paths as persistent IDs.
- [ ] Tauri capability/permission files expose only the minimum commands to the intended window.
- [ ] Ordered progress is carried over channels with cleanup/cancellation behavior documented; commands are used for bounded request/response work.
- [ ] Fake platform adapters allow deterministic unit tests without attached hardware.
- [ ] One documented local command runs frontend format/lint/typecheck/tests/build plus Rust format/lint/tests.
- [ ] Native CI compiles/tests the three target OS families; lack of signing, hardware, or a runner is recorded as a blocker rather than a pass.
- [ ] `memory-bank/techContext.md`, `systemPatterns.md`, `decisions.md`, `activeContext.md`, and `progress.md` are updated after implementation with only verified facts.

## Evidence plan

| Evidence          | Required proof                                                                                                                       |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Static contract   | TypeScript strict check and Rust serialization/contract tests pass                                                                   |
| Rust quality      | `cargo fmt --check`, `cargo clippy` with warnings denied for project code, and `cargo test` pass                                     |
| Frontend quality  | Formatter/linter, typecheck, unit tests, and Vite production build pass                                                              |
| Security boundary | Generated Tauri capability schema validates; negative tests show ungranted commands are unavailable                                  |
| Cross-platform    | Native Windows, macOS, and Linux CI jobs compile and run platform-independent tests                                                  |
| Runtime           | Packaged or development app opens on each OS; command invocation, channel ordering, cancellation, and listener cleanup are exercised |
| Durable context   | Task status and memory-bank architecture/tooling records match the implemented repository                                            |

## Risks and open decisions

- Minimum OS/distro versions and CPU architectures must be selected from product audience plus native API/CI evidence; Tauri’s broad framework prerequisites alone do not constitute this product’s support policy.
- Linux packaging and desktop integration differ across distributions. Start with a documented reference distro for native hardware certification and track additional formats separately.
- macOS App Sandbox/App Store distribution materially changes removable-volume access. Distribution channel must be decided before entitlements and file-access behavior are certified.
- Native device APIs require FFI and lifecycle discipline. Isolate unsafe code, require focused tests, and document ownership of every native handle/callback.

## Follow-up work

- TASK002 — Cross-platform device discovery, details, and identity confidence.
- Transfer, organization/camera identity, verification, format, SanDisk reader-slot, UI, persistence, packaging, and certification tasks linked from the task index.

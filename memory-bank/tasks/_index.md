# Task Index

## Active

Execute tasks according to the [delivery roadmap](ROADMAP.md), not numeric order alone.

| Task                                                        | Status      | Priority | Depends on                         | Outcome                                                                                             |
| ----------------------------------------------------------- | ----------- | -------- | ---------------------------------- | --------------------------------------------------------------------------------------------------- |
| [TASK001](TASK001-foundation.md)                            | in progress | P0       | —                                  | Tauri 2 + Rust + React + strict TypeScript + Tailwind foundation and typed trust boundary.          |
| [TASK002](TASK002-device-discovery-identity.md)             | planned     | P0       | TASK001                            | Native Windows/macOS/Linux device graph, hotplug, details, and identity confidence.                 |
| [TASK007](TASK007-local-store-profiles.md)                  | planned     | P0       | TASK001, TASK002                   | SQLite identities, per-card destinations, camera profiles, sessions, and receipts.                  |
| [TASK003](TASK003-ingest-copy-sort.md)                      | planned     | P0       | TASK001, TASK002, TASK007          | Bounded concurrent copy, camera identity, metadata, time buckets, and collision-safe paths.         |
| [TASK004](TASK004-verification-recovery.md)                 | in progress | P0       | TASK002, TASK003, TASK007          | Full BLAKE3 destination readback, durable manifest, resume, receipt, and format authorization.      |
| [TASK008](TASK008-operator-ui.md)                           | in progress | P0       | TASK001–TASK004, TASK007           | Accessible source, rule, progress, verification, history, and format workflow.                      |
| [TASK009](TASK009-security-destructive-safety.md)           | in progress | P0       | TASK001, TASK002, TASK004, TASK007 | Hostile-media boundary, narrow Tauri capabilities, path safety, and destructive-operation controls. |
| [TASK005](TASK005-safe-format.md)                           | planned     | P0       | TASK002, TASK004, TASK009          | Fail-closed OS-native quick format with remount/post-format validation.                             |
| [TASK006](TASK006-sandisk-slot-mapping.md)                  | planned     | P1       | TASK002                            | Exact SanDisk Professional SD vs microSD slot mapping and calibration.                              |
| [TASK012](TASK012-ingest-lifecycle-receipts.md)             | in progress | P1       | TASK002–TASK004, TASK007, TASK008  | Disconnect, cancel, crash, sleep/wake, safe eject, and operational receipts.                        |
| [TASK010](TASK010-cross-platform-hardware-certification.md) | planned     | P0       | TASK002–TASK009, TASK012           | Cross-platform filesystems, real hardware, failure, and performance certification.                  |
| [TASK011](TASK011-packaging-release-support.md)             | in progress | P1       | TASK001, TASK009, TASK010          | Signed/packageable artifacts and an exact supported-platform contract.                              |

## Dependency-safe batches

1. TASK001.
2. TASK002.
3. TASK007 and the TASK006 hardware-capture work that does not depend on final UI.
4. TASK003.
5. TASK004.
6. TASK008, TASK009, and TASK012 can proceed in parallel on separate ownership boundaries.
7. TASK005 after TASK009 safety gates are integrated.
8. TASK010, then TASK011.

The SanDisk task remains `planned`, not implemented: the connected Windows reader exposed two empty logical units, so the physical SD/microSD mapping still needs controlled insertion tests.

## Completed

- 2026-08-23 — Initialized the repository memory bank. See [Progress](../progress.md).
- 2026-08-23 — Completed research and task planning for the media-ingest product. No task above is implemented or certified.
- 2026-08-23 — TASK001 foundation is implemented and locally verified on Windows; cross-platform CI/runtime evidence remains pending.

## Task entry template

Create one file per substantial task and link it here. Include: objective, owner, status, dependencies, acceptance criteria, evidence, and follow-up work.

Related: [Active context](../activeContext.md) and [Certification matrix](../certification-matrix.md).

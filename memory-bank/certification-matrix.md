# Certification Matrix

This matrix separates implementation claims from verified evidence. Do not mark a feature or surface certified without recording the exact evidence and date.

| Feature or surface              | Status         | Evidence                                                                               | Last checked | Blocker / next action                                                  |
| ------------------------------- | -------------- | -------------------------------------------------------------------------------------- | ------------ | ---------------------------------------------------------------------- |
| Repository context              | certified      | Required memory files and 12 linked task records pass `scripts/verify-memory-bank.ps1` | 2026-08-23   | Keep context synchronized during implementation.                       |
| Requirements and task plan      | certified      | User requirements, primary-source research, `tasks/ROADMAP.md`, and TASK001–TASK012    | 2026-08-23   | Revisit when requirements change.                                      |
| Product behavior                | blocked        | No product implementation exists                                                       | 2026-08-23   | Execute the delivery roadmap.                                          |
| Build and tests                 | not applicable | No build or test configuration exists                                                  | 2026-08-23   | Add or document commands, then run them.                               |
| Local runtime                   | not applicable | No runnable application exists                                                         | 2026-08-23   | Add or document runtime entry point and validation.                    |
| Deployment                      | not applicable | No deployment configuration exists                                                     | 2026-08-23   | Document environment and deployment evidence.                          |
| Security and privacy plan       | partial        | Threats and controls documented in TASK001, TASK002, TASK005, and TASK009              | 2026-08-23   | Implement and test controls.                                           |
| Windows device discovery        | partial        | Native app live inventory rendered a mounted sacrificial SD card and empty paired LUN  | 2026-08-23   | Test removal/reinsert, source ingest, and CID behavior across readers. |
| macOS device discovery          | blocked        | Official API research only                                                             | 2026-08-23   | Requires implementation and native hardware.                           |
| Linux device discovery          | blocked        | Official API research only                                                             | 2026-08-23   | Requires implementation and native hardware.                           |
| Ingest and full verification    | blocked        | Architecture and acceptance criteria only                                              | 2026-08-23   | Implement TASK003/TASK004 and run filesystem/hardware matrix.          |
| Quick format                    | blocked        | OS/SDA research only; no destructive tests                                             | 2026-08-23   | Implement after verification/security gates using sacrificial cards.   |
| SanDisk SD/microSD slot mapping | partial        | Controlled SD insertion on revision `0056`: LUN 0 populated, LUN 1 empty               | 2026-08-23   | Calibrate microSD, repeat on second reader and other OSes.             |

## Evidence standard

Record the command, test, screenshot, log, deployment identifier, or review reference that supports a status. Source or build success alone does not certify deployed or production behavior.

Use only `certified`, `partial`, `blocked`, or `not applicable` as a status.

Related: [Technical context](techContext.md), [System patterns](systemPatterns.md), and [Progress](progress.md).

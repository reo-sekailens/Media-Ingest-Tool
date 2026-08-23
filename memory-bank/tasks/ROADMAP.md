# Media Ingest Tool delivery roadmap

- **Plan status:** ready for implementation
- **Planning date:** 2026-08-23
- **Code status:** no application code has been written

## Product outcome

Build a Tauri 2 desktop application whose Rust core discovers removable media, identifies it with the strongest stable hardware evidence available, lets each source retain its own destination and sorting profile, performs bounded concurrent ingest, independently verifies every destination file, and only then permits a guarded quick-format workflow. React, TypeScript, Vite, and Tailwind CSS provide the desktop UI.

## Delivery sequence

| Phase                                      | Tasks                     | Exit condition                                                                      |
| ------------------------------------------ | ------------------------- | ----------------------------------------------------------------------------------- |
| 1. Foundation                              | TASK001                   | Reproducible Tauri/Rust/React/TypeScript/Tailwind workspace and typed IPC boundary. |
| 2. Source truth                            | TASK002, TASK007          | Device identity and local profile/history contracts work with fixtures.             |
| 3. Ingest engine                           | TASK003, TASK004          | Files copy, sort, resume, and receive independent destination verification.         |
| 4. Operator surface                        | TASK008                   | Complete accessible workflow with fixture and native evidence.                      |
| 5. Destructive and hardware specialization | TASK005, TASK006, TASK009 | Format remains fail-closed; SanDisk slot claims are hardware-certified.             |
| 6. Lifecycle and certification             | TASK010, TASK011, TASK012 | Supported platforms are packaged, benchmarked, and certified separately.            |

TASK003 should not be considered complete until TASK007 provides durable identities and session state. TASK005 must remain unavailable until TASK004 and TASK009 gates pass. TASK006 is additive: the generic device flow must still work when a reader model or slot cannot be identified confidently.

## Non-negotiable invariants

- A drive letter, mount path, volume label, volume UUID, filesystem UUID, reader serial, or USB port path alone is not an immutable card identity.
- Persist a destination automatically only for an exact identity match at an accepted confidence level. Lower-confidence sources require explicit confirmation for the current session.
- Camera identity is `make + model + body serial` when the media exposes it. A camera model alone never merges footage from two bodies.
- Copy into a temporary destination file, compare a full source digest with a fresh full destination read, then atomically publish where the destination filesystem supports it.
- Never silently overwrite a destination path or follow a source symlink/reparse point outside the mounted source.
- Formatting is blocked until the current card and current ingest session are re-resolved and fully verified. Internal/system disks are never eligible.
- Hardware- and OS-specific claims require evidence from that exact platform and device; fixtures and builds are recorded separately.

## Primary research baseline

- [Tauri project creation and React/TypeScript templates](https://v2.tauri.app/start/create-project/)
- [Tauri frontend guidance recommending Vite for SPA frameworks](https://v2.tauri.app/start/frontend/)
- [Tauri IPC commands and events](https://v2.tauri.app/concept/inter-process-communication/)
- [Tauri capability boundary](https://v2.tauri.app/security/capabilities/)
- [Tailwind CSS Vite integration](https://tailwindcss.com/docs/installation/using-vite)
- [SD Card Identification register](https://www.sdcard.org/downloads/pls/)
- [SD Association formatting guidance](https://www.sdcard.org/downloads/formatter/faq/)
- [SanDisk Professional PRO-READER SD and microSD product](https://www.sandisk.com/products/accessories/memory-card-readers/sandisk-professional-pro-reader-sd-microsd)
- [SanDisk Professional PRO-READER Multi-Card product](https://www.sandisk.com/products/accessories/memory-card-readers/sandisk-professional-pro-reader-multi-card)

The implementation tasks contain the narrower OS/API sources and their evidence requirements.

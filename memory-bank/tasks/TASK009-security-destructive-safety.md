# TASK009 — Security boundaries and destructive-operation safety

- **Status:** in progress — explicit Tauri command capabilities, relative-path validation, non-overwrite destination behavior, symlink/reparse/mount-crossing exclusion during source inventory, pre/post-copy source-entry rechecks, and shared bounded hostile-tree limits (1,000,000 files / 64 directory levels) are implemented and locally tested. Handle-based anti-race traversal defenses, format-provider hardening, and cross-platform adversarial fixtures remain pending.
- **Depends on:** TASK001, TASK002, TASK004, TASK007
- **Unlocks:** TASK005, TASK010, TASK011

## Objective

Treat attached media and all path/metadata content as untrusted, constrain the Tauri webview, and make copy and format authorization fail closed across identity changes and time-of-check/time-of-use races.

## Threats and controls

- Reject traversal, reserved names, invalid encodings, device paths, alternate data streams, and unsafe destination components after normalization.
- Do not follow source symlinks, junctions, mount crossings, aliases, or reparse points by default. Prove every opened source remains beneath the selected volume using platform-appropriate handle/descriptor checks.
- Bound directory depth, file count, metadata size, path length handling, progress-event rate, and memory queues to resist hostile media.
- Keep filesystem, storage enumeration, hashing, and formatting in Rust commands with narrow serializable request types.
- Use explicit Tauri capability files; do not grant broad filesystem or shell access to the main webview. Do not load remote web content.
- Re-resolve physical disk, volume, card identity, mount generation, system-disk status, and verification receipt immediately before formatting. Any mismatch invalidates authorization.
- Use an OS-elevated helper or supported privileged service only for the narrow format operation. Authenticate the exact request; never run an interpolated shell command.
- Redact usernames, unrelated host volumes, serials not needed for a receipt, and media metadata from diagnostic exports by default.
- Pin dependencies, audit Rust and JavaScript supply chains, and document updater/signing trust separately.

## Acceptance criteria

- Malicious path/reparse/symlink fixtures cannot escape the selected source or destination roots.
- An unplug/replug, drive-letter/mount reuse, volume remount, identity-confidence downgrade, or destination change revokes format eligibility.
- The webview cannot call undeclared native operations or write arbitrary host paths.
- System, boot, recovery, internal fixed, destination, and ambiguous disks are ineligible for formatting in automated tests.
- Privilege denial and helper tampering fail safely without partially starting the operation.
- A repository threat model and security tests accompany implementation.

## Verification evidence

- Tauri capability review and IPC allowlist test.
- Cross-platform adversarial filesystem fixture suite.
- Race tests that swap mount/device observations between confirmation and execution.
- Static dependency audits plus manual review of every format call site.

## Research sources

- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
- [Tauri content security policy](https://v2.tauri.app/security/csp/)
- [Microsoft reparse points](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-points)
- [Linux openat2 path-resolution constraints](https://man7.org/linux/man-pages/man2/openat2.2.html)
- [Apple file-system programming guide: aliases and symbolic links](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemOverview/FileSystemOverview.html)

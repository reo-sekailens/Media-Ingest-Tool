# TASK005 - Safely quick-format verified source media

## Status and ownership

- **Status:** in progress — a native non-destructive readiness/authorization preflight requires a sealed `completed` receipt bound to the exact source identity key and source generation, a uniquely re-observed `hardware_immutable` medium, and no active ingest. It carries a capacity-inferred allowlisted generic FAT/FAT32/exFAT profile; tokens are short-lived, native-memory-only, and single-use. No UI path, disk number, or provider arguments cross IPC. Windows has an in-process WMI `MSFT_Volume.Format` provider with opaque object-path/capacity revalidation, non-forced quick format, remount validation, marker restoration, and a local format receipt; the live sacrificial card passed only its non-destructive binding probe. macOS has a source-level fixed-argument `diskutil` provider, and Linux has a source-level direct UDisks2 D-Bus provider. Non-Windows compilation/runtime and all destructive hardware certification remain pending.
- The native token is additionally bound to the exact completed run ID and backend-selected allowlisted profile ID. A future provider receives neither a webview path nor a free-form filesystem/profile choice; it must consume this single-use token and re-resolve the medium before any destructive call.
- **Owner:** unassigned
- **Risk:** critical and destructive
- **Depends on:** TASK002 device discovery/identity, TASK004 transfer verification/recovery, and TASK009 security/destructive safety
- **Related:** TASK006 SanDisk Professional slot mapping (required when a slot label is shown or used in confirmation)

## Objective

Provide an operator-initiated quick-format action for an ingested SD or microSD card on Windows, macOS, and Linux. The operation must format only the physical medium that the operator selected, must be impossible to start before the corresponding ingest is durably verified, and must leave the medium mounted, writable, and conforming to an explicitly selected compatibility profile.

This task does **not** provide secure erasure, forensic sanitization, or a generic disk-management UI. An explicitly registered card may opt into format-after-a-fully-verified ingest, but it is subject to every identity, receipt, provider, and post-format validation gate below. A quick format removes filesystem metadata but does not erase prior file contents; the SD Association distinguishes it from the much slower overwrite format on that basis ([SDA Formatter FAQ](https://www.sdcard.org/downloads/formatter/faq/)).

The verified-ingest path now creates a compact root-level `.media-ingest-device-id` marker when the source is writable. The planned per-card registration flow will own the marker/label and format-continuity setting. A real format provider must restore the exact registered marker only after it has revalidated the exact physical medium and post-format filesystem; the marker itself is mutable, copyable filesystem evidence and can never satisfy the format identity gate. Failure to restore it must leave the successful format visible but report continuity restoration as failed, never silently create a different identity.

## Research conclusions

1. SD media should use an SD-aware formatter when possible. The SD Association says its formatter conforms to the SD File System Specification, preserves the SD protected area, and is preferred over generic OS formatters because generic formatting may reduce card performance ([SDA formatter overview](https://www.sdcard.org/downloads/formatter/)).
2. The published formatter downloads are end-user tools, not a documented integration SDK, and this research found no redistribution or automation grant. The Linux edition does provide the `format_sd` command-line tool, requires a whole unmounted device, and defaults to quick format ([Linux user manual](https://www.sdcard.org/pdf/SD-Card-Formatter-Linux-User-Manual-EN-v1.04.pdf)). Its EULA limits copying, redistribution, modification, and third-party service use, so no edition may be bundled or silently installed without legal approval and an explicit distribution decision ([SDA Linux EULA](https://www.sdcard.org/downloads/sd-memory-card-formatter-for-linux/sd-memory-card-formatter-for-linux-arm64-download/)).
3. A built-in cross-platform baseline therefore needs OS-native providers. This baseline must be described honestly as **OS-native quick format**, not as SDA-optimized or universally camera-ready.
4. SD capacity families imply the interoperability filesystem: SD uses FAT12/16, SDHC uses FAT32, and SDXC/SDUC use exFAT ([SDA speed-class table](https://www.sdcard.org/consumers/about-sd-memory-card-choices/speed-class-standards-for-video-recording/)). Capacity alone is only an inference when the reader does not expose the card's SD identity registers.
5. "Ready for use" is a verified postcondition, not a process exit code. Camera-specific readiness requires a named camera profile and a real format/record/playback test on that model and firmware. Otherwise the UI may say only "Formatted and writable" and should recommend an in-camera format before a critical shoot.

## Scope

### Required

- Quick-format one selected removable **medium**, preserving the distinction between reader, slot, whole disk, partition, volume, and card identity established by TASK002.
- Support the repository's Windows, macOS, and Linux targets through separate native providers behind one typed domain operation.
- Support FAT/FAT32/exFAT profiles appropriate to the target device; use filesystem-default allocation-unit sizing unless a camera profile has certified another value.
- Elevate only the destructive helper/provider operation and only after the user confirms the exact re-resolved target.
- Report progress, cancellation semantics, result, and a durable non-sensitive receipt.
- Re-enumerate and validate the new filesystem after formatting.
- Carry a registered card's saved app-marker continuity record through a user-confirmed format and restore it only after the post-format validation succeeds.

### Excluded

- Secure wipe or guarantees that old data is unrecoverable.
- Formatting internal, boot, system, recovery, encrypted, RAID, virtual, network, or destination media.
- Formatting a reader as a unit when it exposes multiple logical slots.
- Running multiple format operations as a batch.
- Claiming support for an unknown camera, reader revision, or filesystem combination without hardware evidence.

## Safety contract

The backend owns the safety decision. The React UI supplies an opaque selection token and confirmation response; it never supplies a raw drive letter, BSD disk name, `/dev/sdX` path, command line, or unrestricted filesystem string.

The state machine is:

`detected -> ingesting -> copied -> verified -> format-offered -> confirmation-bound -> exclusive -> formatting -> remounted -> validated`

Any identity change, device-removal event, failed check, timeout, app restart, privilege-boundary transition, or unexpected state invalidates the confirmation and returns to a non-destructive state.

Before confirmation, show:

- media make/model when available, capacity, filesystem, volume label, stable media-ID suffix and confidence;
- reader name and physical slot label/confidence when TASK006 supports it;
- source mount and destination display names as secondary information only;
- completed ingest job ID, verification method, verification completion time, file count, and verified byte count;
- the exact filesystem/profile and the statement that quick format does not securely erase file contents.

The destructive call is allowed only if every guard passes immediately before and again inside the elevated provider:

1. TASK004 has a successful, current verification receipt for every selected source file, and no file is pending, changed, skipped, conflicted, or failed.
2. The current medium resolves from TASK002's opaque identity to exactly one whole disk and the same slot, size, media fingerprint, reader parent, and insertion generation captured by the verification receipt.
3. The source is removable media; it is not internal, boot, system, pagefile/swap, recovery, mounted as a destination, or an ancestor/descendant of any ingest destination.
4. Only the expected partitions/volumes belong to the target whole disk; all are included in the lock/unmount plan.
5. The device is present, writable, healthy enough to accept the operation, not encrypted/locked, and not in use. Write-protect produces a clear non-destructive error.
6. A per-medium exclusive lease blocks new ingest, verify, eject, and format actions. A lease on a multi-slot reader must not block a sibling slot unless the OS/provider operation affects the whole reader.
7. The user performs a fresh explicit confirmation. No remembered confirmation, keyboard default, countdown, automatic retry, or `--force` equivalent is allowed.
8. The privileged helper accepts only a signed/structured request containing the expected identity snapshot and approved profile, re-resolves the target itself, and rejects paths or arbitrary arguments from the webview.

If a volume cannot be locked/unmounted because a process has it open, report "card is in use" with a retry action. Never force-close applications or force the format.

## Provider plan

| OS      | Discovery-to-target binding                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Destructive provider                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Privilege and exclusivity                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Required post-check                                                                                                                                                                                         |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | Resolve the TASK002 disk identity to a current `MSFT_Disk`/partition/volume object and volume GUID path; disk numbers and drive letters are display/session data only.                                                                                                                                                                                                                                                                                                                              | Use the documented Windows Storage Management API. `MSFT_Volume.Format` supports FAT, FAT32, exFAT and a `Full = FALSE` quick format; a reset-layout flow, if later approved, uses `MSFT_Disk` partition APIs ([MSFT_Volume.Format](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/format-msft-volume), [MSFT_Disk.CreatePartition](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/createpartition-msft-disk)).                                                                               | Just-in-time UAC elevation in a narrow native helper. Acquire exclusive volume access; Windows documents that `FSCTL_LOCK_VOLUME` fails while files are open and that success proves no open files ([FSCTL_LOCK_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_lock_volume)). Do not set the API's `Force` flag.                                                                                                                                                                                | Refresh Storage Management state; assert expected physical disk relation, partition count/layout, filesystem, label, reported capacity, writable mount, and a create/fsync/read/delete sentinel round trip. |
| macOS   | Resolve TASK002 identity to the current whole `DADisk`, use `DADiskCopyWholeDisk`, and bind to `IOMedia`/I/O Registry path; `/dev/diskN` is session-only. Disk Arbitration exposes whole-disk, BSD-name, device-path, GUID, bus, model, and media properties, while warning that keys vary across devices ([Disk Arbitration guide](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ManipulatingDisks/ManipulatingDisks.html)). | Use the system `diskutil` executable by absolute path with a fixed argument schema for the whole disk. Treat its exit status and subsequent Disk Arbitration state as authoritative; never parse localized prose. Use an existing-partition quick reformat by default. A whole-disk erase/repartition is a separately confirmed recovery mode.                                                                                                                                                                                        | Unmount through Disk Arbitration and require its completion callback before formatting. Use a least-privilege signed helper/launch daemon; Apple recommends a separate helper and `SMAppService` for privileged work rather than running the app as root ([Service Management](https://developer.apple.com/documentation/servicemanagement), [Authorization Services](https://developer.apple.com/documentation/security/authorization-services)). The app's macOS sandbox/signing implications must be resolved before implementation. | Wait for Disk Arbitration disappearance/appearance and mount events; re-resolve identity, then assert filesystem/profile, capacity, writable mount, and sentinel round trip.                                |
| Linux   | Resolve through UDisks object paths plus the current block `dev_t`, stable by-id identity where available, sysfs ancestry and kernel disk sequence. Never retain `/dev/sdX` as identity.                                                                                                                                                                                                                                                                                                            | Prefer the UDisks2 D-Bus API, not `udisksctl` (its manual says the CLI is not intended for scripts). `org.freedesktop.UDisks2.Block.Format` supports filesystems and partition tables and documents teardown/no-block behavior ([UDisks Block API](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Block.html), [udisksctl warning](https://storaged.org/doc/udisks2-api/latest/udisksctl.1.html)). If an approved, user-installed SDA `format_sd` is supported, treat it as an optional separate provider. | Let UDisks/polkit authorize the exact block object; authorization varies by object and whether it is considered a system device ([UDisks authorization](https://storaged.org/doc/udisks2-api/latest/udisks-polkit-actions.html)). Unmount every filesystem through UDisks first and keep the application unprivileged. Never call a shell.                                                                                                                                                                                              | Wait for UDisks interface changes and mount; re-resolve by identity/disk sequence, then assert filesystem/profile, capacity, writable mount, and sentinel round trip.                                       |

### Format profiles

Profiles are allowlisted backend data, not free-form UI input:

- `sd-default`: FAT12/16 as selected by the approved provider for true SD media up to 2 GB.
- `sdhc-default`: FAT32 for media over 2 GB through 32 GB.
- `sdxc-default`: exFAT for media over 32 GB through 2 TB.
- `sduc-default`: exFAT for media over 2 TB, only on an OS/reader/camera combination explicitly certified for SDUC.
- `camera:<vendor>:<model>:<profile-version>`: partition scheme, filesystem, label rules, and allocation unit size backed by the camera manufacturer's documentation and hardware certification.

When only capacity is available, mark the family/profile selection as inferred and require confirmation. Do not offer NTFS, ReFS, APFS, ext4, encrypted filesystems, arbitrary cluster sizes, or a custom partition editor in the ingest workflow. The default label is ASCII and constrained to the selected filesystem/provider limit.

## Implementation work packages

1. Define the format request/result/receipt types, state machine, error taxonomy, and destructive-operation lease.
2. Implement the shared safety gate and a privileged-helper request authenticator that re-resolves identity rather than trusting UI paths.
3. Implement the Windows native provider and provider contract tests.
4. Implement the macOS Disk Arbitration plus least-privilege helper provider; document signing, sandbox and minimum-version consequences.
5. Implement the Linux UDisks2/polkit provider. Complete a legal/product decision before adding any optional SDA formatter integration.
6. Implement confirmation, progress, clear failure states, and post-format validation UI.
7. Restore a registered card's marker after the provider has re-identified the exact physical medium and completed all post-format checks; record the restoration result separately from the format result.
8. Add fixtures and destructive hardware tests, then update the certification matrix without merging fixture-only success into hardware certification.

## Acceptance criteria

- A format cannot be offered or invoked until TASK004 has produced a complete successful verification receipt for the current insertion generation.
- The target is re-resolved from opaque stable identity immediately before confirmation and inside the elevated provider; changing a drive letter/device node cannot redirect the operation.
- Internal/system/boot/recovery/destination/virtual/network/encrypted/ambiguous devices are rejected, including when OS removable hints are wrong.
- Removal/reinsertion, card swap in the same slot, reader reconnection, app restart, stale UI, or identity ambiguity invalidates the action without writing.
- Busy or write-protected media fail safely; the app never passes a force flag or terminates another process.
- Only a fresh single-card quick format is supported. A registered card may use the product's opt-in mount-triggered path only after an exact completed receipt, a fresh immutable identity/generation check, post-format validation, and marker-restoration receipt; manual format still requires explicit confirmation. Batch formatting is absent.
- Windows, macOS, and Linux providers create the selected allowed filesystem and return structured operation stages/errors without parsing localized human-readable text.
- After completion the exact same slot contains a newly detected medium/filesystem instance that mounts writable, matches the approved profile, and passes a durable sentinel create/fsync/read/delete check.
- For a registered card, marker restoration occurs only after that successful post-check and only when the re-observed hardware-immutable identity equals the pre-format identity. A missing/changed/ambiguous identity leaves the card formatted but unregistered and requires operator setup; it never restores on slot, mount path, volume ID, or marker evidence alone.
- The receipt records job ID, prior verification receipt ID, non-sensitive media/reader/slot identity hashes, provider and version, profile, start/end timestamps, result, and post-check evidence; it never records full user paths or file contents.
- UI copy says "quick format" and explicitly says old file data may remain recoverable.
- "Camera-ready" appears only for a camera profile that passed the camera certification below; the generic result says "formatted and writable."

## Evidence and certification plan

### Automated and fixture evidence

- Unit tests for every state transition and rejected guard, including property-based tests that mutate one identity field between confirmation and execution.
- Provider-contract fixtures for healthy, RAW, multi-partition, busy, write-protected, encrypted, no-media, disappearing, duplicate-ID, system, and source-equals-destination cases.
- Privileged-helper tests proving arbitrary paths/arguments, replayed tokens, expired confirmations, wrong app identity, and mismatched device snapshots are rejected.
- Filesystem-profile boundary tests at 2 GB, 32 GB, and 2 TB, without allocating equivalent physical storage.
- Recovery tests for process interruption at every provider stage. No recovery path may retry a destructive step automatically.

### Destructive hardware matrix

Use expendable, uniquely labeled cards with backed-up fixtures. For each supported OS/version and reader class, test SDHC and SDXC; add SD/SDUC only when supported hardware is available. Include direct reader connection and hubs/docks, a card with multiple partitions, locked full-size SD, unexpected removal, sleep/wake, reboot, app crash, and another process holding a file open.

For each successful run record:

- pre-operation identity snapshot and verification receipt;
- confirmation target screenshot;
- provider/version/profile and elapsed time;
- partition/filesystem inspection after remount;
- sentinel round-trip result and OS event trace;
- whether the camera profile successfully records, stops, powers off, powers on, plays back, and records again on the exact camera model/firmware.

Maintain separate statuses for automated fixtures, VM/local OS, destructive removable-media hardware, packaged/signed app, and camera hardware. A source build or simulated block device is not destructive-hardware certification.

## Blocking decisions and external resources

- Product/legal approval is required before bundling, downloading, invoking, or benchmarking the proprietary SDA formatter. Until then, implement only the OS-native provider and link users to the official formatter when SD-optimized formatting is required.
- Camera vendors/models/firmware and their required card formats are unspecified. Generic quick format can be planned and tested, but camera-ready profiles remain blocked on that inventory and expendable camera/card hardware.
- macOS privileged-helper design affects app sandboxing, signing, notarization, and minimum supported macOS; decide those packaging constraints before implementation.
- Physical SD/SDHC/SDXC/SDUC cards, a write-protected full-size SD card, supported readers, three OS hosts/VMs with real USB pass-through, and approved destructive test data are required for certification.

## Completion evidence

Not yet available. Record exact test commands, hardware inventory/revisions, screenshots, receipts, and certification results here when implemented.

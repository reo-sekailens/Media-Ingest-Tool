# TASK005 - Safely quick-format verified source media

## Status and ownership

- **Status:** in progress — manual quick format still requires a sealed receipt and `hardware_immutable` identity. Separately, an explicitly accepted managed-card auto-format path requires the same current mount/generation and sealed receipt plus an opt-in marker profile and a matching compact BLAKE3 content witness; it is deliberately copyable-risk continuity evidence, not hardware proof. A first packaged microSD run verified the source but safely skipped formatting because a zero-byte interrupted marker and mutable-marker operation-key transition broke its gates. The installed repair atomically replaces markers, repairs only zero-byte interrupted reserved markers, and retains a stable native/session operation key while using the marker only for profile lookup. The destructive provider, post-format sentinel, marker restoration, and format receipt remain unverified on hardware. Non-Windows compilation/runtime and all destructive hardware certification remain pending.
- A completed run's UI action now rechecks native eligibility on every click, instead of remaining disabled by a stale webview result. The explicitly confirmed recovery action bypasses receipt, managed-card registration, and identity continuity after `FORCE REFORMAT`; it retains only the non-bypassable current removable-target, active-ingest, native profile/provider, single-use authorization, remount, and post-format validation gates.
- Any Force Reformat rejection now stays visible within the open confirmation dialog, so the operator can distinguish an identity, registration, current-mount, or provider gate from a no-op. The dialog retains its retry and cancel controls; this presentation change does not relax a destructive-operation prerequisite.
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

| OS      | Discovery-to-target binding                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Destructive provider                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Privilege and exclusivity                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Required post-check                                                                                                                                                                                         |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | Resolve the TASK002 disk identity to a current `MSFT_Disk`/partition/volume object and volume GUID path; disk numbers and drive letters are display/session data only.                                                                                                                                                                                                                                                                                                                                                                                                                  | Use the documented Windows Storage Management API. `MSFT_Volume.Format` supports FAT, FAT32, exFAT and a `Full = FALSE` quick format; a reset-layout flow, if later approved, uses `MSFT_Disk` partition APIs ([MSFT_Volume.Format](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/format-msft-volume), [MSFT_Disk.CreatePartition](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/createpartition-msft-disk)).                                                                               | Just-in-time UAC elevation in a narrow native helper. Acquire exclusive volume access; Windows documents that `FSCTL_LOCK_VOLUME` fails while files are open and that success proves no open files ([FSCTL_LOCK_VOLUME](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_lock_volume)). Do not set the API's `Force` flag.                                                                                                                                         | Refresh Storage Management state; assert expected physical disk relation, partition count/layout, filesystem, label, reported capacity, writable mount, and a create/fsync/read/delete sentinel round trip. |
| macOS   | No macOS target can currently resolve for formatting. This is intentional: a filesystem-scoped UUID, capacity, mount path, or `diskutil` identifier cannot prove the selected physical medium. A future provider must bind the current whole `DADisk` with `DADiskCopyDescription`/`DADiskCopyWholeDisk` and I/O Registry evidence; Apple documents that Disk Arbitration keys vary by device ([Disk Arbitration guide](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ManipulatingDisks/ManipulatingDisks.html)). | Direct `diskutil eraseVolume` was removed because it could erase media while the old mount path later failed validation after a rename/remount. No quick-format command is issued until the helper returns the newly bound mount.                                                                                                                                                                                                                                                                                                     | A signed least-privilege helper/launch daemon must authenticate its caller, validate the current `DADisk`/IOMedia target before and after unmount, and receive one opaque authorization. Apple recommends a separate helper and `SMAppService` for privileged work rather than running the app as root ([Service Management](https://developer.apple.com/documentation/servicemanagement), [Authorization Services](https://developer.apple.com/documentation/security/authorization-services)). | Wait for Disk Arbitration disappearance/appearance and mount events; re-resolve identity, then assert filesystem/profile, capacity, writable mount, and sentinel round trip.                                |
| Linux   | Resolve through UDisks object paths plus the current block `dev_t`, stable by-id identity where available, sysfs ancestry and kernel disk sequence. Never retain `/dev/sdX` as identity.                                                                                                                                                                                                                                                                                                                                                                                                | Prefer the UDisks2 D-Bus API, not `udisksctl` (its manual says the CLI is not intended for scripts). `org.freedesktop.UDisks2.Block.Format` supports filesystems and partition tables and documents teardown/no-block behavior ([UDisks Block API](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Block.html), [udisksctl warning](https://storaged.org/doc/udisks2-api/latest/udisksctl.1.html)). If an approved, user-installed SDA `format_sd` is supported, treat it as an optional separate provider. | Let UDisks/polkit authorize the exact block object; authorization varies by object and whether it is considered a system device ([UDisks authorization](https://storaged.org/doc/udisks2-api/latest/udisks-polkit-actions.html)). Unmount every filesystem through UDisks first and keep the application unprivileged. Never call a shell.                                                                                                                                                       | Wait for UDisks interface changes and mount; re-resolve by identity/disk sequence, then assert filesystem/profile, capacity, writable mount, and sentinel round trip.                                       |

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

2026-08-27 capability repair: `execute_format_authorization` was present in
the invoke handler but missing from `build.rs`'s generated command manifest
and the main window capability. Both now include it, so a confirmed in-app
format request can reach the native handler. Provider/marker tests, production
frontend build, and memory-bank validation pass. The live Tauri dev window is
currently serving preview fixtures rather than its native removable-device
snapshot, so no destructive button-path certification is claimed and M:
remains unchanged.

2026-08-27 live-provider repair: the frontend used the private
`__TAURI_INTERNALS__` probe, so a real Tauri 2 dev window displayed browser
fixtures. It now uses public `isTauri()` and rendered the live 511.8 GB M:
card through the format confirmation. The non-destructive WMI input probe now
passes with portable exFAT/quick/non-forced fields and `RunAsJob=false`.
Windows still returned before its old mount transitioned, so validation now
requires that disappearance before accepting a remount. No destructive format
receipt exists yet.

2026-08-27 repair evidence: verified removable M: exFAT storage reported `Full
Repair Needed`. Windows PowerShell `Repair-Volume` does not support exFAT, so
`chkdsk M: /f` was used after the app released its open handle. It repaired a
volume-bitmap corruption and one cross-linked controlled MP4; fresh inspection
reports exFAT `Healthy` / `OK`. This is not an application format receipt.

2026-08-27 source/package readiness: the Windows WMI provider now retains the
failure stage for rejected input, missing output, or unreadable return value;
the marker sibling writer deletes a failed temporary record. Focused provider
and marker Rust tests, formatting, and `cargo check` pass. The resulting NSIS
package SHA-256 is `E96A81C4AF07023B32AABDE6D136296389956D612FFE0624BA19644616CD2996`.
Physical destructive proof remains pending a fresh confirmed remount.

2026-08-27 source/fixture evidence: focused Rust authorization tests (3), `cargo fmt --check`, `cargo check`, Prettier, strict TypeScript, and 11 UI fixtures passed. The UI fixture proves only the opaque token confirmation protocol; it is not a provider or destructive-media test. The connected SanDisk PRO-READER cards at D: and M: have unresolved identity, so the app withheld the action and no destructive format was attempted. A package run on a sacrificial card with current hardware-immutable identity is required.

2026-08-27 live blocker: the confirmed M: microSD reached the Windows Storage
Management provider, which returned `43006` for the exact exFAT quick-format
request: the volume is read-only. Disk 5 itself remains online, healthy, and
not read-only; no filesystem content changed. The implementation maps that
provider-specific code to the write-protection message, waits for any returned
storage job, and rejects an unchanged marker as format proof. Manual and
auto-format destructive certification remain blocked until Windows accepts a
write to this media/provider path.

2026-08-27 compatible native-provider evidence: direct Windows
`Format-Volume` quick formatting completed on the same exact M: volume using
exFAT. The output volume is healthy and contains no old card marker or media
folders. The application provider now uses WMI solely to bind/revalidate the
opaque exact target, then calls the supported noninteractive `Format-Volume`
cmdlet with a WMI-sourced single-letter drive and allowlisted filesystem. The
live command-path proof is complete; app-owned receipt/marker restoration and
auto-ingest destructive runs are still required.

2026-08-27 completed automatic destructive run: run
`217d7fdd-cf17-45dd-b587-d81507d2e605` on the exact managed 511.8 GB M:
microSD verified controlled media, sealed the ingest receipt, completed the
WMI-bound native `Format-Volume` exFAT provider, restored the marker, and
wrote the `sdxc-default` format receipt. The manual token-confirmation command
path is still a distinct certification gap; macOS, Linux, camera compatibility,
and hostile-media matrix cases remain open.

2026-08-27 release-binary repetition: the x64 release executable from NSIS
bundle SHA-256 `AE3C98F30A5E1ED116496DC5DDD678E501EFAEC8F07F8370E3AA94840940A305`
completed registered automatic run `cf3048a9-a70f-4edc-9b2d-2afc3e175009` on
the live M: microSD. The sealed receipt, `sdxc-default` format receipt, exact
healthy exFAT capacity, and restored marker were read back from the local
ledger and volume. This is release-binary evidence, not a clean-machine
installer or manual-confirmation-path certification.

2026-08-27 empty-remount backstop: native automatic ingest now detects a
zero-file plan before it persists a run and returns `skipped`; this prevents a
post-format remount race from producing an empty failed receipt attempt. It is
covered by focused Rust tests and does not alter manual ingestion.

2026-08-27 installed-package check: NSIS installer SHA-256
`D27925F12445C80ED9F0DD0FBE6443818C8A783338F7FD9F0C14F6F9F9B0EDA1` completed
silently with exit code 0. Its installed executable launched successfully and
read the retained managed marker, completed automatic format receipt, healthy
exFAT status, and exact M: capacity. This is current-machine install evidence;
manual confirmation and clean-machine certification remain separate.

2026-08-28 recovery exception: the operator UI now exposes a separately
labelled Force Reformat route for a current registered hardware-stable card
whose normal managed-witness/receipt relationship is stale. It requires the
exact typed phrase `FORCE REFORMAT`, preserves the 60-second single-use token,
native target/profile resolution, no-active-ingest check, remount validation,
and sentinel I/O. It deliberately bypasses only receipt continuity and thus
does not create a receipt-bound format row. This is source/fixture coverage;
no destructive hardware test has exercised the exception.

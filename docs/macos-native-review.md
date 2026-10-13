# macOS native compatibility review

Reviewed on 2026-10-13 for the manual-install Intel (`x86_64-apple-darwin`)
and Apple Silicon (`aarch64-apple-darwin`) builds. This document covers native
discovery, Disk Arbitration, eject, and the destructive-format boundary.
Packaging and manual-install instructions have their own evidence records.
Documentation review establishes API requirements; it cannot certify actual
cards, readers, macOS privacy prompts, or camera readiness.

## Evidence and implementation

| Boundary                          | Primary documentation                                                                                                                                                                                                                                                                                                 | Source result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Architecture                      | [Rust macOS target support](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html)                                                                                                                                                                                                                       | Both native targets exist; ARM64 is currently Tier 1 and Intel Tier 2. Native macOS SDK/tooling is needed for this app's framework and C dependencies. The macOS `cfg` selects the same discovery/eject policy for both architectures.                                                                                                                                                                                                                                                                                                                                                          |
| FFI registration                  | [Apple's DiskArbitration header](https://github.com/apple-oss-distributions/DiskArbitration/blob/main/DiskArbitration/DiskArbitration.h) and [description-changed API](<https://developer.apple.com/documentation/diskarbitration/daregisterdiskdescriptionchangedcallback(_:_:_:_:_:)>)                              | Fixed a five-argument ABI mismatch: the declaration and call now include separate matching-dictionary and watched-keys arguments before callback/context. Null watched keys subscribe to all changes. The old four-argument declaration could misinterpret the callback/context pointers on both architectures.                                                                                                                                                                                                                                                                                 |
| FFI scalar types                  | [Apple CFRunLoop header](https://github.com/apple-oss-distributions/CF/blob/main/CFRunLoop.h), [CFBase header](https://github.com/apple-oss-distributions/CF/blob/main/CFBase.h), and [Rust external blocks](https://doc.rust-lang.org/reference/items/external-blocks.html)                                          | `CFRunLoopRunInMode` uses CoreFoundation's unsigned-byte `Boolean`; the Rust argument is now `u8`. Object handles remain opaque pointers and the return code is `i32`, matching Apple's `SInt32`.                                                                                                                                                                                                                                                                                                                                                                                               |
| Callback lifecycle                | [Disk Arbitration notifications](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ArbitrationBasics/ArbitrationBasics.html)                                                                                                                        | Registers before the first watcher snapshot, schedules on a dedicated run loop, coalesces callback wakeups, and reconciles fresh native state. Callbacks never select an ingest or destructive target. Unschedules before releasing the session/context. The periodic snapshot is a missed-event fallback.                                                                                                                                                                                                                                                                                      |
| Ownership and shutdown            | [CoreFoundation ownership](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFMemoryMgmt/Concepts/Ownership.html) and [run-loop thread safety](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/RunLoopManagement/RunLoopManagement.html) | `DASessionCreate` is released. The borrowed `CFRunLoopGetCurrent` result is now retained while shared with the subscription, then released when shared state drops after joining the worker. This prevents shutdown from using a run-loop pointer after its worker exits. Configuration remains on the owning thread; stop/wake happen through CoreFoundation.                                                                                                                                                                                                                                  |
| Mounted-volume projection         | [Apple's disk-description and IOMedia guidance](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ManipulatingDisks/ManipulatingDisks.html)                                                                                                         | Structured `diskutil list -plist`/`info -plist` remains the volume adapter. Nested partitions/APFS volumes are visited. A candidate requires explicit capacity, BSD identifier, internal/external classification, removable/external evidence, existing mount, and no explicit virtual classification. The fresh info identifier and mount must match the list snapshot; a remount or identifier mismatch is rejected. Missing `Internal` evidence fails closed. `WritableVolume` does not gate reading. Actual root-directory access selects ready versus permission-denied/unavailable state. |
| Eject                             | [Apple whole-disk eject requirements](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ManipulatingDisks/ManipulatingDisks.html#//apple_ref/doc/uid/TP40009310-CH3-SW8)                                                                            | The backend supplies a freshly resolved mount and the provider canonicalizes it under `/Volumes`. The absolute `/usr/sbin/diskutil` binary receives `eject` plus one mount argument, without a shell or force flag. The unnecessary `-plist` argument was removed; the provider checks exit status and ignores localized stdout. Diskutil handles unmount/eject policy. Real busy-media, sibling-volume and reader behavior remain unverified.                                                                                                                                                  |
| Privacy                           | [Removable-volume usage description](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription) and [Apple file-access controls](https://support.apple.com/en-ie/guide/security/secddd1d86a6/web)                                                        | Files & Folders consent applies independently of distribution/signing. The OS may prompt before reading removable-card files, including the continuity marker. Marker-read errors do not create trusted identity; actual denied file I/O must fail rather than seal a successful receipt. Native privacy-prompt behavior requires a real installation.                                                                                                                                                                                                                                          |
| Apple Silicon accessory admission | [Apple accessory consent](https://support.apple.com/en-gb/102282)                                                                                                                                                                                                                                                     | On Apple Silicon laptops, the user may need to unlock the Mac and approve a new USB/Thunderbolt accessory or SD card before it appears. Discovery cannot bypass this OS control.                                                                                                                                                                                                                                                                                                                                                                                                                |
| Sandboxing                        | [Disk Arbitration sandbox limitation](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/Introduction/Introduction.html)                                                                                                                             | This review targets the manual-install utility distribution. A future sandboxed distribution needs a separate security-scoped-access design and tests; notification registration alone does not establish permission to read a newly inserted volume.                                                                                                                                                                                                                                                                                                                                           |
| Destructive format                | [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice) and [Apple factored authorization](https://developer.apple.com/library/archive/documentation/Security/Conceptual/authorization_concepts/03authtasks/authtasks.html)                                                          | The provider remains intentionally unavailable and returns `UnsupportedPlatform` before target resolution or media mutation. There is no privileged helper in this build. A future provider needs authenticated narrow IPC, just-in-time authorization, exact current whole-medium binding, and validated remount/sentinel checks. Manual installation or ad-hoc signing does not implement these prerequisites.                                                                                                                                                                                |

Apple's published Disk Arbitration/CoreFoundation conceptual guides are archived,
and the public CF source is older. Their requirements were checked against the
current Apple API pages and available Apple headers. Installed-SDK/header and
utility behavior must also be checked by native runners; the archive publication
date does not establish a new macOS runtime result.

## Verification added

- A macOS native test creates/registers/schedules a Disk Arbitration session,
  receives an initial callback within five seconds, and tears it down. It performs
  no mount, unmount, eject, or format operation.
- A callback test covers description changes, coalesced appeared/disappeared
  callbacks during reconciliation, and the required follow-up request.
- A read-only native `diskutil` test checks the installed utility's system-volume
  info and list XML schemas. The system volume remains excluded from ingest.
- Structured fixtures cover partitions/APFS traversal, matching current
  identifier/mount, virtual/internal rejection, and missing capacity,
  internal/external or identifier evidence. False or absent `WritableVolume`
  does not exclude a readable source. A real directory fixture projects a
  read-only-volume plist to `Ready` while preserving its file and creating no
  marker; injected read denial remains non-ingestable. This fixture does not
  emulate a physically locked card or macOS TCC.

These native tests are compiled only on macOS. A Windows test pass does not run
them. Their actual hosted results belong to the source-specific CI evidence,
not this list of test additions.

On the Windows review host, scoped `rustfmt --check`, Prettier and `git diff
--check` pass. The Disk Arbitration module and its tests also pass Rust 1.98.0
metadata type-checking for both Apple targets using `rustc --edition 2021 --test
--emit=metadata --target <target> src-tauri/src/macos_disk_arbitration.rs`.
This checks target-specific Rust types and declarations; it does not link Apple
frameworks, compare against installed SDK headers, or execute the tests.

Six discovery adapter tests also pass in a temporary Windows projection harness
using copies of the production identity, marker and discovery modules, changing
only discovery's platform `cfg` selectors to enable the macOS adapter. The native
`diskutil` test was excluded. This verifies parser/projection logic, source-file
preservation, marker absence, disappeared-root handling and read-error states on
the Windows filesystem; it does not execute a Mac utility, mount a read-only
volume, or reproduce TCC. The harness's strict all-target Clippy check passes.

## Runner-discovered virtual-source correction

[Quality run 38343853933](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38343853933)
for source `064700eeb6e25c1d4998c2643768d55849657a64` produced sanitized
read-only admission evidence on all four UI runners. macOS 15 Intel reported six
mounted candidates and one admitted image; macOS 15 ARM64 reported nine mounted
candidates and one admitted image; macOS 26 Intel reported 14 mounted candidates
and nine admitted images; macOS 26 ARM64 reported 22 mounted candidates and 14
admitted images. Every admitted entry was APFS and reported `BusProtocol = Disk Image`,
`Internal = false`, removable/ejectable booleans true, and no
`VirtualOrPhysical` field. No system-root candidate was admitted. These are
volume classifications from the installed utility; no volume labels, mount
paths, UUIDs, disk identifiers, serials, capacities or raw logs were retained in
the diagnostic JSON or copied here.

The source adapter previously rejected only a `VirtualOrPhysical = Virtual`
string, so these images bypassed its intended virtual-media exclusion. It now
also rejects the exact case-insensitive `BusProtocol = Disk Image` enum. A
runner-shaped regression keeps the absent virtual field and read-only/removable
flags, rejects the image, then substitutes `USB` and proves the readable
read-only physical-source policy remains eligible. Diagnostic policy mirrors
this additional reason in native validation. The fix does not impose
an empty-inventory expectation or hide other external volumes.

[Final native run 38351288268](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38351288268)
at `0984bc7` passes the core adapter/Disk Arbitration tests on all four native
Mac15/26 Intel/ARM hosts. Native UI snapshots admit zero media, and real inventory
IPC and refresh agree with that empty state. The production adapter is unchanged
from the verified `9e4c4fa` DMGs. These observed counts vary by runner/run and do
not certify any physical card, card lifecycle or permission prompt.

## Read-only source flow

The former writable-volume check conflated reading with writing and excluded
locked SD cards. Source copy uses Rust's [read-only `File::open`](https://doc.rust-lang.org/std/fs/struct.File.html#method.open).
Discovery now attempts [root directory reading](https://doc.rust-lang.org/std/fs/fn.read_dir.html)
without creating a file; it checks opening and the first iterator result because
iteration can fail separately. This is a current directory-access observation,
not proof that every nested directory or media file remains readable. Inventory
and copying still check each actual file, and denied I/O cannot seal a receipt.

`DiscoveredVolume::Ready` becomes `DeviceState::Available`, and the UI permits
manual ingest for a mounted available source. Permission-denied discovery becomes
`Busy`, so the existing UI/native selection gates block ingest. A completed
verified run seals its receipt before trying the optional continuity marker;
marker-writing failure reports `SourceMarkerStatus::Unavailable` while preserving
the verified result. Session configuration therefore remains usable on a locked
card. New on-card registration still requires a successful marker creation and
reports a writable-card error if it cannot create one. An existing readable
marker can select previously saved convenience settings; it does not upgrade
hardware identity. macOS format resolution still returns `UnsupportedPlatform`.

## Remaining certification boundaries

- Native IOMedia/IOKit parent topology, per-card immutable hardware identity, and
  named multi-slot-reader mapping are not implemented by the structured volume
  fallback. Filesystem UUID, mount location, and app marker remain weaker evidence.
- Readable write-protected/read-only sources are no longer excluded by a write
  flag. Actual locked-card ingest, optional marker failure reporting, and
  Files & Folders consent require physical/installed macOS verification.
- USB reader/card insert, swap, disconnect, sleep/wake, sustained copy throughput,
  destination file I/O, TCC accept/deny, and native packaged UI require real Mac
  hardware. A runner's system volume cannot certify an SD reader or external
  filesystem.
- Safe eject of busy media, multi-partition media, and sibling cards needs physical
  checks. Whole-disk eject can affect sibling volumes on that same disk. Current
  source selection is volume-based, so broad multi-volume eject certification is
  withheld until provider ownership and sibling-use checks are exercised.
- In-app quick format remains unavailable on macOS. Use an explicitly chosen
  external/in-camera workflow after independent ingest verification. Do not run
  the GUI as root or install an improvised destructive helper.

Rollback is limited to these source changes; no card, device, permission database,
helper registration, or macOS system configuration was mutated by this review.

# TASK002 — Cross-platform storage discovery, details, and identity

- **Status:** In progress — Windows read-only removable-volume enumeration, event-driven Configuration Manager disk-interface monitoring, storage-descriptor/physical-device/SCSI-LUN topology query, bounded VPD page 0x83 parsing, compact app-marker discovery, and conservative identity normalization are implemented and locally unit-tested. On a direct native Windows SD stack only, discovery now attempts the documented CMD2 CID read and promotes a valid 16-byte response to `hardware_immutable`; USB mass-storage readers remain unresolved when they cannot forward the command. Each new verified writable input card now receives a <=128-byte `MIT2` record with a random managed-card token and path-free BLAKE3 content witness. That record remains mutable/copyable filesystem evidence: VPD and markers can never unlock automatic recall or formatting. A Linux kernel-interface adapter now reads `/proc/self/mountinfo`, `/sys/class/block`, and filesystem space directly without shell parsing, but has only parser/unit coverage from this Windows host; direct-CID runtime/hardware evidence, Linux runtime/hardware evidence, and macOS support remain pending.
- **Owner:** Unassigned
- **Priority:** P0
- **Depends on:** TASK001 desktop foundation and shared IPC contracts
- **Blocks:** Per-device destination persistence, ingest, camera/card association, formatting, and SanDisk reader-slot support

## Objective

Build a native Rust discovery service for Windows, macOS, and Linux that:

1. enumerates removable/external storage already present at app startup;
2. reconciles insert, mount, unmount, eject, and surprise-removal events;
3. models the physical reader, removable medium, partitions, volumes, and mount points as distinct nodes;
4. shows useful device and volume details; and
5. assigns the strongest identity the hardware/OS actually exposes, with an explicit confidence and limitations record.

No single property is universally immutable across all storage hardware and operating systems. Some USB readers expose only the reader serial, some media expose no serial/WWN, filesystem UUIDs can be reformatted or cloned, and OS names are reallocated. The implementation must never rename a weak identifier “immutable.” When no stable medium-level identifier exists, require explicit user confirmation before applying a remembered destination or destructive action.

## Research conclusions and invariants

### Identity is a tuple with provenance, not one magic string

Persist normalized identity evidence and derive an opaque application key from a versioned canonical encoding. Hashing makes a safe key; it does **not** improve the source’s reliability.

| Confidence          | Suitable evidence                                                                                                                    | Permitted use                                                                      |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| `hardware_strong`   | Standards-based device identifier such as WWN/NAA/EUI/VPD identifier or an OS-provided device GUID demonstrably bound to the medium  | Automatic per-medium settings after collision checks                               |
| `hardware_reported` | Manufacturer/transport serial plus vendor, product/model, and capacity; may be absent, duplicated, reader-bound, or firmware-defined | Automatic settings only after proving it follows the medium rather than the reader |
| `filesystem`        | Filesystem UUID and partition UUID/GUID                                                                                              | Reconnect aid only; warn that format changes it and cloning can duplicate it       |
| `topology`          | Reader identity plus physical bus/port/interface/slot path                                                                           | Identifies where media is inserted, not which card it is                           |
| `session`           | Drive letter, volume GUID path, `PhysicalDriveN`, BSD name, Linux devnode/devpath, IOKit registry-entry ID                           | Current-session correlation only; never persisted as medium identity               |
| `ambiguous`         | Missing identifiers or a collision among present devices                                                                             | No automatic destination/format selection; ask the user                            |

Maintain separate keys:

- `reader_key`: the external reader/enclosure identity, when available;
- `slot_key`: reader plus stable physical interface/topology evidence, when available;
- `medium_key`: identity evidence that follows the inserted card/device;
- `volume_key`: filesystem/partition identity;
- `session_key`: generation-scoped OS handle/name for event correlation only.

This separation is mandatory for multi-slot readers: a reader USB serial identifies the reader, not either inserted card. A filesystem UUID identifies formatted contents, not immutable hardware. Linux util-linux explicitly warns that device names are unstable, `/dev/disk/by-id` semantics depend on udev/hardware, and UUIDs can be duplicated by copying media ([upstream util-linux `mount(8)`](https://github.com/util-linux/util-linux/blob/master/sys-utils/mount.8.adoc#indicating-the-device-and-filesystem)). Microsoft likewise documents that DUID composition and availability vary by transport/firmware and that media-vs-device distinction matters for removable-media drives ([Microsoft storage DUIDs](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/device-unique-identifiers--duids--for-storage-devices)).

### Device graph, not a flat drive list

The common model must represent:

```text
host connection -> reader/enclosure -> physical medium -> whole disk
                                              -> partition -> filesystem/volume -> mount point(s)
```

One physical disk can have multiple volumes; one volume can span disks; a reader can remain present with no medium; a device can appear before its filesystem is mounted; mount points can change without physical removal. Identity resolution must operate at the correct node and preserve parent/child relationships.

## Common data contract

The shared device snapshot must support at least:

- session generation, node IDs, parent IDs, node kind, present state, discovery/update timestamps;
- display name, vendor, product/model, revision, bus/transport, external/internal, removable, ejectable, writable/read-only;
- whole-disk/partition relation, partition number/type/GUID where available;
- capacity, block/logical sector size, filesystem type, label, total/free/available bytes, mount point list;
- OS device path/name for diagnostics, clearly marked session-only;
- raw identity candidates with source/provenance, normalized value, scope (`reader`, `medium`, `partition`, `filesystem`, `topology`, `session`), and confidence;
- derived versioned `reader_key`, `slot_key`, `medium_key`, and `volume_key`, plus collision/ambiguity flags;
- access state (`ready`, `unmounted`, `empty_reader`, `unsupported_fs`, `permission_denied`, `busy`, `removed`, `error`) and actionable error code;
- USB VID/PID, reader serial/container ID, physical port/interface/path evidence when the OS makes it available, for the dedicated reader-slot task.

Normalize identifiers conservatively: trim transport padding/NULs, preserve case-insensitive comparison semantics where specified, retain the original value for diagnostics, namespace by source/type, and never concatenate unescaped ambiguous strings. The opaque key must use a versioned canonical encoding so normalization changes can be migrated.

## Native implementation plan

### Windows

Use Win32 Configuration Manager/SetupAPI plus storage/volume IOCTLs. Do not make WMI polling or drive-letter polling the primary detector.

1. Register `CM_Register_Notification` for the appropriate storage **device interface class** (for example `GUID_DEVINTERFACE_DISK`) before the initial enumeration, then enumerate existing interfaces using `CM_Get_Device_Interface_List` or SetupAPI. Microsoft explicitly prescribes register-then-enumerate and warns an arrival can appear in both paths, so events must be deduplicated ([arrival/removal guidance](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/registering-for-notification-of-device-interface-arrival-and-device-removal), [interface-class distinction](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/comparison-of-setup-classes-and-interface-classes)).
2. Keep callbacks minimal: copy identifiers/action into a bounded queue and reconcile on a worker. Handle query-remove by releasing device handles; Microsoft warns retained handles can prevent orderly removal ([arrival/removal guidance](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/registering-for-notification-of-device-interface-arrival-and-device-removal)).
3. Walk the PnP parent relation with `CM_Get_Parent`/`DEVPKEY_Device_Parent` to distinguish disk, USB mass-storage interface, reader/container, and physical connection ([`CM_Get_Parent`](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_get_parent), [`DEVPKEY_Device_Parent`](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-parent)). Capture device instance/container/location properties as topology/session evidence, not automatically as medium identity.
4. Open device interfaces for read-only/query enrichment and use `IOCTL_STORAGE_QUERY_PROPERTY`:
   - `StorageDeviceIdProperty`/`STORAGE_DEVICE_ID_DESCRIPTOR` for VPD identifiers when supplied;
   - `StorageDeviceProperty`/`STORAGE_DEVICE_DESCRIPTOR` for vendor, product, revision, transport, removable-media bit, and serial;
   - optionally probe `StorageDeviceUniqueIdProperty` only when available and document its transport/driver limits.
     Microsoft documents the property IDs and returned structures ([`STORAGE_PROPERTY_ID`](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ne-winioctl-storage_property_id), [`STORAGE_DEVICE_DESCRIPTOR`](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ns-winioctl-storage_device_descriptor), [storage DUID limitations](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/device-unique-identifiers--duids--for-storage-devices)).
5. Enumerate volumes independently, map volume(s) to physical disk(s) with `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`, then query label/filesystem/format-created volume serial and free-space fields. The volume serial returned by `GetVolumeInformationW` is assigned at format time, not the manufacturer serial ([volume-to-disk extents](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_volume_get_volume_disk_extents), [`GetVolumeInformationW`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getvolumeinformationw)).
6. Prefer unprivileged Configuration Manager/SetupAPI discovery. Treat direct physical-drive handles as optional enrichment and design for access denied: Microsoft documents restrictions on direct disk/volume access and notes zero desired access can query attributes in some scenarios ([`CreateFile`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew#physical-disks-and-volumes)). Discovery must still return a useful, lower-confidence record without elevation.

Windows session identifiers (`PhysicalDriveN`, drive letters, volume mount paths, device interface strings) are never medium keys. Surprise removal must invalidate the generation and close all handles before any queued copy/format work proceeds.

### macOS

Use Disk Arbitration for disk/media lifecycle and IOKit/IOUSBHost for the physical parent topology.

1. Create a `DASession`, register appeared, description-changed, and disappeared callbacks, and schedule it on a dedicated dispatch queue. Apple documents these callbacks and warns that unusual media/removal scenarios can produce different callback ordering, so reconcile rather than assume a fixed event sequence ([Disk Arbitration notifications](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ArbitrationBasics/ArbitrationBasics.html)).
2. For each `DADisk`, take `DADiskCopyDescription`, resolve the whole disk, and capture current keys for vendor/model/revision/protocol/path, device/media GUID, BSD name, size/block size, removable/ejectable/writable, volume kind/name/UUID/path and mountability. Apple lists the supported description keys and states callback descriptions are current as of the event ([Disk Arbitration constants](https://developer.apple.com/documentation/diskarbitration/diskarbitration-constants), [DADiskCopyDescription](https://developer.apple.com/documentation/diskarbitration/dadiskcopydescription%28_%3A%29)).
3. Use `DADiskCopyIOMedia` and walk IOKit's service-plane parents by class/subsystem instead of assuming a fixed depth. Read the USB host device’s standardized serial/container/vendor/product/location properties when present. Apple exposes a USB serial-number property and a separate location ID; location is topology, not medium identity ([USB serial property](https://developer.apple.com/documentation/iousbhost/iousbhostdevicepropertykey/serialnumberstring), [USB location ID](https://developer.apple.com/documentation/iousbhost/iousbhostpropertykeylocationid), [I/O Registry APIs](https://developer.apple.com/documentation/iokit/1514293-ioregistryentrycreatecfproperty)).
4. Treat BSD names and `IORegistryEntryGetRegistryEntryID` values as session correlation. Treat volume/media UUIDs as mutable filesystem/partition evidence. Only accept a parent serial as the medium serial after a hardware fixture proves it changes with cards rather than remaining with the reader.
5. Decide direct-download/notarized versus Mac App Store distribution before certification. Apple states sandboxed apps can observe disk appearance/disappearance but their useful access is limited without appropriate user-selected/security-scoped access, and full disk access cannot be granted automatically ([Disk Arbitration sandbox note](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/Introduction/Introduction.html), [App Sandbox file access](https://developer.apple.com/documentation/security/accessing-files-from-the-macos-app-sandbox)).

### Linux

Use the system device abstraction (systemd `sd-device` or the supported libudev interface selected during implementation) plus libmount/libblkid-style APIs. Do not parse `lsblk` human output, shell out to `udevadm monitor`, or hard-code `/sys` parent depths in the primary runtime.

1. Create and enable a udev/device monitor filtered to the block subsystem before enumerating current devices; reconcile the initial snapshot and deduplicate any event seen by both paths. Capture `add`, `change`, `remove`, and media-change behavior.
2. Enumerate disks and partitions separately and walk parents by subsystem/class. Kernel documentation says block devices and partitions are a flat class list, parent position can change, and applications should use abstractions such as udev instead of relying on sysfs implementation layout ([Linux sysfs access rules](https://www.kernel.org/doc/html/latest/admin-guide/sysfs-rules.html)).
3. Capture udev/blkid candidates such as WWN, hardware serial/short serial, bus, model, vendor, filesystem UUID, partition UUID, and persistent by-id/by-path candidates only when present. `ID_PATH`/devpath is topology/session evidence. The systemd `sd-device` API is the supported device enumeration/introspection surface ([systemd `sd-device`](https://www.freedesktop.org/software/systemd/man/latest/sd-device.html)).
4. Resolve mount points through libmount or `/proc/self/mountinfo`-backed library APIs and filesystem details through libblkid/udev. Never parse default `lsblk` columns; upstream documents that default output can change and that newly added devices may precede completed udev enrichment ([upstream `lsblk(8)`](https://github.com/util-linux/util-linux/blob/master/misc-utils/lsblk.8.adoc)).
5. Treat `/dev/sdX`, kernel names, and devpath as session identifiers. Treat `/dev/disk/by-id` as hardware-reported evidence with provenance, not a universal contract; upstream util-linux explicitly says its ID is not strictly defined and depends on udev rules and hardware ([upstream `mount(8)`](https://github.com/util-linux/util-linux/blob/master/sys-utils/mount.8.adoc#indicating-the-device-and-filesystem)).
6. Run discovery as the desktop user. If direct block-device reads for enrichment fail, return the udev database/mount information with reduced confidence. Packaging-specific confinement (Flatpak/Snap) is a separate support variant and requires real confinement tests.

## Reconciliation and lifecycle rules

- Start event monitoring before the initial snapshot on all platforms, then merge snapshot/events by session key and node identity.
- Debounce a short burst only to allow partitions/filesystems/mounts to settle; never discard events solely by time. Use a monotonic generation/revision and perform a fresh authoritative snapshot after every burst.
- Emit immutable snapshots/deltas from Rust over an ordered Tauri channel. Each update carries a revision so the UI can reject stale/out-of-order state.
- Model `present but empty`, `present but unmounted`, `mounted`, `busy`, `query-remove`, and `removed` separately.
- A removal increments the generation, cancels work that still references the old generation, closes native handles, and prevents stale format/copy authorization from being reused after a different card appears in the same slot.
- Reconcile after resume/wake, permission changes, missed-event/channel overflow, and app window recreation. A bounded periodic reconciliation is a safety net, not the primary detector.
- Detect identity collisions among concurrently attached devices. Collision means `ambiguous`, not “same card,” until stronger evidence or user confirmation resolves it.

## Device details shown to users

The details view must distinguish facts from confidence:

- friendly name, vendor, model/product, revision, bus/transport;
- reader/enclosure and slot (when resolvable) separately from inserted media;
- capacity, used/free/available, block size, partition count, filesystem, label, mount point(s);
- removable/ejectable, writable/read-only, mounted/unmounted, supported/unsupported filesystem, busy/access error;
- serial/WWN/GUID candidates with source, scope, and confidence; permit copy-to-clipboard but do not imply immutability;
- application medium key, reader key, slot key, and volume key, plus a clear warning when automatic per-card recall is unsafe;
- current-session OS identifiers under an advanced diagnostics section.

## Implementation work items

1. Define the common device graph, identity-candidate, confidence, access-state, snapshot, delta, and error DTOs plus serialization/TypeScript contract tests.
2. Define a `DeviceDiscovery`/`IdentityResolver` abstraction and a deterministic fake backend.
3. Implement Windows native enumeration, notifications, parent graph, storage descriptor enrichment, and volume mapping.
4. Implement macOS Disk Arbitration lifecycle plus IOKit/IOUSBHost parent enrichment.
5. Implement Linux device monitor/enumeration plus udev/blkid/mount enrichment.
6. Implement canonical normalization, versioned key derivation, collision handling, reader/medium separation, and no-auto-match policy for weak identities.
7. Stream reconciled snapshots to the frontend with revision/order/cancellation handling.
8. Add a diagnostics export that redacts mount paths by default but records OS, adapter result, identity provenance/confidence, event sequence, and error codes.
9. Run the fixture/evidence matrix below and record actual hardware/OS versions. Do not promote simulated evidence to native certification.

## Acceptance criteria

- [ ] Startup enumeration and hotplug work through native APIs on Windows, macOS, and Linux without polling drive letters or parsing CLI display text.
- [ ] Event monitoring begins before initial enumeration, duplicate arrival is harmless, and reconciliation produces a single coherent device graph.
- [ ] Reader, slot, medium, partition, filesystem/volume, mount point, and current session are distinct concepts in the contract.
- [ ] Windows, macOS, and Linux adapters return the common required details or a typed `unavailable` reason; absence of a serial/UUID never crashes discovery.
- [ ] No drive letter, BSD name, Linux devnode/devpath, USB port path, volume label, or IOKit registry-entry ID is persisted as an immutable medium ID.
- [ ] Every derived key includes source provenance and confidence; weak/colliding identity disables silent destination recall and destructive authorization.
- [ ] Two identical-model storage devices can be distinguished when the hardware reports distinct stable identifiers; when they cannot, the UI reports ambiguity and asks for confirmation.
- [ ] A multi-slot reader can expose two simultaneous media nodes under one reader and keeps slot/topology identity separate from medium identity.
- [ ] Insert, mount-late, unmount, eject, rapid swap, surprise removal, empty-reader, sleep/resume, and duplicate-event scenarios have deterministic tests.
- [ ] Discovery remains useful as an ordinary desktop user. Missing privileges reduce detail/confidence with a typed error rather than forcing app-wide elevation.
- [ ] Stale device generation invalidates queued copy/format actions before native I/O begins.
- [ ] Native hardware evidence is recorded separately for each supported OS; CI compile/unit evidence alone does not satisfy runtime acceptance.

## Test and evidence plan

### Automated tests

- Identity normalization/canonical encoding golden tests, including whitespace/NUL-padded serials, case rules, missing fields, non-ASCII labels, and schema-version migration.
- Confidence precedence tests: strong hardware ID wins; reader serial never becomes medium serial without validated provenance; filesystem/topology/session fallbacks cannot silently upgrade confidence.
- Collision tests with identical VID/PID/model/serial, cloned filesystem UUIDs, duplicate labels, and multiple partitions.
- Reconciliation sequence tests: event-before-snapshot, snapshot-before-event, duplicate event, change burst, removal during enrichment, remove/add reuse of same OS name, overflow/full-rescan, suspend/resume.
- Contract round-trip tests between Rust serialization and TypeScript discriminated unions.
- Platform adapter tests around captured, synthetic, non-personal fixture records; unsafe FFI boundaries get focused success/error/cleanup tests.
- UI state tests for empty, loading, ready, unmounted, permission-denied, unsupported, ambiguous, removed, and stale-revision states.

### Native hardware matrix

Run on at least Windows, macOS Intel or Apple silicon, and a reference systemd-based Linux distribution with:

- USB flash drive with unique serial;
- USB flash drive or card reader with no usable serial;
- two identical-model devices simultaneously;
- single-slot SD reader, single-slot microSD reader, and target dual-slot SanDisk Professional reader;
- one card with multiple partitions;
- duplicate/cloned filesystem UUID fixture;
- read-only/write-protected card;
- unsupported or unmounted filesystem;
- empty reader that remains enumerated;
- USB hub connection and port move;
- rapid card swap and unplug during detail enrichment.

For each run, preserve a redacted record of event ordering, graph, identity sources/confidence, collision decision, details, permissions, and cleanup. Repeat the same card through reboot, replug, port move, reader move, label change, and (on disposable media only) format. Expected result: hardware identity follows the card only where actually exposed; filesystem identity changes on format; topology follows the slot/port; session names may change.

### Evidence gates

| Gate           | Evidence required                                                                                                    |
| -------------- | -------------------------------------------------------------------------------------------------------------------- |
| Contract       | Rust/TypeScript contract and identity golden tests pass                                                              |
| Windows native | Real insert/remove and two-identical-device trace; volume-to-disk mapping; ordinary-user and access-denied fallback  |
| macOS native   | Disk Arbitration appeared/change/disappeared trace; IOKit parent evidence; distribution/sandbox mode documented      |
| Linux native   | udev add/change/remove trace; parent traversal and mount enrichment; ordinary-user and confinement status documented |
| Lifecycle      | Surprise-removal/rapid-swap test proves stale generation cannot authorize work                                       |
| Identity       | Replug/port/reader/format matrix proves which values follow reader, slot, medium, filesystem, and session            |
| UX             | Rendered details state shows confidence/ambiguity rather than claiming an unavailable immutable ID                   |

## Permissions and packaging blockers

- Windows direct disk handles may be restricted; the implementation must prove the unprivileged discovery path and record which optional fields need elevation on tested versions.
- macOS sandboxed distribution can observe disks but may not access their contents without user-selected/security-scoped authorization; choose and test the actual distribution model.
- Linux udev availability, rules, desktop auto-mount, and Snap/Flatpak confinement vary. Define the supported reference environment and certify confined packages separately.
- Real device/slot identity cannot be certified without the physical fixtures. Missing hardware is a blocker for the relevant matrix cell, not permission to infer behavior.

## Out of scope

- Copy scheduling, hashing implementation, media sorting, and formatting implementation; this task supplies their trusted device graph and generation-bound identity.
- Inventing a firmware-independent card serial when the reader/OS does not expose one.
- Treating camera metadata as physical-card identity.
- Automatic mounting or filesystem repair.

## Follow-up work

- Per-medium destination settings must store `medium_key`, its confidence/provenance, and require reconfirmation after ambiguity or identity migration.
- The SanDisk reader-slot task must add verified VID/PID/interface/topology mappings on top of this graph and keep product-specific mappings as data with captured evidence.
- Format authorization must bind to `medium_key` **and** current session generation, then re-resolve immediately before the destructive call.
- Camera grouping must distinguish camera body identity from model and from storage-medium identity.

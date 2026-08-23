# TASK006 - Map SanDisk Professional PRO-READER SD and microSD slots

## Status and ownership

- **Status:** in progress; exact reader-fingerprint plus logical-unit controlled-insertion calibration is implemented and locally tested. Windows now recognizes the exact `SanDisk` / `PRO-READER` storage-descriptor family as a presentation/calibration hint, while retaining the derived fingerprint plus LUN as the calibration key. A live controlled SD insertion establishes this connected reader's logical unit 0 as its SD slot; microSD, second-reader, and cross-platform evidence remain pending.
- **Owner:** unassigned
- **Depends on:** TASK002 device discovery/identity
- **Supports:** TASK005 safe format and every ingest/detail view that displays or persists a physical slot
- **Risk:** high; a wrong slot label can cause the operator to ingest from or format the wrong card

## Objective

Recognize the SanDisk Professional PRO-READER SD and microSD device and reliably label each exposed storage logical unit as the physical **SD** slot or **microSD** slot across Windows, macOS, and Linux. Mapping must be based on the reader's numeric hardware identity and validated USB/SCSI topology, never the arrival order, drive letter, `/dev/sdX`, BSD disk number, volume label, card model, or UI list position.

## Supported product boundary

The initial product is the **SanDisk Professional PRO-READER SD and microSD**, retail model `SDPR5A8-0000-GBAND` (regional suffixes may differ). SanDisk documents a USB-C 5 Gbps connection, SD and microSD support, Windows 10+ and macOS 10.9+ compatibility ([official product page](https://www.sandisk.com/products/accessories/memory-card-readers/sandisk-professional-pro-reader-sd-microsd), [official data sheet](https://documents.sandisk.com/content/dam/doc-library/en_us/assets/public/sandisk-pro/product/accessories/pro-reader-sd-microsd/data-sheet-pro-reader-sd-microsd.pdf)). The current conformity declaration identifies regulatory model `G6B` and the `SDPR5A8` SKUs ([SanDisk declaration](https://documents.sandisk.com/content/dam/asset-library/en_us/assets/public/sandisk/collateral/product-compliance/SanDisk-CE-Declaration-of-Conformity-Memory-Card-Pro-Readers.pdf)).

Do not match the PRO-READER Multi-Card (`SDPR3A8`) or other SanDisk readers under this rule. They require distinct topology captures and mapping records even if descriptor strings are similar.

The implementation recognizes only the exact normalized descriptor pair `SanDisk` + `PRO-READER` as `sandisk_pro_reader`. That is a product-family hint to make the operator-facing reader recognizable; it is not an individual-reader identity, a card identity, or a format authorization. Any different descriptor (including `PRO-READER MULTI-CARD`) remains unrecognized until separately captured and approved.

SanDisk does not publish USB VID/PID, SCSI LUN-to-front-slot mapping, interface numbers, or a guarantee that those values remain identical across revisions. The retail SKU is not a USB product ID. Exact allowlist values must therefore come from controlled hardware captures and remain revision-scoped.

## Initial local hardware observation

A read-only Windows probe on 2026-08-23 found one connected, empty PRO-READER with:

- USB hardware ID `VID_0781&PID_D003`, device revision `0056`;
- USB mass-storage parent bus description `PRO-READER`;
- two child disk logical units with PnP instance suffixes `&0` and `&1`; `Win32_DiskDrive` independently reported SCSI LUN 0 and LUN 1 for those children;
- both child disks present while empty and reporting `No Media`/zero capacity;
- both children sharing the same USB parent/container and reported serial string `000000000056`.

The observation used read-only `Get-PnpDevice`, `Get-PnpDeviceProperty`, `Get-Disk`, and `Get-CimInstance Win32_DiskDrive` queries. It is a **discovery seed**, not a certified global mapping. No card was inserted, so it does not establish whether LUN 0 or LUN 1 is SD or microSD. The serial resembles the revision and must be treated as potentially non-unique until two physical readers are compared. Do not ship a reader-unit identity or physical slot label derived solely from this observation.

## Controlled SD insertion evidence

On 2026-08-23, after capturing the empty-reader baseline, an operator inserted a sacrificial full-size SD card into the physical SD slot of the same PRO-READER. Windows then reported logical unit 0 as an online, healthy 255.9 GB removable exFAT disk mounted at `D:`; logical unit 1 remained no-media/zero capacity. The running native application independently showed the populated unit as logical unit 0 and the empty companion as logical unit 1. This establishes a calibration record of **reader fingerprint + LUN 0 -> SD** for this reader only. It does not establish LUN 1 -> microSD until a separate controlled microSD insertion, and it does not promote reader topology to card identity or format authorization.

## Identity model

Keep these identifiers separate:

- `reader_model_key`: allowlisted numeric USB VID/PID plus descriptor/firmware constraints for a supported hardware revision.
- `reader_instance_key`: strongest available reader-instance identifier, with confidence. Prefer a demonstrably unique USB serial; if absent/duplicated, use an installation-local identity plus current topology and disclose that reconnecting to another port may require re-correlation.
- `slot_key`: reader model/revision plus an OS-neutral logical-unit fingerprint, expected to reduce to USB interface + SCSI target/LUN after certification.
- `physical_slot_label`: `sd`, `microsd`, or `unknown`, obtained only from a certified mapping record or guided calibration.
- `media_key`: TASK002's inserted-card identity. It must not be substituted for reader or slot identity.
- `session_device_key`: Windows disk number, macOS BSD name, Linux major/minor or device node. It is valid only for the current attachment/session and never persists as the slot key.

There is no honest universal "immutable" identifier when a USB bridge omits or duplicates serials or hides the SD card CID. The implementation must expose identity confidence and fail closed for destructive work. A stable, certified LUN-to-physical-slot relation can identify the slot even though session device names change.

## Platform discovery plan

### Windows

1. Enumerate disk device interfaces with SetupAPI/Configuration Manager and resolve each to its USB mass-storage parent. Read numeric hardware IDs, device instance ID, parent, container ID, bus-reported description, and `DEVPKEY_Device_LocationPaths`; Windows documents location paths as an OS-set, read-only representation of a device's place in the device tree ([DEVPKEY_Device_LocationPaths](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-locationpaths)).
2. Walk parent/child relations through Configuration Manager rather than parsing localized friendly names ([retrieving device relations](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/retrieving-device-relations)).
3. Query storage descriptors through `IOCTL_STORAGE_QUERY_PROPERTY` for vendor/product/revision/serial and identifiers ([IOCTL_STORAGE_QUERY_PROPERTY](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_storage_query_property)).
4. Query target/LUN from the actual disk device with `IOCTL_SCSI_GET_ADDRESS`, which returns target and logical-unit address information ([IOCTL_SCSI_GET_ADDRESS](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntddscsi/ni-ntddscsi-ioctl_scsi_get_address)). Do not assume the textual PnP `&0`/`&1` suffix is the LUN until fixtures prove the relation.
5. Use `IOCTL_STORAGE_GET_DEVICE_NUMBER` only to join the disk interface to a current volume/partition. Microsoft guarantees that number only until removal or restart, so it cannot identify a physical slot ([IOCTL_STORAGE_GET_DEVICE_NUMBER](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_storage_get_device_number)).

### macOS

1. Subscribe through Disk Arbitration for whole-media appearance, disappearance, and description-change events. Obtain the whole `DADisk`, description dictionary, and backing `IOMedia`; Apple warns that description keys vary even across devices of one type, so missing properties are normal test cases ([Disk Arbitration guide](https://developer.apple.com/library/archive/documentation/DriversKernelHardware/Conceptual/DiskArbitrationProgGuide/ManipulatingDisks/ManipulatingDisks.html)).
2. Traverse the `IOService` parents from `IOMedia` to the USB mass-storage device. Read numeric USB vendor/product IDs, release number, serial, interface number, registry location, and SCSI logical-unit properties where exposed. Apple's USB property constants include vendor ID, product ID, interface number, product string and serial string ([USB descriptor constants](https://developer.apple.com/documentation/iokit/usbspec_h_user-space/usb_descriptor_and_ioregistry_constants)).
3. Capture the IOService-plane registry path as topology evidence using `IORegistryEntryGetPath`, not as a reader identifier guaranteed across ports ([IORegistryEntryGetPath](https://developer.apple.com/documentation/iokit/1514229-ioregistryentrygetpath)).
4. Do not parse `diskN` order. Correlate the observed logical-unit property/path component with the certified `slot_key`; return `unknown` when the property is absent or a revision is unrecognized.

### Linux

1. Subscribe to UDisks2 D-Bus objects/events and link each `Block` to its `Drive`. Use the drive vendor/model/revision/serial/ID plus `SiblingId`; UDisks defines `SiblingId` as an opaque token grouping drives in one physical device, specifically relevant to multi-slot readers ([UDisks Drive API](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Drive.html)).
2. Resolve the underlying udev/sysfs device and walk to the USB device and SCSI-device parent through libudev/systemd device APIs. Capture numeric `idVendor`, `idProduct`, `bcdDevice`, USB serial/interface and SCSI host:bus:target:LUN.
3. Treat `/dev/disk/by-path`/`ID_PATH` as topology corroboration, not card identity. systemd's persistent-storage rules generate by-path links from the physical device path and by-id links from USB/SCSI identifiers; they also provide `by-diskseq` specifically for race-free current-device validation ([systemd persistent-storage rules](https://github.com/systemd/systemd/blob/main/rules.d/60-persistent-storage.rules.in)).
4. Do not claim manufacturer-supported Linux compatibility: SanDisk's product page lists Windows and macOS only. Linux support is this application's own hardware-certification claim.

## Mapping registry and fallback

Store mapping data as reviewed, versioned backend resources, not conditionals in UI code. Each record contains:

- numeric VID/PID, allowed device-release/firmware range, required descriptors and optional regulatory/retail model metadata;
- logical-unit fingerprint and physical label (`sd` or `microsd`);
- platforms/OS versions on which that mapping was observed;
- direct USB versus PRO-DOCK topology variants;
- evidence fixture IDs, physical reader unit count, capture date, and confidence (`single-unit`, `multi-unit`, `certified`);
- any known duplicate/blank serial behavior.

Matching rules are exact and fail closed. A new PID, release, changed LUN topology, missing required descriptor, duplicate child key, or more/fewer than the certified logical units yields `unknown`; it must never fall through to a similar product string.

For unknown revisions, offer a guided, non-destructive calibration:

1. Require both slots empty and confirm two no-media endpoints if the platform exposes them.
2. Ask the operator to insert one expendable microSD card into the microSD slot, observe exactly one endpoint transition, then remove it.
3. Insert that same microSD card through a passive full-size SD adapter into the SD slot, observe the other endpoint, then remove it.
4. Repeat once after reader reconnect. Persist an installation-local mapping only if both passes agree; otherwise retain `unknown` and collect a diagnostic fixture with consent.

Calibration may improve display labels but must not authorize formatting on its own. TASK005 still revalidates reader, slot and media identity at execution time.

## Implementation work packages

1. Define reader/slot identity types, confidence levels, mapping-record schema, and sanitized fixture schema in the TASK002 domain.
2. Add the exact SanDisk model recognizer with no physical labels until hardware correlation succeeds.
3. Implement Windows topology/LUN extraction and golden fixtures from supported hardware.
4. Implement macOS Disk Arbitration/I/O Registry extraction and equivalent fixtures.
5. Implement Linux UDisks/libudev/sysfs extraction and equivalent fixtures.
6. Run controlled slot-correlation experiments, review the resulting mapping registry, and enable `sd`/`microsd` labels only for certified records.
7. Add guided calibration and `unknown` UX for unsupported revisions.
8. Feed the semantic slot key and confidence into ingest details, receipts, per-device destination settings, and TASK005 confirmation without exposing raw device paths as identity.

## Acceptance criteria

- `SDPR5A8` is recognized by numeric, revision-scoped hardware evidence; display strings and retail SKU alone never trigger special handling.
- On each supported OS, two reader slots retain distinct semantic slot keys across card insertion/removal, arrival-order reversal, application restart, host reboot, reader reconnect, drive-letter/device-node/BSD-name changes, and simultaneous occupancy.
- A controlled same-card experiment proves which logical unit is the SD slot and which is the microSD slot. The mapping is repeated after reconnect and on at least two physical reader units before confidence is `certified`.
- Direct USB and every claimed PRO-DOCK connection mode are tested separately. Moving a PRO-READER among dock bays may change topology but not its physical SD/microSD labels.
- Two identical PRO-READERS attached simultaneously remain distinguishable. If their serials are absent or duplicated, the UI exposes the reduced identity confidence and per-reader destination persistence does not silently cross-apply.
- Empty slots, one occupied slot, both occupied slots, rapid swap, removal during discovery, write-protected SD, passive microSD-to-SD adapter, multiple partitions, RAW media, and duplicate volume labels all produce the correct slot result.
- Unknown PID/revision/topology returns `unknown` or requires guided calibration; it never guesses from disk ordinal, LUN ordinal without a mapping record, size, filesystem, label, or card model.
- The slot mapping remains correct when the inserted card lacks a serial/CID or when both cards have the same model/capacity/label.
- TASK005 receives an opaque semantic slot key plus confidence and rejects destructive work when the mapping/identity changes after confirmation.
- No fixture contains user file names, full mount paths, unrelated USB inventory, or raw personal identifiers; reader serials are hashed/redacted unless an isolated test serial is explicitly approved.

## Hardware certification protocol

### Required inventory

- At least two physical `SDPR5A8` readers; include each available device revision/firmware and record regulatory model.
- PRO-DOCK 4 if dock support will be claimed, with every bay exercised.
- One microSD test card and passive SD adapter for same-media slot correlation; a second SD card for simultaneous-slot tests.
- Windows 10 and 11 hosts, supported macOS Intel/Apple-silicon hosts, and representative Linux x86_64/ARM64 distributions with real USB pass-through.

### Capture set per OS/topology

Capture, sanitize, and turn into a golden semantic fixture:

1. reader absent;
2. reader present with both slots empty;
3. microSD slot only occupied;
4. both empty again;
5. same microSD through SD adapter in SD slot only;
6. both slots occupied with distinct test cards;
7. remove/reinsert each card in reverse order;
8. disconnect/reconnect reader, change USB port, reboot, sleep/wake;
9. repeat with two identical readers and, if supported, in every PRO-DOCK bay.

For each transition record monotonic event order, USB descriptors, reader parent/container or sibling group, OS logical-unit address, current session device identifier, media identity/confidence, and resulting semantic slot key. Photograph the physical placement with the fixture ID visible; do not include production media or user content.

Certification requires the mapping to be identical across two cold-start repetitions per OS/topology. A single Windows capture may seed the implementation but cannot certify macOS, Linux, multiple hardware units, or PRO-DOCK behavior.

## Known blockers

- The available reader currently had both logical units empty. The initial probe established USB `0781:D003` revision `0056` and two logical units, but **did not establish which LUN is SD versus microSD**.
- A second physical reader is required to determine whether serial `000000000056` is unique and whether the LUN mapping is consistent across units.
- macOS and Linux hosts plus real USB access are required; VM-only or Windows-only evidence cannot certify those providers.
- PRO-DOCK 4 hardware is required before dock compatibility or bay-stable reader identity can be claimed.
- SanDisk publishes no LUN mapping or compatibility promise for Linux. Any such claims must come from this project's recorded hardware evidence and be scoped to tested versions.

## Completion evidence

Not yet available. Add reviewed mapping records, sanitized fixtures, physical-placement photographs, test commands/results, OS/hardware inventory, and certification status after the controlled protocol passes.

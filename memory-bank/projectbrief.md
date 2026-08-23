# Project Brief

## Purpose

Create a fast, reliable desktop media-ingest tool for copying camera media from removable cards to operator-selected storage while preserving device and camera identity, organizing output, independently verifying every file, and safely preparing verified cards for reuse.

## Users and stakeholders

- Primary user: a media operator or photographer ingesting one or more camera cards on a workstation.
- Stakeholders: camera operators whose media must stay attributable to the correct camera body and the person responsible for destination storage and card reuse.

## Scope

- Tauri 2 desktop application with a Rust core and React + TypeScript + Tailwind CSS UI.
- Windows, macOS, and Linux support; exact versions and architectures must be certified before release.
- Hotplug discovery and rich details for removable storage and its reader/topology.
- Stable identity with explicit confidence and immutable hardware evidence where the host exposes it.
- A remembered destination and organization profile per exactly identified source device.
- Bounded concurrent copy, camera/time sorting, resume/recovery, full-file verification, receipts, safe eject, and guarded quick format.
- Special slot identification for the SanDisk Professional PRO-READER SD/microSD family after exact hardware validation.

## Success criteria

- No file is reported transferred until a full source digest matches a fresh full read of the destination file.
- Multiple cards can ingest concurrently to different destinations without identity, progress, or receipt crossover.
- Multiple cameras of the same model remain separate using body serial or an explicitly unresolved/operator-assigned identity.
- Replugging a different card into the same reader, slot, drive letter, or mount path cannot inherit another card's trusted destination or format authorization.
- Formatting cannot target an internal/system disk and is unavailable until the current card's ingest is completely verified.
- Performance and correctness are measured on every declared OS and named supported reader rather than inferred from compilation.

## Constraints

- Drive letters, mount paths, labels, and volume UUIDs are mutable and must not be presented as immutable card identifiers.
- SD CID is a per-card hardware identifier, but a USB reader may not expose it to the host; the product must disclose lower-confidence fallbacks.
- Camera serial and accurate capture timezone metadata are not guaranteed to exist in every media format.
- Formatting is destructive and normally needs elevated OS authorization. “Quick format” removes filesystem structures but is not secure erasure.
- Computer formatting alone cannot guarantee a card has every camera-vendor-specific directory/customization; certified camera profiles or in-camera formatting may still be required.
- Hardware certification needs physical cards/readers and sacrificial media that are not currently evidenced in the repository.

See [Product context](productContext.md) and [Technical context](techContext.md).

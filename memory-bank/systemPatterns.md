# System Patterns

## Architecture

- Tauri 2 is the desktop shell. React + TypeScript + Vite + Tailwind CSS render the operator UI.
- Rust owns all trusted operations: device discovery, identity normalization, local persistence, source inventory, path planning, copy scheduling, hashing, verification, eject, and formatting authorization.
- Frontend-to-core commands are narrow typed requests. Rust emits throttled typed state/progress events; the UI never receives authority to access arbitrary files or execute shell commands.
- OS adapters implement one common domain contract for Windows storage APIs, macOS Disk Arbitration/IOKit, and Linux udev/sysfs/UDisks2.

## Data and ownership

- SQLite in the local application-data directory is authoritative for profiles, sessions, file state, identity observations, and receipts.
- The mounted source filesystem is authoritative for current source contents; a preflight inventory is a snapshot that must be revalidated during ingest.
- Each identifier remains namespaced and retains provenance/confidence. Matching is exact within a namespace; a composite identity records its parts rather than hashing away evidence.
- Destination profiles belong to accepted source identities. Camera profiles belong to camera-body evidence and are not silently equated with a card or reader.
- Completion receipts are append-only evidence projections from the local store, not a second editable record.

## Security and privacy

- Treat removable media, filenames, metadata, and filesystem structures as hostile input.
- Do not follow links/reparse points or permit normalized paths to escape source/destination roots.
- Use least-privilege Tauri capabilities and a separately authorized narrow formatting path.
- Re-identify a card immediately before any destructive action and reject internal, system, boot, recovery, destination, or ambiguous disks.
- Keep all media and operational data local by default; redact identifiers and host paths from exported diagnostics.

## Change patterns

- Platform adapters return raw observations plus normalized projections; do not flatten unavailable fields into guessed values.
- Long-running work is represented as durable state machines with cooperative cancellation and connection-generation checks.
- Copy to a same-directory temporary name, flush as required, fully re-read/hash destination, and only then publish/mark verified.
- Concurrency is bounded per source, destination, and shared topology group and calibrated with benchmarks.
- Certification status distinguishes fixtures, local native hardware, packaged artifacts, and platform/device-specific proof.

Related: [Technical context](techContext.md), [Decisions](decisions.md), and [Certification matrix](certification-matrix.md).

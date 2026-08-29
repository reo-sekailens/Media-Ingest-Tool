# TASK003 — Cross-platform ingest, camera identity, copy scheduling, and sorting

## Task metadata

- **Status:** In progress — portable regular-file enumeration, a planned-byte destination free-space preflight using the nearest existing destination ancestor, bounded 1–16 worker verified copy, a local native-ingest/receipt path, and initial pure-Rust metadata-backed original/camera-day/hour/minute destination planning are locally tested. A native read-only preview safely returns current-generation plan totals and sample paths; its UUID is retained for ingest so unknown-camera fallback paths remain deterministic. Root-level Windows `System Volume Information` is explicitly excluded as unreadable host metadata; ordinary inaccessible entries still fail the inventory rather than being silently skipped. Only after a successful SQLite receipt seal and completed lifecycle state does the source root receive a compact, create-new-only app marker when writable; existing valid markers are recognized and the marker is excluded from camera-media copying. This marker is mutable filesystem evidence, not a stable-card identity. Metadata fixture coverage/timezone policy, device-aware permits/progress/cancellation, cross-platform fixture evidence, native preview evidence, and performance characterization remain pending.
- **Portable destination projection:** Planner components are normalized for Windows-compatible names and byte limits; the copy primitive independently rejects unportable destinations.
- **Portable collision keys:** Planner collision checks use NFC normalization and Unicode default case folding before deterministic suffixing.
- **Destination scheduling:** Native operations lease a resolved destination root so independent destinations run concurrently while same-root operations serialize conservatively.
- **UI responsiveness:** All current ingest scan/planning, copy/verification,
  and preflight filesystem work runs on Tauri blocking workers. Per-operation
  progress IPC is coalesced to one event per 100 ms while exact native byte
  counters and terminal lifecycle events remain authoritative.
- **Timestamp folders:** Camera-day and interval destination levels use
  portable ISO 8601 basic timestamps (`YYYYMMDDTHHMMSS+HHMM`) built from the
  aware EXIF capture offset. A naive EXIF timestamp preserves its recorded
  camera wall clock for grouping and uses an `-offset-unknown` label until an
  explicit camera timezone policy exists.
- **Priority:** P0
- **Owner:** Unassigned
- **Depends on:** TASK001 (Tauri/Rust/React/TypeScript/Tailwind foundation), TASK002 (storage discovery and stable device/volume identity), TASK007 (local state and destination/camera profiles)
- **Blocks:** TASK004 (verification and recovery), TASK005 (safe card formatting), ingest UI work
- **Target platforms:** Windows, macOS, and Linux desktop
- **Last researched:** 2026-08-23

## Objective

Build the Rust ingest engine that enumerates every regular file on a selected removable volume, resolves the destination configured for that stable device identity, extracts camera and capture-time evidence, creates deterministic destination paths, and transfers files with bounded parallelism and progress/cancellation support.

The engine must be fast on one card, scale across several cards, and never confuse two cameras merely because they share a make/model. It must not overwrite an unrelated destination file, follow links outside the selected volume, or claim an ingest is complete before TASK004 verifies it.

## Scope boundary

This task includes the Rust domain model, planning/enumeration, metadata extraction, path generation, the bounded copy pipeline, cancellation, and performance characterization. It exposes typed commands/events for the Tauri shell, but does not implement the React UI.

This task does not format/eject a card, delete source files, silently deduplicate files, or define the final durability/format authorization gate. Those belong to TASK004/TASK005.

## Research conclusions and decisions

### Copy strategy

- Use blocking filesystem workers rather than pretending normal files are asynchronous. Tokio documents that ordinary filesystem operations are blocking on most operating systems and recommends batching work into as few `spawn_blocking` calls as practical: [Tokio filesystem guidance](https://docs.rs/tokio/latest/tokio/fs/).
- Implement a portable verified-stream path first: an open source handle is read through reusable large buffers, the source hash is updated as bytes pass, and a uniquely created temporary destination file is written. This gives exact progress, cooperative cancellation, bounded memory, and only one source read during an uninterrupted ingest.
- Keep a pluggable native-copy path for measured cases. Rust's current `std::fs::copy` uses kernel/native facilities where available: `copy_file_range`/`sendfile`/`splice` on Linux, `CopyFileEx` on Windows, and `fclonefileat`/`fcopyfile` on macOS: [Rust `std::fs::copy`](https://doc.rust-lang.org/std/fs/fn.copy.html). Because native copy does not expose the byte stream to the hasher, it requires a separate source hash pass; it must only be selected when benchmarks show a net benefit and it must still feed TASK004's independent destination verification.
- Parallelize across files and independent devices, not blindly across chunks of one slow card. Each source device and destination volume receives its own I/O permits. Two cards may ingest concurrently; two writers targeting one rotational disk must respect that destination's lower concurrency.
- Use bounded queues and a reusable buffer pool. Enumeration/metadata, copy, and verification are separate stages. A slow destination or verifier must backpressure upstream work instead of allowing unbounded file plans, tasks, or buffers.
- Never use one unbounded `spawn_blocking` task per file. A small fixed worker set owns batched blocking loops; cancellation is checked between chunks. Tokio warns that started blocking work cannot simply be aborted: [Tokio blocking-task guidance](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html).

### Camera identity is evidence, not a guess

- Store camera make/model for display and a separate identity key for grouping. The strong automatic key is a versioned hash of the manufacturer namespace plus the normalized body serial, for example `BLAKE3("camera-id-v1\\0" || make || "\\0" || body_serial)`. Do not expose the full serial in a folder name; use the display label plus a short non-secret key suffix.
- Read the standard Exif `Make`, `Model`, `BodySerialNumber`, `DateTimeOriginal`, `SubSecTimeOriginal`, and `OffsetTimeOriginal` fields. CIPA's current standards list identifies Exif 3.1 and its XMP mapping as the current specifications: [CIPA digital-camera standards](https://www.cipa.jp/e/std/std-sec.html). Body serial and timestamps are metadata values, not cryptographically immutable facts; their source, raw value, parser, and confidence must remain auditable in the manifest.
- Maker-note serial fields may supplement the standard field only through an explicitly tested manufacturer adapter. They must not be normalized into the same namespace without recording the manufacturer and source tag. ExifTool's maintained tables demonstrate that QuickTime and maker-note metadata are format/vendor-specific: [ExifTool tag tables](https://exiftool.org/TagNames/) and [QuickTime tags](https://exiftool.org/TagNames/QuickTime.html).
- A camera model alone is never an identity. If serial evidence is absent, create a run-scoped `unknown-camera` identity so files from two same-model bodies are not merged. A user may map TASK002's stable card identity to a camera profile, but the result is `user_mapped`, not `embedded_serial`; cards can move between cameras. Conflicting embedded serials on a mapped card stop automatic propagation and require review.
- Allow careful propagation of a strong camera identity to a related media group only when all strong evidence on that group/card agrees and the policy is recorded. Never infer identity from filename numbering alone.

### Metadata reader choice

- Start with the pure-Rust `nom-exif` adapter because its current API covers image Exif plus video/audio track metadata without FFmpeg/system libraries and includes common still, HEIF, RAW, and ISO-BMFF inputs: [nom-exif crate documentation](https://docs.rs/nom-exif/latest/nom_exif/). Pin the reviewed release in the eventual lockfile.
- Treat claimed format support as unverified until the repository fixture corpus passes. Parser failures, missing fields, and unknown formats are normal per-file outcomes, not ingest failures.
- Keep the metadata interface replaceable. A fallback adapter may be added only after a dependency/license/packaging decision and hostile-file tests. Do not silently shell out to a user-installed ExifTool/ffprobe, because versions and availability would make behavior non-deterministic.
- Bound bytes, nesting, and time spent parsing metadata and fuzz the adapter boundary. Copying an unsupported file must still work; its camera/time evidence falls back according to the rules below.

### Capture time and deterministic buckets

Resolve one `CaptureTimestamp` with the source and confidence attached. Use this precedence:

1. Exif/XMP original capture time with explicit UTC offset and optional subseconds.
2. Container/QuickTime creation or original date with an explicit offset.
3. Exif original time without offset, retained as its recorded camera wall clock with an explicitly unknown offset until a saved camera/device timezone policy is available.
4. Container time without offset, interpreted only by a saved per-camera/device policy.
5. Source filesystem modification time, marked `filesystem_fallback`.

Never use destination creation time, ingest time, platform `ctime`, or the ingest host's timezone as capture time. When EXIF has no offset, retain its wall clock and mark the destination offset unknown; apply a configured camera/device timezone only after that policy exists. Persist the original wall time, parsed offset, chosen IANA zone, normalized instant when available, ambiguity flag, and the exact source tag. QuickTime timestamps need format-specific treatment; the maintained QuickTime table notes that timezone behavior varies by tag: [ExifTool QuickTime tag notes](https://exiftool.org/TagNames/QuickTime.html). Use an embedded/pinned IANA timezone database for deterministic Windows/macOS/Linux results: [IANA Time Zone Database](https://www.iana.org/time-zones).

Support these sort modes:

- `original_tree`
- `camera/day`
- `camera/custom_interval`, where interval is a positive integer plus `minute` or `hour`
- a validated custom template made only from supported tokens

Day and custom intervals are calculated in the selected capture/project timezone. Interval buckets are anchored at local midnight, with offset included in ambiguous fall-back-hour folder labels. Reject zero, negative, or impractically small intervals. Record the resolved bucket start/end in the manifest so a timezone database update cannot silently move an existing ingest.

Suggested safe default layout:

`<camera-label>__<camera-key-short>/<YYYY-MM-DD>/<original-filename>`

Custom interval example:

`<camera-label>__<camera-key-short>/<YYYY-MM-DD>/<HH-mm>_<offset>/<original-filename>`

### Filesystem and collision semantics

- Enumerate every regular file, including hidden files. Do not follow symbolic links, junctions, mount points, reparse points, sockets, devices, or other special entries. Report skipped non-regular entries in the plan. Rust explicitly warns that filesystem APIs can be subject to time-of-check/time-of-use races: [Rust filesystem TOCTOU guidance](https://doc.rust-lang.org/std/fs/index.html).
- Preserve the exact source-relative path losslessly in the Rust/backend manifest even when it is non-UTF-8 on Unix. Send a separately escaped display string to the frontend. Destination names are a deterministic, reversible-safe projection; the original name/path always remains in the manifest.
- Validate destination paths as relative components. Reject absolute paths, prefixes, `.`/`..`, separators inside tokens, Windows device names, trailing dots/spaces, control characters, and paths that exceed the tested safe limit. Follow Microsoft's filename rules on every platform to keep exported folders portable: [Windows naming rules](https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file).
- Detect collisions with a portable key (Unicode normalization plus case folding), even on currently case-sensitive volumes. Never overwrite an unrelated final file.
- If two planned files target the same portable key, preserve both by adding a deterministic suffix derived from source device identity, source-relative path, and eventually the content hash. A pre-existing exact verified file may satisfy an idempotent retry only when TASK004 links it to the same manifest entry. Content equality alone must not silently deduplicate two distinct source files; deduplication is an explicit future policy.
- Create the temporary file in the final file's directory using exclusive creation (`create_new` semantics), write and verify it, then commit it with a same-directory rename. Rust notes that rename does not work across mounts, hence the same-directory rule: [Rust `std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html).

## Required domain contracts

Define serializable Rust/TypeScript contracts without exposing raw OS handles:

- `IngestPlan`: run ID, source device key and generation, source root, destination profile snapshot, sort policy snapshot, enumeration totals, warnings, and planned files.
- `PlannedFile`: stable entry ID, lossless source-relative path reference, initial handle metadata/fingerprint, type, byte length, metadata evidence, camera identity, capture timestamp, bucket, intended destination, portable collision key, and plan state.
- `CameraIdentity`: versioned key, make/model, optional body serial stored in protected local data, evidence source, confidence (`embedded_serial`, `maker_note_serial`, `user_mapped`, `run_scoped_unknown`), and conflicts.
- `CaptureTimestamp`: original value, parsed local value, optional offset/instant, IANA zone and source, ambiguity flag, precision, evidence tag, and confidence.
- `DestinationProfile`: keyed by TASK002 device identity, selected root, sort/template policy, timezone policy, collision policy, and last validation result. If TASK002 cannot provide a stable device key, do not persist the association against a drive letter/mount point; require selection for that insertion.
- `CopyProgress`: run/file IDs, phase, bytes copied/verified, planned totals, instantaneous and smoothed throughput, ETA, queue depth, and recoverable error. Throttle UI events while retaining exact backend counters.
- `CopyOutcome`: temporary path reference, bytes written, source streaming digest/checkpoints, before/after source fingerprint, elapsed time, and close/sync result. TASK004 owns the verified/committed state.

## Pipeline and scheduling plan

1. Freeze source device key/generation, destination profile, sort/timezone policy, and source/destination volume identity into an immutable plan.
2. Reject source/destination overlap, destination inside the source tree, same-file aliases, insufficient writable capacity (with safety margin), and a disconnected/replaced source before copying.
3. Walk the source without following links. Open and inspect regular files using the narrowest safe handle-based operations available. Record all files before starting so count/bytes progress and completeness are meaningful.
4. Parse metadata through the bounded adapter and resolve camera/time evidence. Unknown or malformed metadata produces warnings and deterministic fallback; it never causes file loss.
5. Generate and validate all destination paths as a set. Resolve portable collisions before copying when possible; finalize content-hash suffixes after the source digest exists.
6. Feed plans through bounded per-source queues to a fixed blocking worker pool. Acquire both source and destination I/O permits, copy in reusable chunks, update the source BLAKE3 digest/checkpoints, publish throttled progress, and check cancellation/disconnect between chunks.
7. Re-read handle metadata after the last source byte. If size or modification evidence changed, mark the file unstable and send it to recovery; never bless it for formatting.
8. Flush/close the temporary destination and hand the outcome to TASK004. The copy worker must not rename into a user-visible final path until the verification contract permits it.
9. Release permits and buffers on success, error, cancellation, panic, card removal, and application shutdown.

Initial tuning knobs must be explicit and benchmarked rather than hard-coded folklore:

- reusable buffer size and pool byte cap
- maximum active files per source device
- maximum readers/writers/verifiers per destination volume
- threshold for large-file parallel BLAKE3 hashing
- progress event cadence
- metadata parser byte/time limits

Ship conservative defaults, expose advanced overrides only after tests, and store benchmark evidence by OS/filesystem/device class. Multi-threaded BLAKE3 is available through the official Rust crate's `rayon` feature, but only the explicit Rayon methods are parallel: [BLAKE3 Rust documentation](https://docs.rs/blake3/latest/blake3/).

## Implementation work packages

### TASK003.1 — Domain and planner

- Define contracts and typed error categories.
- Freeze settings/device generation into an immutable plan.
- Add capacity/overlap/writability checks and complete enumeration.
- Produce deterministic plan serialization for recovery and test snapshots.

### TASK003.2 — Metadata and camera identity

- Implement the adapter interface and pinned pure-Rust reader.
- Normalize make/model/serial without destroying the raw evidence.
- Implement confidence/conflict behavior and run-scoped unknown identities.
- Add capture-time precedence, explicit timezone handling, DST ambiguity reporting, and media-group rules.

### TASK003.3 — Sort paths and collisions

- Implement day/custom-interval buckets and template validation.
- Add portable component/path sanitization and deterministic shortening.
- Resolve in-plan, case-folded, Unicode, pre-existing, and late hash collisions without overwriting.

### TASK003.4 — Bounded copy engine

- Implement reusable-buffer streaming workers, source/destination permits, backpressure, cancellation, device-removal handling, and progress snapshots.
- Add the optional native-copy adapter behind a capability/benchmark decision and preserve identical verification semantics.
- Hand temporary outcomes to TASK004; no source mutation.

### TASK003.5 — Performance characterization

- Benchmark small-file-heavy, mixed, and large-video workloads on Windows/macOS/Linux using SD, microSD, SSD destination, HDD destination, and two simultaneous cards where hardware is available.
- Record throughput, CPU, memory high-water mark, queue depth, cancellation latency, and verification overlap.
- Tune by device/destination class without using make/model strings as identity.

## Acceptance criteria

### Recent implementation evidence

- 2026-08-28 — The operator UI exposes capture-time sorting as selectable
  EXIF day, EXIF hour, custom-minute, and original-tree tags, including an
  explicit destination-depth display. The planner accepts arbitrary bounded
  intervals from 1 to 1,440 minutes, anchored to local capture-day midnight.
  Rust unit tests and browser-fixture UI tests pass; native EXIF preview
  evidence remains outstanding.

- 2026-08-28 — Model-first camera directory labels retain a short stable
  identity suffix to avoid merging identical models. Up to eight validated
  operator custom fields are projected as label/value folders before the
  camera/time layout and persist in the marker-backed auto-ingest profile.
  Local unit and fixture evidence is complete; real-media native preview
  evidence remains outstanding.

- 2026-08-28 — Operators can drag the destination-depth folder tags to set
  the precise order of camera, custom field, time, and original-tree folders.
  The backend rejects duplicate, missing, or sort-incompatible segments and
  always appends the filename last. The chosen order is included in previews,
  manual/auto ingest, and marker-backed registration. Fixture UI and Rust unit
  tests pass; a real-card native preview remains outstanding.

- 2026-08-28 — Positioned auto-ingest registration immediately before manual
  ingest after all destination-organization settings, preventing setup from
  preceding the profile it records.

- 2026-08-28 — Set Up Auto-Ingest now matches Start Verified Ingest's
  full-width action size, making the adjacent choices equally clear.

- 2026-08-28 — Organization is covered through a real temporary filesystem
  copy: a reordered custom-field/capture-day/camera/interval layout copies to
  the exact planned path, verifies bytes, and seals a receipt. Legacy marker
  profiles without a saved drag order use the canonical layout; new profiles
  persist that complete validated order. Invalid interval or incomplete order
  inputs are rejected before planning.

- 2026-08-28 — Hardware-backed organization qualification passed with three
  controlled EXIF JPEGs mounted under `M:\MIT_ORGANIZATION_FIXTURE`. The
  native test confirmed Sony FX3 and +08:00 capture EXIF, the explicit custom
  folder/camera/day/30-minute ordering, verified destination bytes, and a
  sealed receipt. It writes only a temporary destination and leaves source
  fixture files and the card marker intact.

- 2026-08-28 — Custom-folder configuration now presents one input per custom
  destination depth. Its value is the sole emitted path component; older
  persisted label/value entries retain only their value in the destination
  projection. Browser fixture and Rust planner/copy tests cover this behavior.

- 2026-08-28 — Destination-depth tags now use both native HTML drag/drop and
  pointer-enter reordering while pressed, fixing browser button-drag gaps.
  The result remains a complete ordered permutation with stale previews
  discarded. Fixture tests and browser automation verified a real reorder.

- [ ] Windows, macOS, and Linux enumerate and copy every regular file in the fixture tree without following any link/reparse/mount escape.
- [ ] Two connected cards ingest concurrently, while configured source/destination permits and the memory cap are never exceeded.
- [ ] A slow verifier/destination applies measurable backpressure; queued tasks and memory remain bounded for a million-file synthetic plan.
- [ ] Copy progress is monotonic and reconciles exactly with planned bytes; cancellation stops between chunks and leaves only TASK004-recognized temporary/recovery state.
- [ ] Unplug/replug or TASK002 device-generation change stops affected work and cannot resume against a different card at the same mount path.
- [ ] The same make/model with two different body serials produces two camera keys/folders; model-only media never merges automatically across cameras/runs.
- [ ] Missing, malformed, conflicting, and vendor-specific serial/time metadata remains auditable and produces the documented fallback/conflict state.
- [ ] Still, RAW, HEIF, MOV/MP4, and unknown-binary fixtures exercise camera/time extraction or the explicit fallback. Unsupported parsing never prevents byte copying.
- [ ] Daily and arbitrary N-minute/N-hour buckets are deterministic across OSes, explicit timezones, midnight, leap day, and DST gap/fold cases.
- [ ] Non-UTF-8 source names, Unicode normalization pairs, case-only collisions, reserved Windows names, long paths, duplicate basenames, and pre-existing destinations are preserved or safely projected without overwrite.
- [ ] Source mutation during copy is detected and is not handed to TASK004 as a valid completed copy.
- [ ] Benchmark evidence demonstrates that defaults do not materially regress the best correct portable path; native copy is enabled only for an evidenced win with the same final verification.
- [ ] No operation in this task deletes or formats source media.

## Evidence and test plan

- Unit/property tests for identity normalization, time precedence, interval flooring, DST ambiguity, template validation, path projection, collision resolution, and progress arithmetic.
- Fixture corpus with sanitized real outputs from at least two identical-model camera bodies, images with/without standard body serial, representative maker notes, paired RAW/JPEG, MOV/MP4, timezone/no-timezone metadata, malformed metadata, and unknown files. Store generated/sanitized fixtures only; never commit personal media or real serials.
- Filesystem integration matrix: NTFS/exFAT on Windows, APFS/exFAT on macOS, ext4/exFAT on Linux; case-sensitive and insensitive destinations; local and removable destination where available.
- Fault injection after enumeration, open, chunk read/write, flush, close, metadata re-read, cancellation, and device-generation change.
- Performance results with hardware, OS, filesystem, card/reader class, file distribution, cold/warm-cache caveat, copy-only throughput, end-to-end verified throughput, CPU, and peak RSS.
- Tauri contract tests proving serialized plans/progress preserve IDs and 64-bit byte counts without exposing raw paths/serials unnecessarily.

## Known constraints and follow-up decisions

- Camera body serial metadata can be absent, incorrect, or editable. The product can provide stable, explicit identity evidence and prevent same-model conflation; it cannot manufacture a trustworthy immutable camera ID from model/filename alone.
- Filesystem timestamps and timestamp semantics vary. Any fallback must be visibly lower confidence and recorded.
- Network/cloud destinations may not provide local rename, sync, or capacity semantics. They require a separate destination adapter/capability task; this task targets filesystem paths mounted on the host.
- Actual throughput depends on card, reader, bus sharing, filesystem, antivirus/indexing, destination, and verification reads. "Fast" is certified by the evidence matrix, not a fixed advertised number.

## Completion evidence

Not implemented. Attach code/test paths, benchmark reports, platform/device coverage, and remaining unavailable-hardware blockers here when complete.

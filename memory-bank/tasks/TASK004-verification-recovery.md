# TASK004 — Cryptographic verification, durable manifests, recovery, and format authorization

## Task metadata

- **Status:** In progress — independent BLAKE3 destination verification, same-directory publication followed by final-path writable `sync_all` and exact regular-file re-stat, queued planned-file evidence, and one atomic SQLite completion transaction that commits every exact verified entry, seals the immutable ordered BLAKE3 receipt, and advances the run are locally tested. Interrupted work now retains a flushed deterministic entry-scoped `.partial` checkpoint; explicit recovery removes only that exact owned regular file and copies it afresh, never treating partial bytes as verified. Ordered post-completion source-marker creation and explicit crash recovery are covered: an interrupted run with persisted source/destination snapshots becomes `recovery_required`, not auto-resumed; recovery requires the same current medium key/generation/mount, independently rehashes published final files, copies only missing planned entries, and validates any pre-crash receipt projection. Byte-range resume, reconnect after a changed generation, directory-entry durability, full durability/format gate, and fault/hardware evidence remain pending.
- **Priority:** P0
- **Owner:** Unassigned
- **Depends on:** TASK002 (stable device identity and connection generation), TASK003 (immutable ingest plan, copy outcomes, temporary-file contract), TASK007 (shared local-store migrations and profile/session persistence)
- **Blocks:** TASK005 (format/eject), completion receipts and history UI
- **Target platforms:** Windows, macOS, and Linux desktop
- **Last researched:** 2026-08-23

## Objective

Prove that every planned source file has a byte-for-byte verified destination, make that proof recoverable after interruption, and issue a narrowly bound authorization that TASK005 can use to format only the exact card generation that completed the verified ingest.

Verification must be quick enough to overlap usefully with copying, but correctness wins over optimistic completion. Count/size checks alone are not verification. A green result requires complete manifest coverage, source stability, a cryptographic comparison, successful destination commit, and durable local state.

## Scope boundary

This task owns BLAKE3 verification, per-file/run state machines, the local manifest database, temporary-file recovery, retry/resume, final-path commit, receipts, and the format-authorization contract.

It does not perform formatting or source deletion, and it cannot prove durability beyond the guarantees actually provided by the OS, filesystem, bridge, and hardware cache. TASK005 must independently revalidate device identity/generation and execute the platform-specific format operation.

## Research conclusions and decisions

### Verification definition

A file is `verified` only when all of the following are true:

1. The source entry is in the frozen complete plan.
2. The source was opened from the selected TASK002 device generation and remained stable for the read (size and available handle metadata before/after).
3. Exactly the planned number of bytes were read and written.
4. BLAKE3-256 source content evidence captured during copy matches an independent BLAKE3-256 read of the closed destination temporary file.
5. The destination file was flushed/synced as far as the platform exposes, closed, and committed to the collision-safe final path.
6. The committed final path is re-statted and still has the verified length/identity expected by the manifest.
7. The manifest transition is durably committed.

BLAKE3 is selected for the primary ingest digest because its official implementation is cross-platform, SIMD-aware, incremental, and optionally multi-threaded: [official BLAKE3 implementation](https://github.com/BLAKE3-team/BLAKE3) and [Rust crate features](https://docs.rs/blake3/latest/blake3/). Store the algorithm and schema version with every digest so a future algorithm does not reinterpret old receipts.

The destination hash must come from an independent reopen/read after the writer flushes/closes; hashing bytes only before writing cannot catch a bad write, wrong target, truncation, or later collision. Hash previously copied destination files while the next card file is being read when source/destination permits allow it. On a shared slow disk, verification reads must honor the same destination scheduler to avoid write/read thrashing.

### Logical verification versus physical durability

- Call the result `byte_verified`, not "physically archived." Reopen/read/hash proves equality through the filesystem interface. OS and device caches may still conceal power-loss risk.
- Call `File::sync_all` (or the platform-equivalent adapter), close handles, and sync the containing directory where supported before issuing a format authorization. Record unsupported or failed durability operations explicitly; never silently upgrade them to success. Rust exposes `sync_all` but platform/filesystem guarantees remain external: [Rust `File` API](https://doc.rust-lang.org/std/fs/struct.File.html).
- For the manifest database, use SQLite WAL with `synchronous=FULL`, not `NORMAL`, because SQLite documents that WAL `NORMAL` can lose recent commits after power loss while `FULL` adds a WAL sync after each commit: [SQLite synchronous modes](https://sqlite.org/pragma.html#pragma_synchronous). Verify the returned journal/synchronous modes at startup.
- Keep the SQLite database/WAL on the host application's local data directory, never on the source card or a network filesystem. SQLite WAL requires same-host shared memory and its WAL/SHM files are persistent database state: [SQLite WAL documentation](https://www.sqlite.org/wal.html).
- A format authorization requires byte verification plus successful mandatory sync operations under the destination capability policy. If a filesystem cannot provide the configured durability level, surface `verified_with_durability_warning` and require an explicit policy/user acknowledgement; do not call it unconditionally format-safe.

### Transaction and file state model

Use one monotonic state machine; transitions are transactional and idempotent:

`planned -> copying -> copied_unverified -> verifying -> byte_verified -> committing -> committed -> receipt_included`

Terminal/side states:

- `cancelled`
- `retryable_error`
- `source_changed`
- `source_missing`
- `destination_missing`
- `destination_conflict`
- `verification_failed`
- `durability_warning`
- `quarantined_temp`

No direct transition from `copying`, `copied_unverified`, or `verifying` to `committed`. A run becomes `complete_verified` only when every planned regular file is `receipt_included`, every planned skip is an explicitly allowed non-regular entry, totals reconcile, the ordered manifest root is sealed, and the source device key/generation still matches.

### Durable manifest

Use a versioned SQLite manifest with at least these logical records:

- `ingest_run`: run ID, source device key/generation, source volume snapshot, destination profile/root identity, plan/policy versions, state, totals, timestamps, cancellation/error, manifest root, durability result, and format-authorization state.
- `ingest_file`: entry ID, lossless source-relative path encoding plus display value, source fingerprint, intended/final destination, collision decision, byte length, source and destination BLAKE3, metadata/camera/time/bucket snapshot references, state, attempts, error category, and commit identity.
- `copy_checkpoint`: entry ID, fixed chunk index/offset/length, domain-separated source chunk hash, temporary-file length/identity, and transaction timestamp.
- `state_event`: append-only transition, attempt, reason, and sanitized diagnostic fields for audit/recovery.
- `ingest_receipt`: immutable sealed snapshot version, ordered manifest root, totals, format-gate decision, and export metadata.

Do not store raw media, secrets, or copied logs. Raw camera serials are local sensitive identifiers: store only what TASK003's privacy decision allows, and export redacted receipts by default.

SQLite transactions are atomic even across interruption when configured correctly: [SQLite transactional guarantees](https://www.sqlite.org/transactional.html). Keep transactions small and never hold one open across a file copy/hash. Batch high-frequency progress in memory; persist meaningful state/chunk checkpoints at bounded intervals.

### Chunk checkpoint and resume policy

- Default small-file recovery is file-granular: a non-verified temp is validated or discarded and recopied. This keeps the common path simple.
- Large files use fixed-size, versioned chunks (initial proposal: 16 MiB, benchmark before freezing). During copy, compute a domain-separated BLAKE3 hash per source chunk and transactionally record it only after the corresponding destination bytes are flushed to the temporary file.
- On restart, identify the temp by run/entry ID and exclusive file identity, never name alone. Validate its length is exactly the last committed boundary, hash every committed destination chunk against its stored source chunk hash, truncate uncheckpointed tail bytes, and confirm the source device generation plus file fingerprint. If any check fails, quarantine/restart the file; do not append blindly.
- For a strict resume after interruption, re-read and compare already checkpointed source chunks when the source may have changed. Replaying validated bytes into the incremental hasher restores a standard whole-file BLAKE3 without serializing library-internal hasher state. Optimization may skip the source re-read only for a verifiably read-only source under an explicit policy; it may not silently weaken the default.
- The final destination pass always computes the standard whole-file BLAKE3 and compares it with the standard source hash. Chunk hashes are recovery evidence, not a substitute for the final digest.
- Never serialize private `blake3::Hasher` memory as a persistence format. Persist versioned public digests/checkpoints only.

### Commit, conflicts, and idempotence

- Temporary files are uniquely and exclusively created in the final directory so the final rename stays on one filesystem. Rust documents that rename cannot cross mount points and that replacement behavior has platform details: [Rust `std::fs::rename`](https://doc.rust-lang.org/std/fs/fn.rename.html).
- Revalidate the final portable collision key immediately before commit. If another process/file appeared, hash and classify it; never overwrite it. Resolve a differing collision with TASK003's deterministic suffix and transactionally update the manifest.
- If a final file already belongs to the same sealed entry and still passes full verification, mark an idempotent retry without another copy. Equal content at an unrelated path/entry is not automatic deduplication.
- Rename only a byte-verified closed temp. Sync the final file/parent directory as supported, re-stat it, then commit the manifest transition. If the process crashes between filesystem rename and database commit, startup reconciliation recognizes the final file by entry/run metadata and verifies it before repairing state.
- A verification mismatch never deletes the only temp automatically. Move/rename it into an app-owned quarantine name where safe, record the evidence, and retry from source under the configured retry cap.

### Completeness proof and receipt

Create a deterministic ordered manifest root from domain-separated canonical records containing at least entry ID, lossless source-relative path encoding, final relative destination, byte length, source BLAKE3, destination BLAKE3, camera key, capture/bucket decision, and final state. Use length-prefixed binary canonicalization, not ambiguous string concatenation.

The sealed receipt includes:

- run/source device key and connection generation
- destination profile/root identity snapshot
- planned, verified, committed, skipped, failed counts and bytes
- per-file algorithm/version and digest (redacted export may omit sensitive path/camera fields)
- manifest-root algorithm/version and root digest
- copy/verify timing and warnings
- durability capability/result
- exact format-gate decision and reason

A "quick recheck" may validate database integrity, receipt signature/root, file count/size, and selected hashes, but must be labelled `quick_audit`. Only a complete destination content rehash is a current `byte_verified` re-verification. Do not present sampling/counts as equivalent.

### Format authorization contract

Issue an opaque, short-lived, single-use `FormatAuthorization` only when:

- run state is `complete_verified`;
- every planned file is included in the sealed receipt;
- no source changed/missing, verification failure, unresolved conflict, or pending temp exists;
- destination durability policy is satisfied or the separately recorded acknowledgement policy allows continuation;
- TASK002 reports the exact same immutable source device key and connection generation still present;
- authorization binds run ID, receipt root, source device key, connection generation, source volume/partition identity, expiry, and a random nonce.

TASK005 must consume the authorization transactionally and re-check all bound identities immediately before any unmount/format command. A mount path, drive letter, label, card model, or reader slot alone can never authorize formatting.

## Required Rust/Tauri contracts

- `VerificationRequest` and `VerificationResult` keyed by run/entry IDs and opaque backend path references.
- `SourceDigestEvidence`: algorithm/version, byte count, full digest, chunk scheme/version/checkpoints, and source before/after fingerprint.
- `DestinationDigestEvidence`: independent-read algorithm/version, byte count, digest, file identity, sync capability/result, and final re-stat.
- `IngestRunStatus`: exact state/totals, warnings, retryable/terminal failures, progress, and whether a format authorization can be requested.
- `IngestReceipt`: versioned sealed record with canonical manifest root and export-redaction mode.
- `FormatAuthorization`: opaque token returned to the UI; raw device handles/paths and authorization internals remain backend-only.
- `RecoveryAction`: resume, restart file, retry verify, reconcile committed file, quarantine temp, or abandon run. Abandonment never deletes source files and deletes/quarantines app-owned temps only after explicit confirmation/policy.

Suggested implementation dependencies, to be pinned after license/MSRV review:

- official `blake3` crate with `rayon` for explicit large-file parallel methods
- SQLite via the repository's chosen Rust binding, bundled/pinned consistently across platforms
- `serde` for versioned records, `uuid` for run/entry IDs, and structured error serialization
- the bounded worker/cancellation primitives selected in TASK003

Do not add a second manifest store. SQLite is the authority; exported JSON/CSV receipts are immutable projections.

## Recovery algorithm

On startup or device reconnect:

1. Open the local manifest with configured pragmas, run an integrity check appropriate to startup cost, and verify recorded schema version/migrations.
2. Find nonterminal runs; never auto-resume against a mount path alone.
3. Reconcile manifest state with temporary and final files using entry/run IDs, exclusive file identity, size, and hashes as required.
4. Require TASK002's same device key. A new connection generation is allowed only after explicit identity revalidation; a different device blocks resume even if it reused the same slot/letter.
5. For each interrupted large file, validate/truncate to the last committed chunk boundary or quarantine/restart it. For small files, restart from zero unless the completed temp independently verifies.
6. Resume through the normal bounded TASK003 pipeline, preserving attempts/errors and monotonic aggregate totals.
7. Re-seal the receipt only after all current entries pass. Any plan change creates a new plan/receipt version and invalidates prior format authorizations.

Retry only classified transient failures (temporary sharing violation, disconnect awaiting same device, short-lived destination unavailable) with bounded exponential backoff and jitter. Permission denial, out-of-space, source mutation, path policy violation, digest mismatch, device identity change, and repeated I/O failure require user action or a fresh attempt; no infinite loops.

## Implementation work packages

### TASK004.1 — Manifest schema and state machine

- Define versioned records, legal transitions, transactional repository, pragmas, migrations, and startup integrity handling.
- Enforce invariants in both domain logic and database constraints where practical.

### TASK004.2 — Streaming/checkpoint evidence

- Integrate TASK003 source full/chunk digests.
- Persist bounded chunk checkpoints without progress-write amplification.
- Implement strict chunk validation/truncation/resume and file-granular fallback.

### TASK004.3 — Independent destination verification and commit

- Reopen/read/hash temps through the destination scheduler.
- Implement sync/close, collision recheck, same-directory commit, final re-stat, and crash-window reconciliation.
- Keep mismatch artifacts quarantined and auditable.

### TASK004.4 — Completeness root and receipts

- Canonicalize ordered records, seal/validate the manifest root, export redacted receipts, and distinguish full reverify from quick audit.

### TASK004.5 — Format gate

- Issue/consume single-use, expiring, exact-device-generation authorizations.
- Provide denial reasons that the UI can explain and TASK005 can enforce independently.

## Acceptance criteria

- [ ] A bit changed, inserted, deleted, reordered, truncated, zero-filled, or written to the wrong destination file causes verification failure.
- [ ] Count/size-only equality cannot produce `byte_verified` or a format authorization.
- [ ] Destination hashing is an independent reopen/read after close/sync and uses the same versioned BLAKE3-256 definition as source hashing.
- [ ] Final completion requires every planned regular file and exact planned byte total; one missing, unstable, cancelled, quarantined, or failed entry blocks completion and formatting.
- [ ] The manifest state machine rejects illegal transitions and remains consistent under injected process termination at every transition boundary.
- [ ] Startup reconciliation repairs the rename-before-database-commit window only after verifying the final file; it never assumes name/size equality.
- [ ] Large-file interruption resumes only from validated fixed chunk boundaries; corrupt/truncated/extra-tail temps cannot be appended to.
- [ ] Source change, same mount path with a different device, or conflicting connection generation blocks automatic resume.
- [ ] Concurrent cards and multiple verification workers cannot cross-link runs, files, destinations, digests, progress, or format authorizations.
- [ ] Pre-existing equal/different files, case-fold collisions, external writes during commit, out-of-space, permission changes, disconnects, and retry exhaustion have deterministic non-overwriting outcomes.
- [ ] SQLite mode is read back and asserted as local-host WAL plus `synchronous=FULL`; database/WAL relocation/copy code treats WAL/SHM as a unit or uses SQLite's supported backup/checkpoint path.
- [ ] Receipt canonicalization produces the same manifest root on Windows, macOS, and Linux for identical logical records, including Unicode and non-UTF-8 source-name encodings.
- [ ] A quick audit is visibly distinct from full byte verification; only full evidence can satisfy a new strict re-verification request.
- [ ] Format authorization is single-use, expires, is bound to run/receipt/device/volume/generation, and is denied for any incomplete or durability-blocked run.
- [ ] No recovery, verification, receipt, or format-gate path deletes or modifies source files.

## Evidence and test plan

- Known-answer BLAKE3 vectors plus generated files at 0 B, 1 B, chunk boundary ±1 B, buffer boundary ±1 B, sparse-looking content, multi-GiB logical size where feasible, and many-small-file sets.
- Mutation matrix for source during read and destination after write: single-bit, truncation, extension, swap, wrong-target, same-size corruption, stale pre-existing file, and timestamp-preserving corruption.
- Deterministic crash/fault injection after each state transaction, chunk write, chunk checkpoint, temp sync, destination hash, rename, directory sync, receipt seal, and authorization issue/consume.
- Power-loss testing where safe/available on disposable destination media; otherwise record it as an external-hardware blocker and do not equate process-kill tests with power-loss proof.
- OS/filesystem matrix: NTFS/exFAT (Windows), APFS/exFAT (macOS), ext4/exFAT (Linux), plus network filesystem only as an explicitly unsupported/capability-warning case.
- Concurrency tests for two sources to one destination, two destinations, cancellation, app close, card removal/reinsert, destination removal, and manifest contention.
- Property tests for legal state transitions, canonical record encoding, manifest-root determinism, format-token binding/expiry/single use, and retry caps.
- Performance report separating copy-only and end-to-end verified throughput, verifier overlap, source/destination contention, hash CPU, manifest write amplification, recovery validation time, and cold/warm-cache limitations.
- Readback tests from the Tauri boundary proving the UI never receives raw authorization internals and cannot manufacture a successful gate from booleans.

## Performance targets to establish during implementation

Targets must be baselined on named hardware rather than invented in advance. The implementation evidence must establish and then enforce regression thresholds for:

- end-to-end verified throughput versus raw OS copy on the same card/destination
- time-to-first-progress and UI progress cadence
- peak RSS under a million-file plan and multi-card copy
- verification overlap efficiency on SSD and rotational destinations
- cancellation latency bounded by one copy/hash chunk plus OS I/O completion
- restart scan and large-file resume validation time
- SQLite transaction rate/WAL size/checkpoint pause under many small files

## Known constraints and blockers

- No application can guarantee that bytes reached nonvolatile media when an OS/device bridge falsely reports flush completion. Record the achieved filesystem-level guarantee and require stronger workflow policy where the destination is operationally critical.
- Full byte verification necessarily reads destination content. It can overlap and use BLAKE3/SIMD/parallel files, but it cannot be replaced by count/size checks without weakening reliability.
- Resume cannot be authorized when the source identity is unavailable/ambiguous. Mount path, label, reader model, and slot are insufficient.
- Cross-platform power-loss and actual SD/reader evidence requires physical devices and disposable test media. Missing hardware is a certification blocker, not a reason to mark the feature complete.

## Completion evidence

Partial implementation: `src-tauri/src/local_store.rs` schema version 5 retains native-only source/destination roots and recovery-required records. `src-tauri/src/lib.rs` provides the explicit, identity-checked `resume_verified_ingest` command, and `src-tauri/src/ingest.rs` independently rehashes existing final files. Local tests cover legacy-row fail-closed behavior, the recovery transition/retry contract, corrupt-existing-file rejection, and a partial final-tree crash window. Attach checkpoint/fault matrix, cross-platform, and hardware evidence before completion.

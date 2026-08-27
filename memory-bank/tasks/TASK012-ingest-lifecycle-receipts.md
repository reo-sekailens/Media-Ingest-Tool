# TASK012 — Source lifecycle, receipts, eject, and operational recovery

- **Status:** in progress — bounded-copy cancellation is exposed through an opaque client-generated UUID validated by Rust, checked between copy/verification chunks, and reported through typed terminal as well as queued/copying/verifying updates. Actual 1 MiB writes report aggregate bytes plus a one-based frozen-plan file ordinal and total; the immediately following independent destination digest readback reports the same path-free file metadata under the `verifying` stage. Cancellation now flushes and retains an entry-scoped `.partial` checkpoint. It remains unverified and is removed only by explicit recovery of that same frozen plan entry, which copies it afresh; no partial byte range is resumed or reported transferred. Cancelled and failed terminal updates retain aggregate bytes already written. Native snapshots now also issue a per-medium connection generation that survives refreshes and changes only after observed removal/reinsertion; ingest start and format eligibility require that generation and use native, not webview-provided, identity confidence. On Windows, a monitored source that was previously observed in a native snapshot now triggers cooperative cancellation if its exact current media-key/mount pairing disappears. Each snapshot now additionally probes every mounted Windows volume with `IOCTL_STORAGE_CHECK_VERIFY` and reconciliation runs at most once per second, so a card absence is detectable even when the shared USB reader emits no disk-interface event. This avoids mount-arrival false positives but is not reconnect-safe resume or immutable-medium proof. A native close request now remains open while the Rust scheduler has any active ingest and emits an explicit keep-or-cancel choice to the operator; cancellation is cooperative and returns the runs to durable recovery rather than falsely completing them. On desktop Tauri, suspend/resume window events are unavailable; the app therefore forces a native device/history recheck when it regains focus, but full sleep/wake behavior still needs a platform-specific power-event adapter and hardware proof. Safe eject now revalidates an exact sealed receipt, current media key/generation, unique current mount, and no active ingest before opening a provider-owned exclusive volume handle. The Windows provider flushes, locks, and dismounts the volume, maps it to the exact disk PnP devnode, then calls `CM_Request_Device_EjectW`; it never traverses to a shared USB reader/hub, reports veto/non-ejectable failures accurately, and remains unavailable on other platforms. A matching sealed history row restores the eject control after restart. It has no native hardware proof yet. Byte-range checkpointing, sleep/wake adapter, non-Windows providers, and hardware lifecycle evidence remain pending.
- **Depends on:** TASK002, TASK003, TASK004, TASK007, TASK008

## Objective

Current packaged validation: a cancelled 2.688 GB microSD verification reached `recovery_required` with no receipt; the installed rebuild then resumed that exact unchanged marker-backed generation and sealed a five-file / 2,688,024,576-byte receipt with matching persisted BLAKE3 digests. The current package also cancelled a copy after it created an entry-scoped `.partial`; explicit recovery removed that partial, copied from the frozen plan, sealed `caf94fcc-89c9-453c-afcc-3201c5022a0d`, and independently SHA-256 matched all five files. Physical microSD removal took the live inventory from two cards to one; reinsertion restored two and rendered `Insertion 3`. The registered generation-3 automatic run sealed `ca981d73-a337-49da-bf23-134bd93dca3d` for five files / 2,688,024,576 bytes, with independent SHA-256 matching all five destination files; restart suppressed a duplicate generation-3 run. The original UI did not receive its terminal state before restart even though the receipt/ledger were complete, so terminal delivery requires repair. The corrected eject package attempted the selected SD card but Windows PnP vetoed it; the two PRO-READER LUN disk devices expose no eject-support capability, so this reader cannot be software-ejected without affecting its shared parent. This is Windows package evidence only; crash/sleep, removal during ingest, supported-hardware eject, destination-failure, second-reader, macOS, and Linux lifecycle evidence remain open.

Close the operational gaps around hotplug, pause/cancel, app shutdown, sleep/wake, safe ejection, and handoff receipts so an ingest can be understood and recovered without guesswork.

## Scope

- Model connection generations separately from durable device identity; never continue work on a reused mount path without a fresh exact match.
- Define cooperative cancellation at file/chunk boundaries, including which blocking operations cannot stop immediately.
- Persist checkpoints before reporting progress as durable.
- Reconcile active sessions on launch and after sleep/wake by re-enumerating sources and destinations.
- Reconcile opt-in registered cards on mount only through their exact app marker and current connection generation; never treat a mount path, slot, or volume identifier as a match. Treat this as convenience continuity rather than immutable identity, and record source/preflight errors instead of silently retrying.
- Provide safe eject only after all file handles and workers are closed, manifests are durable, and the OS confirms flush/unmount/eject.
- Generate immutable completion receipts and append-only operational logs with sanitized export.
- Define retry policy for individual files and sessions; no infinite retries and no automatic formatting after recovery.

## Acceptance criteria

- Removing and reconnecting the same card resumes only verified/eligible files; a different card on the same path never resumes the session.
- Cancel leaves `.partial` files and database state in a documented recoverable state and never labels them transferred.
- Closing the window cannot abandon an active native worker without a visible choice and durable checkpoint.
- Safe eject reports OS rejection or busy handles accurately and never implies the physical card is safe before confirmation.
- Receipts distinguish copied, verified, skipped-identical, failed, cancelled, and source-changed files.
- Mount-triggered ingest starts only for an explicitly registered marker after current-mount, destination/preflight, and no-conflict checks; its ordinary cancellation action is available during the run. It does not claim a marker is immutable hardware identity.
- Crash, sleep/wake, and reconnect scenarios pass on every supported OS in TASK010.

## Verification evidence

- Deterministic state-machine tests for cancel, removal, reconnect, shutdown, resume, and eject.
- Fault-injected native runs on each supported OS with sanitized event timelines and receipts.
- Packaged-app close/sleep/wake/eject screenshots and logs, recorded separately from fixture tests.
- Receipt schema fixtures proving every terminal file outcome and connection generation is represented.

## Research sources

- [Tokio blocking task cancellation limitations](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Windows PnP safe-removal API](https://learn.microsoft.com/en-us/windows/win32/api/cfgmgr32/nf-cfgmgr32-cm_request_device_ejectw)
- [Apple Disk Arbitration eject API](https://developer.apple.com/documentation/diskarbitration/dadiskeject%28_%3A_%3A_%3A_%3A%29)
- [UDisks2 Drive.Eject](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Drive.html)

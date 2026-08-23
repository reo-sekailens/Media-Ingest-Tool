# TASK012 — Source lifecycle, receipts, eject, and operational recovery

- **Status:** in progress — bounded-copy cancellation is exposed through an opaque client-generated UUID validated by Rust, checked between copy/verification chunks, and reported through typed terminal as well as queued/copying/verifying updates. Copying bytes come from actual 1 MiB writes; cancelled and failed terminal updates retain aggregate bytes already written. Native snapshots now also issue a per-medium connection generation that survives refreshes and changes only after observed removal/reinsertion; ingest start and format eligibility require that generation and use native, not webview-provided, identity confidence. On Windows, a monitored source that was previously observed in a native snapshot now triggers cooperative cancellation if its exact current media-key/mount pairing disappears. This avoids mount-arrival false positives but is not reconnect-safe resume or immutable-medium proof. Safe eject remains deliberately unavailable: current discovery has no provider-owned disk/volume handle bound to the immutable medium, and a drive letter/reader name cannot safely authorize an eject call. Checkpoint persistence, per-file/verification progress, shutdown/sleep handling, the required provider binding, and hardware lifecycle evidence remain pending.
- **Depends on:** TASK002, TASK003, TASK004, TASK007, TASK008

## Objective

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
- [Windows eject request API](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-ioctl_storage_eject_media)
- [Apple Disk Arbitration eject API](https://developer.apple.com/documentation/diskarbitration/dadiskeject%28_%3A_%3A_%3A_%3A%29)
- [UDisks2 Drive.Eject](https://storaged.org/doc/udisks2-api/latest/gdbus-org.freedesktop.UDisks2.Drive.html)

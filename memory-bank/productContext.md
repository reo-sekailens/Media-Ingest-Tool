# Product Context

## User needs

- See every newly attached card quickly and know which physical card/reader slot it represents.
- Review trustworthy capacity, filesystem, mount, read-only, model, serial, topology, and identity-confidence details.
- Remember a different destination and sorting rule for each exactly identified card.
- Ingest multiple cards quickly without allowing concurrency to overload one reader, USB controller, source, or destination.
- Keep media from same-model cameras separate and receive an auditable proof that every file arrived intact.
- Reuse an eligible card quickly after verified ingest without risking the wrong disk.

## Primary workflows

1. Attach one or more cards; the app creates a new connection generation and enumerates physical disk, volume, reader, slot/topology, and mount evidence.
2. Confirm or choose each card's destination, camera identity, timezone, and folder-template preview.
3. Preflight source inventory, collisions, destination capacity, filesystem limits, and identity stability.
4. Copy using bounded device-aware concurrency into temporary files while hashing the source byte stream.
5. Re-read each destination file, compare its full BLAKE3 digest, atomically publish it, and produce a durable receipt.
6. Safely eject, or choose guarded quick format only when the exact current card and completed verification receipt remain eligible.

## Product boundaries

- Desktop-local operation is the current plan; accounts, cloud storage, remote databases, analytics, and a backend are out of scope.
- The first release copies regular files and directories. It does not edit, transcode, catalog, upload, or delete destination media.
- Existing destination files are never overwritten silently. Exact digest matches may be skipped with a receipt; conflicts require an explicit policy.
- Reader-slot detection is evidence-based. Unknown models remain generic instead of receiving a guessed slot label.
- The app can prepare a standard filesystem, but “camera ready” is only claimed for camera/card profiles that have been tested on that exact workflow.

## Terminology

- **Source card:** removable storage containing files to ingest.
- **Reader:** host-connected hardware that exposes one or more card slots.
- **Connection generation:** one continuous attachment/mount lifetime; changes on removal/reconnect.
- **Identity evidence:** a namespaced hardware, OS, volume, topology, or content value observed for a source.
- **Identity confidence:** `hardware-card`, `hardware-disk`, `reader-slot`, `volume`, or `session-only`; only the first two are candidates for automatic durable matching.
- **Camera identity:** make + model + body serial when present; otherwise explicitly unresolved or operator-assigned.
- **Ingest:** inventory, copy, verification, and receipt creation.
- **Verified:** full source digest equals a fresh full destination read for the current ingest.
- **Quick format:** filesystem reinitialization without secure overwrite.

Related: [Project brief](projectbrief.md), [Active context](activeContext.md), and [Tasks](tasks/_index.md).

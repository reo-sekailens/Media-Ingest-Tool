# Active Context

## Current focus

Implement the remaining TASK004 recovery/durability boundaries alongside conservative TASK002 device identity evidence.

## Known facts

- User-selected stack: Tauri with Rust, React, TypeScript, and Tailwind CSS.
- Required workflows: cross-platform removable-device discovery/details, opt-in per-card registration and mount-triggered ingest, per-device destinations, bounded concurrent ingest, camera/time sorting, full verification, quick format, and SanDisk SD/microSD slot mapping.
- Reliable immutable identity must use an evidence hierarchy because mount/volume identifiers are mutable and card hardware IDs are not always exposed through USB readers.
- Tauri 2 + Rust + React + TypeScript + Vite + Tailwind foundation is present, with Node 24.14.0 and Rust 1.98.0 policy files.
- The Windows-local foundation gate has passed: formatting, lint, strict typecheck, Vitest, Vite build, Rust fmt/clippy/test, and the memory-bank verifier.
- Tauri custom commands are permission-manifested and limited to the main window; no filesystem or shell plugin is enabled.
- Windows discovery enumerates currently mounted removable volumes through Win32 APIs and maps only format-created volume serial data into filesystem-scoped evidence. It does not yet infer physical-card identity.
- Live Windows reader evidence: the connected SanDisk PRO-READER revision `0056` exposed a sacrificial full-size SD card as logical unit 0 and an empty companion as logical unit 1. The app rendered the same two-unit topology. This calibrates only this reader's LUN 0 as SD; it does not identify the card immutably, and microSD calibration remains pending.
- Windows recognizes the exact storage-descriptor family `SanDisk` + `PRO-READER` as a non-authoritative `sandisk_pro_reader` presentation/calibration hint. The derived reader fingerprint plus LUN remains the actual calibration key, and no reader-family recognition can authorize card recall or formatting.
- A clean sacrificial card may contain Windows-created root `System Volume Information`; the ingest inventory explicitly excludes only that host-metadata directory so its protected ACL does not block a normal clean-card scan. Other inaccessible entries are not skipped.
- Hardware copy evidence exists for the connected SD card: three controlled files totaling 1,310,762 bytes passed the verified core to both `F:\dummy\1` and `F:\dummy\2`, with receipts and independent SHA-256 comparisons. The test intentionally bypassed the Tauri command lifecycle, so native UI/progress, SQLite completion, and marker timing still require an IPC-level run.
- The desktop UI now exposes a Rust-owned source-media scan for the currently selected mounted card. The command rechecks the card's current media key and mount before enumerating regular files, and the UI renders the resulting file/byte count per selected card. Browser fixture QA proves the preview-only guard and status message; native scan execution remains unverified.
- A remembered destination is now exposed through a Rust-only profile boundary. Both lookup and save re-enumerate the current source and use its observed identity confidence; only `hardware_immutable`/`hardware_stable` media can use SQLite destination recall. The currently connected USB reader has unresolved card identity, so it remains session-only by design.
- The ingest ledger persists a complete queued plan, exact verified-file entries, and an ordered domain-separated BLAKE3 manifest root. A native run cannot transition to completed unless its matching immutable SQLite receipt seal is stored.
- The native completion seam is covered by a fixture lifecycle test: the source marker is absent through copying, verified entries are persisted and the receipt is sealed before the completed transition, and only afterward is the marker created. This is a local durable-order test, not an IPC or physical-card proof.
- Per-card setup now creates or recognizes the app marker, saves a local destination/sort/auto-ingest/auto-format profile keyed to that exact marker, and starts a verified ingest once for each newly observed connection generation when the same opted-in marker reappears. This is personal-workflow continuity, not immutable device proof: copied/changed markers or unavailable destinations can prevent or misdirect recall, and the app must not use marker evidence for formatting. The new `format_provider` boundary carries only native resolved-target/profile/validated-mount types and presently rejects every platform until Windows Storage Management, macOS Disk Arbitration, and Linux UDisks providers are implemented. Marker restoration is available only after a provider validates the remounted filesystem. Normal filesystem permissions, hidden/read-only attributes, or ACLs cannot prevent outside-app writes/deletes; a full-size SD physical write-lock is the only broadly effective external write barrier.
- The format control now asks the native backend for a non-destructive eligibility explanation after a completed ingest. Eligible cards can request a 60-second opaque authorization, then require an explicit modal confirmation that names the media, capacity, sealed run, inferred profile, and recoverability warning. The confirmation submits only the token; it sends no mount path, drive letter, or provider arguments.
- A configured auto-format now re-resolves the source by immutable key, connection generation, mount, and registered marker after receipt sealing. Until a native provider is installed it reports `skipped`, rather than a destructive-operation failure, and does not touch the card. The connected PRO-READER card still has unresolved USB-reader identity and is therefore ineligible for auto-format even with a marker profile.
- Registered auto-ingest profiles persist the selected camera-interval duration as well as the sort mode. On remount, the same one-minute or one-hour interval is supplied to the native verified-ingest planner instead of silently defaulting to an hour. The native boundary rejects an interval outside 1–1,440 minutes and disables an auto-format preference when auto-ingest is not also enabled.
- Windows now has an in-process Storage Management WMI provider: it resolves only the native-discovered mounted drive to one `MSFT_Volume`, checks the current capacity, holds its opaque WMI path for the actual `MSFT_Volume.Format` quick-format call (`Full=false`, `Force=false`), then waits for a matching remount. It has no shell/Powershell path. A read-only probe bound the live sacrificial `D:` card to WMI successfully; no destructive format was run. macOS and Linux remain fail-closed until their native providers are implemented. A successful auto format restores the registered marker and writes an immutable local format receipt tied to the sealed ingest run; an empty ingest is skipped to prevent reformatting a blank card on each mount.
- macOS now has a source-level `diskutil` adapter selected on that OS. It accepts only the current native mount root, requires a removable writable volume identifier and exact capacity match, runs the absolute system `diskutil eraseVolume` with fixed quick-format arguments, and polls the remounted filesystem before marker restoration. It has no macOS compile, runtime, authorization, or hardware evidence yet. Linux remains fail-closed pending UDisks2 D-Bus integration.
- Linux now selects a source-level direct UDisks2 D-Bus provider. It resolves the current `/proc/self/mountinfo` `/dev` source to a UDisks block object, requires a removable sysfs ancestor, refuses `HintSystem`/read-only devices, requests UDisks `Block.Format` with the `erase=quick` option, and validates a remounted filesystem. It has no Linux runtime evidence. A cross-target check from Windows reached the existing Tauri/libdbus native-dependency boundary before compiling the app; a Linux CI or host is needed for that platform build and UDisks hardware proof.
- Auto-format now runs from both the normal verified-ingest completion path and recovery completion path on Tauri's blocking pool, with a typed `formatting` progress stage. It is explicitly limited to the mount-triggered flow; a user-started manual ingest and a zero-file card return without attempting format. Unit coverage confirms both guard cases.
- The read-only Windows sacrificial-card probe now also reopens the opaque WMI volume path and builds the documented `MSFT_Volume.Format` input object (`FileSystem=exFAT`, `Full=false`, `Force=false`) without executing the method. This corrected an instance-versus-class WMI method lookup issue before any destructive attempt.
- Device snapshots now carry a per-medium connection generation, distinct from their refresh sequence. A generation remains stable through refreshes but increments after the medium is observed absent and then reappears; ingest start and format eligibility require the stored generation. Fresh native observation also replaces any webview-provided identity confidence before an ingest run is persisted.
- The operator can request a native, read-only organization preview for the current media, destination, and sort rule. It revalidates media key, mount, and connection generation before enumerating and returns only totals plus sample relative paths. Its opaque operation UUID is reused by the subsequent ingest so run-scoped unknown-camera paths remain stable; changing the destination or rule discards the preview. Browser fixtures expose only the desktop-runtime guard, not a native preview result.
- After same-directory publication, each verified destination is reopened with the writable flush access Windows requires, synced again, and re-statted through its final path as a regular file of the exact verified length before it can enter the manifest. This is a filesystem-level durability improvement, not proof of directory-entry persistence, physical-media flush, or power-loss survival.
- The final SQLite completion step is now one transaction: every exact planned entry is committed, the immutable receipt is sealed, and the run transitions to completed together. Any later entry mismatch rolls back earlier updates, leaving the run in copying for startup reconciliation rather than exposing a partial completion manifest.
- The React client now tracks active operation/cancellation state and progress labels by source-media key rather than one global ingest. A second selected card can start its own native bounded ingest while another is active, and its stop control remains bound to its own opaque operation ID. Browser fixture mode does not execute these native concurrent transfers.
- Native format authorizations are now consumed by the operator-initiated execution command. It removes the opaque authorization before the final native snapshot/provider path, rechecks the sealed receipt, immutable medium key, insertion generation, capacity-derived profile, and unique mount, then invokes the platform provider, validates the remount with a durable sentinel, conditionally restores a registered marker, and records the format receipt. The command has fixture/source checks only; destructive hardware behavior remains unverified.
- Shared source enumeration now has a fixed default ceiling of one million regular files and 64 nested directories. Scan, preview, and ingest all reject a tree exceeding either bound before persisting a plan; the limits are a hostile-media safety boundary, not a hardware performance certification.
- Generated destination paths now project every component into a portable form before copy: control/reserved Windows characters and names, trailing dots/spaces, and oversized components are normalized deterministically. The copy primitive independently rejects an unportable destination supplied by any caller; source components remain preserved as source evidence.
- In-plan destination collisions now use NFC normalization plus Unicode default case folding rather than native `PathBuf` equality. A plan therefore resolves source names that differ only by case or canonically equivalent Unicode into deterministic distinct destination names before any copy starts.
- Format readiness now computes only an allowlisted generic filesystem profile from current card capacity: FAT through 2 GiB, FAT32 through 32 GiB, and exFAT through 2 TiB. The profile remains visibly inferred; SDUC and camera-specific profiles are withheld pending certification, and no formatter is invoked.
- Source enumeration and direct copy now reject Unix entries whose filesystem device ID differs from the selected root, preventing an in-card mount point from escaping to another filesystem. Windows continues to reject mount points as reparse entries. This is metadata-level protection; handle-based anti-race traversal remains pending.
- Native ingests now lease a resolved destination root for the complete copy/verification operation. Different destination roots remain concurrent, while a second operation aimed at the same root waits instead of multiplying read/write verification pressure on that volume. The limit is deliberately conservative until hardware benchmarks establish a safe higher per-volume concurrency.
- The two opt-in Windows hardware probes now use the controlled `D:\\DCIM\\100DUMMY` fixture rather than the card root, because the sacrificial card contains unrelated screenshots. They create unique test-only folders below each approved dummy destination and never overwrite existing destination content. On 2026-08-24 both passed, and a separate SHA-256 comparison found zero mismatches across the three copied files in each destination.
- Interrupted native runs now persist their source and destination roots only inside SQLite. At startup, a complete interrupted record becomes `recovery_required`; legacy/incomplete rows fail closed. The operator may explicitly resume only when current discovery reports the exact stored medium key, connection generation, and mount. Recovery rehashes every existing final file against the frozen source plan, copies only missing paths, and accepts a pre-crash JSON receipt only when it exactly matches newly derived evidence. Any recovery failure returns to `recovery_required`, never auto-retries or overwrites a conflicting destination. Large-file chunk checkpoints and cross-reinsert recovery remain pending.
- Format tokens are now bound to the selected sealed run ID and backend allowlisted capacity profile in addition to the exact immutable medium/generation and expiry. This tightens the future provider contract; it does not add a destructive OS formatter or make the currently unresolved USB-reader card formatable.
- Sudden copy/verification cancellation or worker termination now returns the run to `recovery_required` rather than a terminal failure. An explicit resume after reinsertion accepts a changed connection generation only when fresh native discovery reports the same `hardware_immutable` medium key at the stored mount; the new generation is persisted before completion, so a reused drive letter or mutable marker cannot continue the run.
- Each ingest accepts an opaque UUID cancellation handle. The Rust scheduler rejects duplicate active IDs and cooperatively cancels at existing copy/hash chunk boundaries; it does not claim immediate interruption of a blocking filesystem call.
- Generated Git commit messages must include a context-specific imperative title and a blank-line-separated body describing material additions, changes, removals, rationale, and relevant verification or compatibility notes.
- Auto-format is additionally interlocked with the ingest scheduler: while any other active ingest still names the same immutable medium key, the completed mount-triggered run reports `skipped` and no provider is resolved. This avoids erasing a card another operation may still be reading; a later mount-triggered ingest is required to reconsider formatting.
- macOS and Linux now feed the same mount-triggered registered-card workflow through a native snapshot reconciler that polls once per second. It uses the same native discovery, connection-generation, source-removal cancellation, and channel-closure semantics as the Windows snapshot pipeline; it is a bounded fallback until a platform event subscription is implemented.
- A fresh 2026-08-25 read-only Windows Storage Management probe confirmed the attached SanDisk PRO-READER card is not represented as an `MSFT_Disk` at all: the system exposes only its mounted logical volume, while all returned `MSFT_Disk` objects are internal drives. There is therefore no opaque whole-disk identifier to strengthen the existing USB-reader identity gate; formatting this card must remain unavailable.
- On 2026-08-26, the Windows native UI observed the PRO-READER's LUN 0 no-media endpoint and a mounted 511.8 GB exFAT card on LUN 1. A controlled three-file, 3,670,016-byte fixture on `M:` scanned and previewed successfully, then completed native verified ingest to the isolated `F:` certification destination. The receipt `92ff1f3c-e591-447f-901d-82d63e5dc441.json` was sealed, the source marker was created, and an independent SHA-256 comparison found zero mismatches. The app correctly withheld quick format because card identity remained unresolved. No physical slot label was calibrated in this run, and no destructive formatter, auto-ingest remount, eject, cancellation/recovery, or two-card concurrency path was exercised.
- The operator then confirmed the physical placement: the 255.8 GB LUN 0 card is full-size SD and the 511.8 GB LUN 1 card is microSD. Windows-native calibration was persisted through the app; the refreshed inventory visibly labels LUN 0 `SD slot (calibrated)` and LUN 1 `microSD slot (calibrated)`. This is a single-reader, single-session calibration only; reconnect, restart, second-reader, and non-Windows repeat evidence remains required for certification.
- A later read-only scan of the full-size SD card found 19 regular files rather than an empty controlled fixture, including files outside the new test folder. No full-card ingest was started, so no unrelated content was copied to the certification destination. The full-size card's end-to-end ingest remains unproven in this pass.
- Safe eject now has a Windows-only, fail-closed native path. It accepts no IPC mount path: a sealed receipt, current medium key, exact connection generation, current unique mount, and no active ingest are rechecked before the volume is flushed, locked, dismounted, mapped to its exact disk PnP devnode, and submitted to `CM_Request_Device_EjectW` only after the app releases its own handle. The UI restores eligibility from a matching sealed history row after restart. It never ejects a shared reader or hub. Rust/provider and desktop-IPC fixtures pass; unsupported platforms remain unavailable.
- On 2026-08-26, safe eject was replaced with the Windows PnP `CM_Request_Device_EjectW` request for the exact disk devnode after the app releases its own volume handle. The packaged Windows retry still returned a PnP veto and left both cards mounted. Read-only PnP inspection established why: both SanDisk PRO-READER LUN disk devices expose capability `0x10` (unique ID only), while their shared `USB\\VID_0781&PID_D003` parent exposes `0x94` but not the Windows eject-supported bit. This reader cannot be software-ejected through Windows PnP; the application must report the refusal rather than claim success or remove the shared parent holding the other card.
- Native desktop close requests now check the Rust-owned active scheduler before allowing the window to disappear. If any ingest is active, the close is prevented and the webview receives an explicit keep-ingesting or cancel-ingests choice; cancellation remains cooperative at the existing chunk boundaries, so a later close is still required after the worker has reached recovery. A browser fixture covers the no-mounted-card dialog surface. Desktop Tauri exposes no sleep/wake window events, so foreground focus triggers a native inventory/history recheck; full power-event handling and native close/power-loss evidence remain pending.
- Actual verified-copy writes now carry an aggregate byte count and path-free frozen-plan file ordinal through the typed native progress channel. Manual, recovery, and auto-ingest labels render `file n of total`; Rust and browser-fixture tests cover the metadata path.
- The fresh destination BLAKE3 readback now emits the same per-file ordinal and actual bytes as `verifying`, with separate copy and verification aggregate counters so either stage remains bounded by the frozen plan total. Rust multi-worker coverage checks both byte totals and the auto-ingest fixture renders the verification update. Rehash progress for files already published before crash recovery remains terminal-only.
- Fresh native Windows evidence now covers the registered auto-ingest path without auto-format: the packaged app registered the existing M: marker to isolated `F:\media-ingest-auto-cert-20260826`, then after restart observations completed sealed runs `9df43fc8-0179-4b02-86db-940e424fc4f9`, `1c6f56b8-c21a-4c63-a4bd-f018ff1debf6`, and `f25658e3-aa58-414f-9e00-fff8705e77d3` for the three controlled M: files. The latter two are recorded in the local ledger as completed from `M:\` to the registered destination at generation 2. Independent SHA-256 comparisons found no mismatch. A `mountvol` removal attempt was denied, so a physical remove/reinsert event has not yet been certified.
- The repeated restart observations exposed a correctness defect: the webview's per-session attempt set was lost on restart, so the same mounted generation could auto-copy again. Schema version 11 now persists whether a run was automatic, and native profile lookup suppresses auto-start when a sealed automatic run already exists for the exact source identity and generation. A partial unique SQLite index also atomically permits only one queued/copying/recovery/completed automatic run per identity/generation, preventing two app sessions from racing the lookup. The freshly rebuilt package completed post-migration run `738d6152-6f13-4009-badc-aa2f280a4b7b` (3 files / 3,670,016 bytes, independent SHA-256 zero mismatch); restart left the ledger count at 5 and the selected card visibly stated it had already auto-ingested for this mount. Physical removal/reinsert is still unverified.
- The same freshly rebuilt package also completed a manual verified ingest from the selected M: card to fresh isolated `F:\media-ingest-manual-v11-cert-20260826`. Run `336875d5-6b91-4f7f-8717-3ea0ab7afee6` has `auto_ingest_triggered = 0`, a sealed three-file / 3,670,016-byte receipt, and independent SHA-256 zero mismatch. This closes the current packaged manual-ingest evidence gap; it does not exercise interruption, concurrent cards, or physical remount.
- A current-core opt-in two-card probe now runs the controlled full-size SD fixture `D:\DCIM\100DUMMY` and microSD fixture `M:\DCIM\100CERT` concurrently, each with two bounded workers and an isolated destination under `F:\media-ingest-hardware-tests`. On 2026-08-26 both completed with independent receipts; independent SHA-256 checks found 3/3 matching files and zero mismatches for each card. This demonstrates separate core copy/verification pipelines, not concurrent Tauri IPC, profile recall, cancellation/recovery, or a second reader.
- A fresh unsigned Windows package (`media-ingest-tool.exe` SHA-256 `C033C78526B65A504FFF4FA6CB0C3829CC763F85AD679C1F23198D43839F8A46`; NSIS SHA-256 `1999C736FD5160136A19E66170BC42C9D43035A45A101D3EF7FA1D2348AD1101`) completed concurrent manual desktop-IPC ingests from both calibrated cards into separate folders under `F:\media-ingest-packaged-concurrent-efb6cec8-0817-4a7f-aafb-6243fb07e2c1`. The microSD was visibly copying 972.6 MB of its 2.688 GB five-file plan when the full-size SD run was accepted; both later sealed receipts. Receipt `4f4ceb3b-5ed9-4e8d-ac79-43486619700a` covers microSD 5 files / 2,688,024,576 bytes and receipt `235d92c7-6f73-4bb0-936c-753b87a8f093` covers full-size SD 19 files / 6,465,617 bytes. A fresh independent SHA-256 re-read found 19/19 and 5/5 matching payload files respectively, with zero missing, extra, or mismatched files after excluding Windows `System Volume Information` metadata and the app marker. A stale pre-marker native inventory initially caused the second start to be rejected; the React client now refreshes native inventory after a successful ingest so the next card action has its current mutable key/generation. Rust's exact source revalidation remains unchanged. This is packaged two-card concurrency evidence, but interruption/recovery, physical remount, another reader, and non-Windows evidence remain open.

## Open questions

- Packaged lifecycle validation on 2026-08-26 deliberately cancelled an active 2.688 GB microSD verification; the UI correctly returned it to `recovery_required` with no receipt. Recovery now accepts that unchanged marker-backed generation while retaining the immutable-identity rule for a changed generation. The rebuilt-and-installed Windows NSIS package correctly suppressed a duplicate auto-ingest after restart and rendered “Registered card already auto-ingested for this mount”; resuming the original run sealed a five-file / 2,688,024,576-byte receipt with matching persisted BLAKE3 digests. Physical removal/reinsert, crash/sleep, eject, destination-failure, second-reader, macOS, and Linux evidence remain open.

- The current installed Windows package also exercised the durable partial checkpoint: cancellation during the 2.688 GB microSD copy left run `caf94fcc-89c9-453c-afcc-3201c5022a0d` in `recovery_required` and retained its entry-scoped `.partial`. Explicit recovery removed that checkpoint, recopied and verified the frozen plan, then sealed a five-file / 2,688,024,576-byte receipt. An independent SHA-256 comparison found 5/5 matches and no partial remained. This proves the package path for cooperative cancellation, not physical removal or crash/power loss.

- The previous Windows safe-eject provider only issued `IOCTL_STORAGE_EJECT_MEDIA` to a volume, which cannot safely remove a USB card reader. It now flushes, locks, and dismounts the resolved volume, maps that volume to its exact disk device number and PnP devnode, closes its own lock handle, then requests `CM_Request_Device_EjectW` for that disk only. It never walks up to a shared USB reader/hub, and explicit PnP veto/non-ejectable errors leave the card mounted. A first package attempt exposed a self-veto: the prior implementation asked PnP to eject while holding that handle and reported the card busy. The handle-order fix is source-verified but awaits a rebuilt package rerun. Safe eject accepts a matching current marker-backed generation because it is non-destructive; formatting remains immutable-identity-only. Sealed history restores the eject control after a restart.

- Final application name and reverse-domain bundle identifier.
- Exact Windows/macOS/Linux minimum versions and CPU architectures to certify.
- Which SanDisk model(s) are physically available: SDPR5A8, SDPR3A8, SDDR-A451, PRO-DOCK configurations, or another variant.
- Which camera makes/models and media formats are available for metadata and “camera-ready” format certification.
- Which sacrificial cards/readers and destination media are available for destructive and performance tests.

## Next updates

- The React operator surface now uses a Tailwind-first minimalist workspace with light and dark blue modes, a persistent-in-session mode toggle, restrained source rail, open workflow panels, practical typography, and responsive layout. Auto-ingest configuration is now intentionally modal: the source plan has a dedicated setup button, while the dialog owns destination, opt-in, conditional auto-format, safety disclosure, and registration action. Browser fixture screenshots cover light desktop plus dark desktop/mobile and the setup dialog; native runtime validation remains pending.
- Destination controls now prefer Tauri's native directory dialog in both the main plan and auto-ingest setup modal. The dialog plugin is registered as an explicit minimal capability; manual path typing remains available as a desktop-failure and browser-fixture fallback. Rust compile and browser fixture fallback are verified, but a native OS dialog selection has not been exercised in a packaged/Tauri runtime.
- Each newly verified writable input card now receives a compact `MIT2` record: a random managed-card token and the path-free BLAKE3 manifest root of that completed ingest (at most 128 bytes). Legacy `MIT1` records remain readable. This is useful continuity and accidental-swap evidence, but it is copyable/mutable filesystem state and cannot make the unresolved PRO-READER cards format-eligible.

- Add checkpoint/recovery and startup reconciliation before treating interrupted ingest runs as resumable.
- Capture the controlled microSD insertion and repeat the LUN mapping on a second reader/other supported OSes before treating it as a product-wide mapping.
- Update certification rows only as fixture, native, packaged, and hardware evidence is produced.

See [Progress](progress.md) and [Tasks](tasks/_index.md).

# Current update — 2026-08-27 managed-card auto-format repair

The native execution command was registered in the runtime handler but omitted
from the Tauri build manifest and window capability. It is now generated and
granted as `allow-execute-format-authorization`; the repaired dev session
builds and starts. The current dev UI is still displaying its preview fixture
instead of M:'s native inventory, so it cannot yet provide destructive
button-path evidence. M: remains untouched, exFAT, Healthy/OK, and retains
its controlled source files. This is an open live-runtime blocker, not a
format certification result.

The Tauri runtime probe was also corrected to use the public Tauri 2
`isTauri()` API; the dev window now renders the actual D: and M: inventory,
not browser fixtures. The live WMI input probe for M: passes after omitting
Windows-rejected optional empty-label/automatic-allocation inputs and requiring
`RunAsJob=false`. Windows still reports completion before the old mount has
transitioned, so remount validation now requires observing the old mount go
away before it can accept the replacement. Physical format and auto-format
certification remain pending this stricter live retry.

The Windows packaged physical test copied and independently preserved the
five-file / 2,688,024,576-byte microSD source into
`C:\Users\Workstation\Documents\MediaIngestManagedFormatCert20260827` and
sealed receipts, but did not format the card. Investigation found two native
gates correctly skip rather than erase: an old zero-byte reserved marker could
not be upgraded, and creating a marker during ingest changed the mutable
marker-derived operation key after the receipt was sealed. The repair writes
marker replacements through a synced sibling plus Windows replace operation,
repairs only an empty interrupted reserved marker, and keeps mutable marker
evidence out of the native/session operation key. A new managed-card witness
is MIT2 token plus a path-free BLAKE3 digest (109 bytes on the live card);
the marker selects an opt-in profile but never becomes hardware identity.

Focused marker, operation-key, and auto-format guards plus `cargo check` pass.
The rebuilt NSIS package SHA-256 is
`19AF80DD2CBB183CC9B30ED33830865443DAAF0EA2DB58FBB2A9A6EF2116FE7D`.
It is installed and awaiting one newly confirmed physical M: remount for the
destructive auto-format proof. Until that run records a format receipt and a
fresh writable remount, managed auto-format remains partial—not certified.

# Current update — 2026-08-27 exFAT repair and provider diagnostics

The live sacrificial M: microSD was verified as the removable 511.8 GB SanDisk
PRO-READER volume before repair. Windows reported `Full Repair Needed`;
`Repair-Volume -Scan` does not support exFAT, so `chkdsk M: /f` was run after
closing the app's own handle. It repaired a corrupt allocation bitmap and one
cross-linked test MP4 allocation. Fresh inspection reports exFAT, `Healthy`,
and `OK`; no NTFS conversion or repartition occurred.

The provider now distinguishes rejected WMI input, missing WMI output, and an
unreadable WMI return value. Failed marker sibling writes now remove their
temporary file. Package
`E96A81C4AF07023B32AABDE6D136296389956D612FFE0624BA19644616CD2996` is ready
for a fresh confirmed physical M: remount; destructive format proof remains
pending.

# Current update — 2026-08-27 operator visual refinement

The React operator fixture now uses a more deliberate editorial-utility visual
language: a subtle measurement-grid backdrop, translucent sticky command bar,
quiet source rail, stronger device hierarchy, tabular operational values, and
native serif headings. It also supplies visible focus treatment, a keyboard
skip link, dark-mode color-scheme support, and reduced-motion handling. Light
and dark browser-fixture screenshots were captured; no native ingest contract
or packaged-desktop evidence changed.

The follow-up accessibility pass gives previously link-like secondary actions
clear bordered button boundaries, removes the inactive Settings control, and
uses Title Case for actions and headings. Destination fields have names,
autocomplete protection, and spellcheck disabled; the workspace has a skip
link, clear focus state, modal overscroll containment, and reduced motion.
TypeScript, 12 UI fixtures, build, and desktop/mobile browser checks pass.
This is WCAG AA-oriented source and browser-fixture evidence, not a formal
accessibility conformance certification or packaged desktop proof.

The first secondary-action treatment left those buttons at link-sized padding,
which made the header actions look cramped and created uneven whitespace.
They now have a consistent 40 px control height, 12 px horizontal padding,
rounded visible boundaries, and a restrained sans heading system. Browser QA
measured the repaired `Change` action at 80 by 40 px and captured the corrected
desktop layout; the browser console is clean.

# Current update — 2026-08-26

Windows now combines the existing per-LUN disk-interface subscription with a one-second snapshot reconciliation and `IOCTL_STORAGE_CHECK_VERIFY` on each mounted volume. This detects an SD or microSD medium being removed from the shared SanDisk PRO-READER even if the USB reader itself never emits a PnP removal event. The reader parent is not ejected or treated as absent. The installed NSIS package `738E5C98644E3FAC72995B275CDA874F71D86BD4EB8866F12431602D607106E2` rendered the two live calibrated slots at startup; physical pull/reinsert proof is still required.

The operator UI now exposes the native per-medium connection generation as `Insertion N` in Hardware Evidence. The rebuilt installed NSIS package `7C8B9FA9ED1F6CDFFFC2C1526BFFCF9FE2714935AEA4EFEAB1F74381B560797E` rendered `Insertion 1` for the selected SD card. A physical removal/reinsertion must advance that displayed value before this lifecycle can be certified.

The running package currently renders the microSD as `Insertion 2` and says its registered card was already auto-ingested. Because generations are seeded from the durable receipt ledger on launch, this alone is not proof of the most recent physical cycle. Certification requires a captured one-card absent state followed by a returned microSD with a higher insertion value; source-removal-during-copy, destination loss, other readers, and other platforms remain open.

Physical lifecycle evidence is now complete for the calibrated microSD LUN on this reader: while the app remained running, inventory changed from two cards to one card after removal, then the returned microSD rendered `Insertion 3`. Its registered auto-ingest created completed receipt `ca981d73-a337-49da-bf23-134bd93dca3d` for generation 3 (five files / 2,688,024,576 bytes); independent SHA-256 checks matched all five source/destination pairs. After restart, the package rendered `Registered card already auto-ingested for this mount` and did not replay the generation-3 run. The original running UI failed to settle from its full-byte `copying` label despite the sealed receipt, so live terminal-state delivery remains an open defect; receipt integrity, restart recovery, and deduplication are evidenced.

The UI now defensively reconciles its active-operation display against native history every two seconds while an ingest is active. A completed history row with a sealed receipt clears the matching stale operation, restores its completed-run association, and displays verified completion without an app restart. Focused UI tests and TypeScript pass; this source-level repair still needs a packaged repetition of the terminal-state scenario.

The repaired package now has that physical evidence: a new microSD cycle created completed generation 4 run `4772aebe-e8a6-4c8b-8111-953818f80235`; while the same app session remained open, the UI reconciled its history and rendered `5 files verified · receipt sealed`, with no active ingest control and no restart.

2026-08-27 format diagnosis: the live native format confirmation was opened for
the 511.8 GB managed M: microSD. Directly exercising the identical
`MSFT_Volume.Format` request returned `43006` (read-only volume); the card's
label, marker, and source folders remained intact. Disk 5 reports online,
healthy, and not read-only, so this is a volume/provider restriction rather
than an elevation failure. The provider now maps 43006 to the readable
write-protected outcome, follows a returned `MSFT_StorageJob`, and requires the
old marker to disappear before accepting format success. Destructive completion
and auto-format certification remain blocked by the live Windows read-only
format response.

2026-08-27 compatible formatter proof: `Format-Volume -DriveLetter M
-FileSystem exFAT -Force -Confirm:$false` completed on that exact 511.8 GB
M: volume, leaving a healthy exFAT filesystem with only `System Volume
Information`; the old managed marker and media folders are gone. The provider
now keeps WMI only for exact-target binding/revalidation and runs that Windows
Storage cmdlet as a noninteractive fixed command with a WMI-sourced drive
letter and allowlisted filesystem. Its successful remount plus marker absence
is the format proof. This proves the native provider path; a fresh app-owned
manual run is still needed to prove marker restoration and format receipt, then
the matching auto-ingest run.

2026-08-27 automatic destructive proof: registered M: auto-ingest run
`217d7fdd-cf17-45dd-b587-d81507d2e605` verified the controlled source,
sealed its receipt, formatted the exact 511.8 GB card through the native
provider as exFAT, restored its managed marker, and persisted the matching
`sdxc-default` format receipt. A format can cause the active reader slot to be
observed as a new generation before marker restoration settles. The profile
lookup now retries an initially absent marker profile, and suppresses only an
empty card that has a completed format receipt; this prevents a self-triggered
second auto-ingest without blocking the next media-bearing mount. The manual
UI confirmation command path and non-Windows/camera evidence remain open.

2026-08-27 release-runtime repetition: the release x64 executable built for
NSIS bundle `AE3C98F30A5E1ED116496DC5DDD678E501EFAEC8F07F8370E3AA94840940A305`
started automatically on the still-mounted managed M: card with controlled
media and completed run `cf3048a9-a70f-4edc-9b2d-2afc3e175009`. Its local
ledger confirms the completed automatic run and `sdxc-default` marker-restored
format receipt. Native inspection afterwards reported M: `exFAT`, `Healthy`,
`OK`, exact 511,801,556,992-byte capacity, and only Windows metadata plus the
42-byte marker. This is release-binary evidence, not a clean-machine installer
or cross-platform certification.

2026-08-27 empty-remount hardening: `start_verified_ingest` now returns a
typed skipped automatic result before persisting a run when a mount-triggered
plan has zero media files. This is an independent native backstop for the
post-format generation race; it leaves no failed run, no receipt, and no format
attempt. Focused Rust coverage and the full frontend test/type checks pass.

2026-08-27 installed-package launch: the newly rebuilt NSIS installer
`D27925F12445C80ED9F0DD0FBE6443818C8A783338F7FD9F0C14F6F9F9B0EDA1` completed
silently with exit code 0. The installed executable at
`%LOCALAPPDATA%\\Media Ingest Tool\\media-ingest-tool.exe` was launched and
observed the retained completed auto-format receipt, managed marker, and
healthy exact-capacity M: volume. This verifies installation/launch on the
current machine, not a clean-machine install.

2026-08-27 operator UI clarity: connected-media cards now expose each mounted
drive letter and filesystem alongside capacity, free space, and reader slot;
the selected-source header repeats the observed drive detail. Ingest-history
states are title-cased for scanability. Drive letters remain mutable mount
details, never source identity or authorization evidence.

2026-08-27 light-mode contrast repair: all secondary/eyebrow text and input
placeholders now use #475569 rather than faint slate-400/500, amber notices use
#92400e, and disabled controls use opaque #475569 text on #e2e8f0 instead of
low-opacity text. Browser fixture measurement confirms the changed light-mode
pairs at 6.15:1–7.58:1. This is focused rendered evidence, not formal full-app
WCAG conformance certification.

2026-08-27 icon-system refinement: the operator shell now uses `lucide-react`
icons for the ingest mark, media cards, rescan action, verification state,
warning, and modal close control, replacing textual/glyph substitutes. Rendered
fixture QA confirms five semantic SVG icons, no legacy glyphs, no horizontal
overflow, and a clean browser console.

The packaged application icon now uses the same blue hard-drive/download mark.
`tauri icon` regenerated the Windows `.ico`, macOS `.icns`, PNG, AppX, iOS,
and Android variants from `src-tauri/icons/media-ingest.svg`; a new x64 NSIS
installer was built with SHA-256
`EC2477120B23E37E2490A0A6DA79079F86402D76EF407239F16AC386738D4884`.

2026-08-28 organization controls: capture-time sorting is now exposed as
imageboard-like selectable tags for EXIF day, EXIF hour, an operator-entered
1–1,440 minute interval, or the original tree. The UI displays the exact
destination directory depth and components before planning. The planner now
accepts arbitrary bounded minute intervals (for example, 30 or 37 minutes),
anchored at local capture-day midnight; metadata remains EXIF-first with the
existing explicit filesystem-time fallback when no usable embedded timestamp
is available. Browser fixture evidence and Rust unit tests cover this; a
native metadata-preview run on real media remains unverified.

2026-08-28 macOS source support: mounted external-volume discovery now uses
the absolute `/usr/sbin/diskutil` binary and structured XML property-list
responses. It rejects internal/read-only/non-mounted entries, captures
filesystem UUID/capacity and non-authoritative reader presentation evidence,
and feeds the existing one-second snapshot reconciliation worker. Safe eject
now permits only a canonical `/Volumes/...` mount that was revalidated by the
Rust command boundary before it invokes the same absolute binary.

This is partial macOS support, not full platform certification. Apple's Disk
Arbitration documentation requires event reconciliation because callback order
may vary, and safe formatting still requires Disk Arbitration target binding
plus a signed least-privilege helper/SMAppService. No macOS host, SDK, card,
reader, authorization, signed package, or notarization credential is available
here. Cross-compilation to `aarch64-apple-darwin` installed the Rust target but
correctly stopped when `objc2-exception-helper` required an Apple-compatible C
compiler/SDK that this Windows host does not have.

2026-08-28 macOS packaging gate: `tauri.macos.conf.json` declares the macOS
13 minimum required for the planned `SMAppService` helper model and enables
the hardened runtime. CI now has a macOS-host job that runs the existing
frontend/Rust matrix and produces an unsigned `.app`/DMG artifact. This gate
does not substitute for Developer ID signing, notarization, clean-Mac install,
or hardware workflow evidence.

2026-08-28 macOS format correction: the source-level direct `diskutil
eraseVolume` provider has been removed and replaced with an explicit
`UnsupportedPlatform` authorized-helper boundary. The former command could
successfully destructively erase a volume, then wait for the old mount path
after the new `MEDIA_INGEST` label caused a different remount path, leaving no
safe marker/receipt completion. The replacement makes no destructive call until
a signed helper proves a current whole-medium Disk Arbitration/IOMedia binding,
performs privileged work after one opaque authorization, and returns the new
validated mount. This restores fail-closed behavior but does not constitute
macOS quick-format support.

2026-08-27 typography hierarchy repair: a global `font: inherit` rule had
silently reset all button/input/select size utilities to 16 px, making controls
compete with section titles. It now inherits only the font family. The rendered
operator scale is selected-device title 36 px, section title 18 px, status
value 16 px, label 11 px, secondary action 12 px, and primary action 14 px;
the browser fixture has no horizontal overflow or console errors.

2026-08-27 header simplification: the app mark is now followed only by the
single product name, `Media Ingest Tool`; the redundant local-workflow label
and `Ingest Station` title were removed. Browser fixture checks confirm the
new heading is present, the old heading is absent, and the console is clean.

2026-08-27 divider cleanup: the selected-device header no longer draws its own
bottom border; the status strip supplies the sole divider. Browser inspection
confirms a 0 px header bottom border and a single 1 px status-strip top border.

2026-08-27 aggregate ingest dock: active native operations now retain their
stage, byte totals, file position, measured rate, and a bounded stage/file log
in the webview. A fixed bottom dock aggregates known bytes into total percent
and ETA; it expands to per-operation progress and logs. The shell reserves
6.5rem while collapsed and up to 48vh while expanded so scrollable content is
never hidden behind the dock. TypeScript, 13 UI tests, and the production build
pass; a live native multi-operation run is still needed for populated dock QA.

2026-08-27 dock refinement: Live View is intentionally present only while one
or more native ingests are active or queued, avoiding idle workspace clutter.
It is compact when collapsed and, on desktop, centered within the main
workspace to the right of the 19rem source rail rather than spanning it.

2026-08-27 destination cleanup: removed the non-functional `Change` control
from Destination & Organization. The named destination field and native folder
chooser remain the only actions for changing the ingest destination.

2026-08-27 destination affordance: the editable destination field now uses a
contrasting active surface and stronger boundary in dark mode, rather than the
same muted panel surface that made it appear disabled.

2026-08-27 destination-memory alignment: destination recall and save controls
now disable whenever native discovery cannot provide stable hardware card
identity. Their tooltip and visible session-only status explain that this is a
safety boundary, not an unavailable destination-field editor.

2026-08-27 installer build: the current workspace produced unsigned x64
Windows installers: NSIS `Media Ingest Tool_0.1.0_x64-setup.exe` (3,216,487
bytes, SHA-256 `D7F4DEC2D55F3BFF7EBFD1433FBD64402B0A58BB352FFBADD7FA28D999734CF1`)
and MSI `Media Ingest Tool_0.1.0_x64_en-US.msi` (4,591,616 bytes, SHA-256
`B53CC017A0879AA5838479DD262DDA889FEA2CDC5B283BE8C121D1575738E8E6`).

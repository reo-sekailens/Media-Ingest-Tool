# macOS filesystem and webview review

Research date: 2026-10-13. This review covers Intel and Apple Silicon because
the audited behavior is shared Rust/Tauri code; it does not establish that a
package runs on either architecture. The baseline was commit
`7e6df7db86f9fbffd1a678392f11cf8f9c5e39bc` plus the existing working changes.
Other changes made during this task must be checked against the acceptance
items below before treating a finding as resolved.

## Integration resolution

The integration implements MAC-R01 (minimum 13.3), MAC-R02 (exclusive macOS
publication), MAC-R03 (shared exact root metadata exclusions), MAC-R04 (nearest
existing ancestor with traversal/overlap rejection), MAC-R06 (macOS fullfsync
plus file-backed WAL/FULL readback), and MAC-R07 (five purpose strings and
permission guidance). Their Mac runtime/package checks are pending the new
exact-source hosted run. MAC-R05 directory-entry durability and MAC-R08 physical
sleep/wake remain open. The finding table below describes the initial baseline,
not the current fixed source. See [current validation](macos-compatibility.md).

The native follow-up on commit `efa8cfa5f4e09c2e51bac873756f56c31be55519`
isolated the ExFAT failure: `File::sync_all` succeeded, while the exclusive
rename returned Darwin ENOTSUP (45). Both primitives succeeded on APFS and
case-sensitive APFS. The integration maps that exact publication failure to
an actionable unsupported-destination response. It retains the mandatory
full flush and atomic refusal to replace existing files. ExFAT is supported
as a source; Mac destinations use APFS/APFSX. ExFAT-destination certification
remains blocked. This finding does not establish physical drive-cache safety.

## Audited implementation

The review read `memory-bank/projectbrief.md`, `activeContext.md`, the task
index and TASK004/TASK010/TASK011/TASK012, then inspected the working diff and
these runtime paths:

- `src-tauri/src/ingest.rs`: inventory, root validation, copy/readback,
  publication, cancellation, recovery and receipt writes.
- `src-tauri/src/organization.rs` and `metadata.rs`: portable names, collision
  projection, recorded capture offsets and computer-local organization.
- `src-tauri/src/local_store.rs` and `storage_marker.rs`: SQLite migrations,
  completion sealing, recovery and card continuity records.
- `src-tauri/src/lib.rs`: native command boundaries, device revalidation,
  bounded workers, app data/log paths, close and focus handling.
- `src/App.tsx`, `Modal.tsx`, `CardConfiguration.tsx`, their styles,
  `vite.config.ts`, Tauri configuration and the capability file.

The lockfile at review time pins Tauri 2.11.5, wry 0.55.1, dialog 2.7.2,
log 2.9.0, rfd 0.16.0, chrono 0.4.45, nom-exif 3.6.2, rusqlite 0.37.0,
libsqlite3-sys 0.35.0, unicode-normalization 0.1.25 and unicode-casefold 0.2.0.
Rust policy is 1.98.0 and the frontend uses Tailwind 4.3.3. Source/configuration
and the final lockfiles remain authoritative if these versions change.

## Findings and acceptance items

| ID      | Baseline observation                                                                                                                                                                                                                                                               | Required acceptance evidence                                                                                                                                                                                                                                                                                                                               |
| ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MAC-R01 | The configured macOS minimum is 13.0, while Tailwind v4 requires Safari 16.4. Tauri uses the system WKWebView, and its version table maps Ventura 13.3 to Safari 16.4 and Ventura 13.0 to 16.1.                                                                                    | Raise the bundle/documented minimum to at least macOS 13.3, or provide an independently verified compatibility implementation. Inspect the produced bundle's `LSMinimumSystemVersion`; a Vite JavaScript target alone does not backport CSS.                                                                                                               |
| MAC-R02 | `verified_copy_to_with_progress` and `write_receipt` check destination existence, then call replacing `std::fs::rename`. An external writer can occupy the name between those operations.                                                                                          | Use an OS-atomic no-replace publication operation. On macOS, `renamex_np`/`renameatx_np` with `RENAME_EXCL` is supported only where the volume advertises the capability. Unsupported operations must fail safely, preserving the verified partial. Test a competing existing destination, a broken symlink and case/Unicode-equivalent destination names. |
| MAC-R03 | Root inventory excludes Windows `System Volume Information` and the app marker, but visits macOS volume service data. Apple's documented `.fseventsd` is a per-volume event log. Other known system roots such as `.Trashes` and `.Spotlight-V100` also require a policy decision. | Apply a narrow root-only system-metadata exclusion consistently to inventory, copy planning and the content witness. Keep ordinary dotfiles and identically named nested directories. Test unreadable excluded directories and ordinary unreadable media separately. No affected physical Mac/card has been observed here.                                 |
| MAC-R04 | A destination may be typed as multiple missing directory levels, but `validate_ingest_roots` canonicalizes only its immediate parent. Space and lease preflight already find the nearest existing ancestor.                                                                        | Resolve the existing ancestor plus the missing suffix coherently before creating a plan. Keep source/destination overlap rejection, portable-component validation and permission/I/O failure behavior. Test a nested new destination and a nested path pointing back into the source.                                                                      |
| MAC-R05 | File content is synced, but media publication, receipt publication and marker replacement do not explicitly persist parent-directory entries. The manifest does not encode a distinct directory-durability result.                                                                 | Preserve the existing byte-verification claim, document this limit, and keep macOS destructive formatting unavailable. A full durability policy needs tested filesystem-specific directory sync and honest unsupported/error outcomes before any stronger format-safe claim.                                                                               |
| MAC-R06 | SQLite requests WAL and `synchronous=FULL` but does not read those values back. It does not enable macOS `fullfsync`.                                                                                                                                                              | Verify effective startup pragmas. Evaluate macOS `fullfsync=ON` for the completion authority; it is a separate setting from synchronous FULL. Record performance and error behavior rather than assuming the settings are interchangeable.                                                                                                                 |
| MAC-R07 | Automatic native access can trigger removable-volume privacy consent; baseline macOS configuration contains no purpose strings. Native I/O failures are generally rendered as protected entries or generic I/O failure.                                                            | Add accurate optional privacy purpose strings to the merged Info.plist for removable volumes and applicable destination folders/network volumes. Document Files & Folders consent recovery and exercise both Allow and Deny in an installed package. The strings improve the prompt; they do not grant access.                                             |
| MAC-R08 | Focus restoration requests a new snapshot/history. No macOS power notification adapter pauses or revalidates active work at sleep/wake. The Disk Arbitration worker has a 30-second fallback snapshot.                                                                             | Retain this as an explicit runtime-certification gap. Test sleeping during copy/readback, wake without a focus transition, source/destination removal during sleep and mount reuse. A future adapter can observe NSWorkspace sleep/wake notifications and revalidate the same native source generation before continuing.                                  |

Sources supporting MAC-R01:
[Tailwind compatibility](https://tailwindcss.com/docs/compatibility),
[Tauri webview versions](https://v2.tauri.app/reference/webview-versions/), and
[Apple Safari 16.4 release notes](https://developer.apple.com/documentation/safari-release-notes/safari-16_4-release-notes).

Sources supporting MAC-R02:
[Rust rename API](https://doc.rust-lang.org/std/fs/fn.rename.html) and
[Apple's current XNU rename manual](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/rename.2).
Do not replace this with a hard-link-only technique: exFAT does not support
hard links in Microsoft's filesystem feature contract.
[Microsoft filesystem comparison](https://learn.microsoft.com/en-us/windows/win32/fileio/filesystem-functionality-comparison).

Source supporting MAC-R03:
[Apple FSEvents volume-log documentation](https://developer.apple.com/library/archive/documentation/Darwin/Conceptual/FSEvents_ProgGuide/FileSystemEventSecurity/FileSystemEventSecurity.html).
The additional metadata-directory policy is an implementation recommendation;
its permission behavior still needs macOS fixtures/physical evidence.

Sources supporting MAC-R05/MAC-R06:
[Rust File sync API](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all),
[pinned Rust 1.98 Apple sync implementation](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/fs/unix.rs),
[Apple's current XNU fcntl manual](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/fcntl.2),
[SQLite synchronous](https://sqlite.org/pragma.html#pragma_synchronous),
[SQLite fullfsync](https://sqlite.org/pragma.html#pragma_fullfsync), and
[SQLite checkpoint_fullfsync](https://sqlite.org/pragma.html#pragma_checkpoint_fullfsync).

Sources supporting MAC-R07/MAC-R08:
[Apple removable-volume purpose string](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription),
[Apple Documents purpose string](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdocumentsfolderusagedescription),
[Apple wake notification](https://developer.apple.com/documentation/appkit/nsworkspace/didwakenotification), and
[Apple sleep notification](https://developer.apple.com/documentation/appkit/nsworkspace/willsleepnotification).

## Filesystems and path semantics

APFS supports both case-sensitive and case-insensitive variants. The current
planner computes a conservative NFC plus full Unicode case-fold collision key
for every planned destination, then adds a deterministic source-path suffix
when two files collide. Native `Path` components remain the filesystem path
representation. This is appropriate conservative planning; it is not an exact
implementation of every volume's collation. APFS normalization behavior and
exFAT's volume up-case table can differ from the crate's Unicode tables.
The OS no-replace operation must arbitrate existing names at publication.
[Apple files and directories](https://developer.apple.com/documentation/technologyoverviews/files-and-directories),
[Apple APFS filename FAQ](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/APFS_Guide/FAQ/FAQ.html),
[unicode-casefold 0.2 API](https://docs.rs/unicode-casefold/0.2.0/unicode_casefold/trait.UnicodeCaseFold.html), and
[Microsoft exFAT specification](https://learn.microsoft.com/en-us/windows/win32/fileio/exfat-specification).

The 120-byte component limit, reserved-character replacement and extension
preservation are conservative. A full path can still exceed the host or
filesystem limit, and unavailable space, quotas or permissions can still fail
after preflight. APFS volumes share container space. The native streaming
write/error path therefore remains authoritative; preflight cannot reserve
capacity. APFS, APFS Case-sensitive and exFAT each need real mounted-volume
acceptance checks on both Mac package architectures.
[Apple Disk Utility formats](https://support.apple.com/guide/disk-utility/file-system-formats-dsku19ed921c/mac).

The source walker rejects symlinks and different Unix `st_dev` values, while
the copy primitive repeats source type/length/mtime checks. This protects the
ordinary static media tree, not all concurrent pathname replacement races.
The existing post-readback recovery re-stat checks length/type/mtime, not
handle/inode identity. A fuller race-resistant design would walk/open against
owned directory descriptors, reject symlink traversal at every component,
and compare opened-handle identities; that is separate security scope.

`to_string_lossy()` still appears in persisted/receipt source paths. If a
mounted filesystem exposes a non-UTF-8 Unix filename, lossless recovery is not
established. Keep this limitation visible rather than claiming arbitrary Unix
path preservation. No claim is made that normal APFS or camera exFAT filenames
trigger it.

## Sync, recovery and persistence

Pinned Rust 1.98.0 already implements Apple `sync_all()` and `sync_data()` using
`fcntl(F_FULLFSYNC)`, with errors returned. Adding a second manual full-sync
call around every file would duplicate this work. Apple's current source
documents the device-buffer flush and warns that some drives may ignore it.
The app correctly propagates file-sync failures rather than sealing those
files; a real macOS exFAT destination must demonstrate that its driver/bridge
supports the operation. No fallback should silently strengthen a failed sync
into success.
[Pinned Rust sync implementation](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/fs/unix.rs),
[Apple fcntl manual](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/man/man2/fcntl.2).

SQLite completion commits all verified file records and the receipt seal in
one transaction. Interrupted runs become `recovery_required` on startup;
recovery is explicit, uses persisted destination paths, rehashes existing
final files and recopies missing files. Partials never count as transferred.
These source properties are shared between the architectures; native crash,
sleep and unplug results remain a separate gate.

The database resolves through Tauri `app_data_dir()`, rather than the install
bundle or source card. With `com.mediaingest.tool`, the documented macOS path
is `~/Library/Application Support/com.mediaingest.tool/media-ingest.sqlite3`;
logs use `~/Library/Logs/com.mediaingest.tool/`. This supports a read-only
installed bundle and manual drag-copy installation. Preserve the identifier
and the database plus WAL/SHM files when migrating or collecting state.
SQLite WAL requires a local same-host database, not a network share.
[Tauri paths](https://v2.tauri.app/reference/javascript/api/namespacepath/),
[SQLite WAL](https://sqlite.org/wal.html).

## Native picker, permissions and webview boundary

The existing native picker uses dialog `open` with `directory: true` and
`multiple: false`. The Rust plugin is installed and the main-window capability
grants `dialog:allow-open`. rfd 0.16 lists macOS folder picking and starting
paths as supported; its dialogs rely on the native windowed NSApplication
environment supplied by Tauri. Browser mocks do not validate that panel.
[Tauri dialog plugin](https://v2.tauri.app/plugin/dialog/),
[rfd 0.16 macOS threading/support](https://docs.rs/rfd/0.16.0/rfd/).

Apple describes Open/Save panel selection as implied consent for selected
removable-volume files. Automatic card inventory has no such explicit panel
selection, so the OS may prompt. A previously allowed destination path and a
valid saved profile do not override subsequent permission denial. The app is
currently a desktop bundle without an App Sandbox entitlement; do not add
sandbox entitlements or security-scoped-bookmark assumptions solely because
the distribution is ad-hoc signed/manual.
[Apple removable-volume consent](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription).

The CSP's `ipc:` and `http://ipc.localhost` connect sources match Tauri's
documented cross-platform IPC example. It loads no remote scripts/fonts,
and Tauri's CSP asset nonce/hash injection has not been disabled. The
filesystem/native safety authority remains in Rust; the webview has typed
commands rather than a general filesystem or shell capability. No
macOS-specific CSP mismatch was found by static inspection.
[Tauri CSP guidance](https://v2.tauri.app/security/csp/),
[Tauri security configuration](https://v2.tauri.app/reference/config/#securityconfig).

WKWebView persistence is distinct from the Rust-owned SQLite ledger. Tauri
documents that `dataDirectory` is unsupported on macOS and its replacement
data-store identifier needs macOS 14+. The app does not configure either;
there is no reason to raise the minimum solely for that unused option.
[Tauri webview data configuration](https://v2.tauri.app/reference/config/#windowconfig).

## Capture time and metadata

The metadata adapter uses pure Rust nom-exif rather than an external FFmpeg or
Exif tool. Recorded `DateTimeOriginal` accepts only its own
`OffsetTimeOriginal`; unknown offsets retain the camera wall clock. Computer
local organization converts only known instants through `chrono::Local` at
the capture instant, including historical offsets. The filesystem fallback is
an explicit UTC rendering of the OS-reported modification time; it cannot
reconstruct camera timezone evidence absent from a FAT/exFAT timestamp.
Frozen recovery paths are not recalculated after a system timezone change.
[nom-exif 3.6.2](https://docs.rs/nom-exif/3.6.2/nom_exif/),
[chrono 0.4.45 Local](https://docs.rs/chrono/0.4.45/chrono/offset/struct.Local.html).

These properties are source-reviewed and have repository fixture coverage.
Camera RAW/video variants, exFAT timestamp driver interpretation and host
timezone rules still require representative Mac inputs. Tests must compare
recorded EXIF clocks with independently different source mtimes so a fallback
cannot accidentally satisfy capture-folder expectations.

## Verification boundary

The initial findings are a static/source and primary-documentation review.
The separate native follow-up exercised disposable APFS/APFSX/ExFAT images,
shipping DMG startup and instrumented WKWebView on hosted Intel/ARM Macs.
Native Open panels, Gatekeeper consent, physical devices and sleep/wake were
not exercised. This does not certify Apple Silicon model variants, physical card identity,
reader topology, destructive formatting, performance or power-loss recovery.

Documentation checks can reject contradicted compatibility claims and direct
targeted fixes. They cannot substitute for an Intel and an Apple Silicon
runner building the exact reviewed snapshot, or a clean manual installation
and representative removable-media workflow. Native CI and package inspection
evidence should be recorded separately in TASK010/TASK011 and the maintained
certification matrix.

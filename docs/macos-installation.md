# Install Media Ingest Tool on a Mac

Manual installation does not require an Apple Developer account. Packages use
ad-hoc signing, without Developer ID signing or Apple notarization. macOS can
require an explicit exception for the downloaded app. This follows
[Tauri's ad-hoc distribution guidance](https://v2.tauri.app/distribute/sign/macos/)
and [Apple's instructions for opening downloaded apps](https://support.apple.com/en-us/102445).

## Choose the package

The configured minimum is **macOS Ventura 13.3** on either architecture. This
aligns the system WKWebView with Tailwind 4's Safari 16.4 requirement; it is a
compatibility baseline, not certification of every macOS release or Mac model.
See [Tailwind browser support](https://tailwindcss.com/docs/compatibility) and
[Tauri's system WebKit table](https://v2.tauri.app/reference/webview-versions/).

In **Apple menu > About This Mac**, check the processor/chip:

| Mac                 | Download target            |
| ------------------- | -------------------------- |
| Intel processor     | `x86_64-apple-darwin` DMG  |
| Apple M-series chip | `aarch64-apple-darwin` DMG |

Use the matching native build. An Intel DMG is not a universal application;
the ARM64 build avoids needing Rosetta on Apple Silicon. End users do not need
Node, Rust, Xcode, command-line tools, an Apple account, or a signing certificate.

Download from the repository's [Releases](https://github.com/reo-sekailens/Media-Ingest-Tool/releases).
Nightlies are prereleases. Check the source commit and architecture in
`artifact-manifest.json` and compare the selected DMG's SHA-256 with `SHA256SUMS`:

```sh
shasum -a 256 "downloaded-package.dmg"
```

## Install and first launch

1. Open the matching DMG in Finder.
2. Drag **Media Ingest Tool.app** to **Applications** (or your user's Applications
   folder if you cannot write the system folder).
3. Eject the disk image and open the copied app from Applications. Running from
   the read-only image is not the installation workflow.
4. If macOS blocks this trusted, checksum-verified download because its developer
   cannot be verified, open **System Settings > Privacy & Security**, find the
   blocked app, and choose **Open Anyway**, then **Open** in the confirmation.
   Attempting to open the app first makes this per-app option available.

Apple documents this per-app exception. It can be unavailable on a managed Mac.
Do not disable Gatekeeper globally, remove quarantine recursively, or replace
the app's signature. A damaged/modified-app or malware warning needs investigation
and a fresh verified download; it is not the routine unidentified-developer case.
[Apple Gatekeeper guidance](https://support.apple.com/en-us/102445).

## Card and destination access

On an Apple Silicon **laptop**, unlock the Mac and approve a new USB/Thunderbolt
reader or SD accessory if prompted. A denied accessory is not exposed to the app.
[Apple accessory approval](https://support.apple.com/en-gb/102282).

Allow the app's removable-volume access prompt when reading your card. Select
your destination using the app's folder picker. Access to Desktop, Documents,
Downloads, removable storage or network volumes can require privacy permission.
If access was denied, review **System Settings > Privacy & Security > Files &
Folders**, enable the relevant access for Media Ingest Tool, then reopen the app
and refresh inventory. Purpose strings explain access; they do not grant it.
[Apple file-access controls](https://support.apple.com/guide/mac-help/control-access-to-files-and-folders-on-mac-mchld5a35146/mac),
[removable-volume consent](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription).

The app copies files from **mounted storage volumes**. A camera connected only
through PTP/MTP is not a mounted card and is outside this discovery provider.
Reader/card immutable identity depends on what macOS and the reader expose;
the UI reports confidence rather than assuming a USB reader serial identifies
the card. Unknown or changing mounts remain subject to native safety checks.

Use a writable local **APFS or case-sensitive APFS destination** and preserve
the source card until the full copy/readback succeeds and a receipt is available.
ExFAT cards can be sources. ExFAT destinations do not provide the atomic
no-overwrite rename required by this app on the tested Mac systems; the app
returns an unsupported-publication error and preserves the source and verified
partial instead of completing the ingest. APFS and exFAT tests on
disposable runner disk images do not certify physical media, power-loss safety,
NAS/SMB servers, all readers, or all camera file formats.

**Quick Format is unavailable on macOS.** The provider rejects it because an
authorized native helper has not been implemented. Use the camera's own format
workflow after independently confirming your backup; the app does not request
an administrator password or silently invoke elevated formatting commands.

## Update, uninstall and diagnostics

Quit the app after active operations finish, then replace its Applications copy
with the matching new version. Keep the downloaded prior version and its
checksums if you need to roll back the binary. Local database migrations can make
older binaries incompatible with newer state; replacing the app is not a database
rollback. Do not edit the bundle after signing.

Moving the app to Trash preserves local profiles, history and logs. User data is
under `~/Library/Application Support/com.mediaingest.tool/`; logs are under
`~/Library/Logs/com.mediaingest.tool/`. Removing those folders is a separate user
decision and removes local continuity/history evidence. Keep them for diagnosis
and backup before removing anything.
[Tauri data paths](https://v2.tauri.app/reference/javascript/api/namespacepath/).

For a problem, quit the app after reproduction and read the log instructions in
[the development guide](development.md#macos-support-logs). Record macOS version,
Intel/M-series architecture, package/source SHA, reader/card/filesystem, and the
exact visible error. Permission dialogs, physical card I/O, sleep/wake and actual
safe eject still require installed-device acceptance evidence.

The implementation audit and available validation are linked in
[the compatibility report](macos-compatibility.md).

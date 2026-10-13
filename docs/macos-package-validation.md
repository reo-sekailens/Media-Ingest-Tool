# macOS manual-install package validation

The macOS artifacts are separate native Intel (`x86_64`) and Apple Silicon
(`arm64`) DMGs. They use ad-hoc signatures and hardened runtime, with no Apple
Developer account, Developer ID certificate, provisioning profile, or
notarization credentials. This is a direct-download distribution configuration.
An ad-hoc signature verifies bundle integrity; it does not identify a trusted
developer or make Gatekeeper accept an Internet download automatically.

The declared minimum is read from `src-tauri/tauri.macos.conf.json`. It is
macOS 13.3 because the Tailwind 4 frontend requires Safari 16.4-era web platform
features. A minimum-version declaration and Mach-O check do not establish
runtime support on that exact OS version.

## Configured automated gates

Quality runs frontend checks, strict native Rust checks/tests, native WKWebView
smoke tests, and app/DMG validation on macOS 15 and macOS 26, on both Intel and
Apple Silicon runners. Its manual dispatch can validate an isolated branch or
immutable commit without a release or pull request. `check-only` skips the
distribution package builds while retaining native UI checks.
The release/nightly package matrix and manually dispatched macOS installer
workflow build both native architectures on macOS 15 and apply the same DMG
gate before uploading distribution artifacts. This avoids Rosetta being mistaken
for native Intel or Apple Silicon evidence.

`scripts/verify-macos-bundle.mjs` requires a Mac of the package architecture and
a fresh disposable user profile. It:

1. Requires one DMG, hashes it, verifies its disk-image checksum, and mounts it
   read-only at a task-local temporary directory.
2. Verifies the mounted app's signature, copies it with `ditto` into a temporary
   `Applications` directory, verifies the copied signature, and detaches the DMG.
   Startup therefore cannot rely on the image remaining mounted.
3. Checks the generated Info.plist identifier, application version/type,
   configured minimum OS, executable, icon, and the five privacy purpose
   descriptions for removable/network volumes and Desktop/Documents/Downloads.
4. Inspects every bundled Mach-O file using `file`, `lipo`, and `otool`. The
   main executable must contain exactly the expected native architecture; nested
   code must include it. Deployment load commands must target macOS and require
   no newer OS than the configured minimum. Dynamic dependencies must be Apple
   system libraries or resolve inside the app bundle, excluding dependencies on
   Homebrew, developer working directories, or other machine-local libraries.
5. Requires an ad-hoc signature and the configured hardened-runtime flag. The
   integrity check uses `codesign --verify --deep --strict`; it never re-signs
   or alters the distributed bundle to repair a failure.
6. Starts the copied app through LaunchServices with `open -n -W`, without a
   shell-profile PATH, resolves its exact installed executable process, and
   requires newly emitted application-start and SQLite-ready log markers. The
   process must survive another ten seconds. No card is supplied and no ingest,
   eject, or format command is invoked.

The JSON evidence records the source commit, DMG SHA-256, runner OS, target,
actual architectures/deployment versions/dependencies, and completed startup
checks. CI uploads available evidence even when the gate fails. Created app data/logs stay
in the disposable runner profile; the validator removes only its own temporary
mount/copy directory. It refuses to reconcile an existing operator SQLite store.

Run on a fresh test Mac account after building the native DMG:

```sh
node scripts/verify-macos-bundle.mjs \
  src-tauri/target/aarch64-apple-darwin/release/bundle \
  aarch64-apple-darwin \
  macos-validation/report.json
```

Use `x86_64-apple-darwin` for both path and target on Intel. Reports are native
runner evidence only after the workflow actually passes. Local Node parser and
failure tests on Windows do not count as Mac installation results.

## Installing a downloaded DMG

The four native production gates passed for source
`9e4c4fadc1c615677fd61c0cf6c68c78fea30c85` in
[run 38349630929](https://github.com/reo-sekailens/Media-Ingest-Tool/actions/runs/38349630929).
Actual hosts were Intel/ARM macOS 15.7.9, Intel 26.6.1 and ARM 26.6.2.
Downloaded bytes were independently checked against each native report,
manifest and SHA256SUMS. The overall run had a separate ARM26 test-driver
failure; no production DMG gate failed. The corrected native UI harness has
separate passing evidence in the [compatibility record](macos-compatibility.md).

Choose the artifact matching **Apple menu → About This Mac**: **Chip** indicates
Apple Silicon; **Processor: Intel** indicates Intel. Verify the DMG against the
release's SHA256SUMS, open the image, copy **Media Ingest Tool.app** into
**Applications**, and eject the image before launching the copied app.

For a trusted download blocked because Apple cannot verify the developer, try
opening the copied app, then use **System Settings → Privacy & Security →
Open Anyway**, and confirm **Open**. Apple documents this per-app exception;
managed-Mac policy can prevent it. A damaged/modified bundle, malware warning,
or broken signature is a failed validation, not an instruction to bypass checks.
Keep the built signature intact and use the per-app exception. Do not disable
Gatekeeper globally or re-sign the installed app as an installation step.

macOS may separately request access to removable/network volumes or protected
destination folders. Grant access when that location is part of the intended
ingest. A refusal must remain a visible discovery/copy error rather than being
reported as success. The folder chooser supplies operator intent; this direct
distribution build is not an App Sandbox application.

## Limits and remaining acceptance

These package gates do not reproduce a browser's quarantine metadata,
Gatekeeper's first-download dialogs, a clean physical Mac installation, every
M-series generation, or macOS 13.3 runtime behavior. Startup markers and process
survival do not prove WebKit rendering, IPC, or operator workflows. Separate
native UI evidence is described in [macOS UI validation](macos-ui-validation.md).

Real reader/card discovery, permissions granted/refused under Finder launch,
copy/verification/receipts on actual source and destination filesystems,
hotplug, disconnect/replug, cancellation, recovery, sleep/wake, and eject still
need hardware acceptance. macOS quick format remains fail-closed until its
authorized-helper and hardware safety requirements are implemented and tested.
Manual installation and research do not waive that boundary.

## Primary documentation checked

Reviewed 2026-10-13; project behavior is governed by the installed versions and
actual build output.

- [Tauri macOS signing and ad-hoc signatures](https://v2.tauri.app/distribute/sign/macos/)
  documents `signingIdentity: "-"` and the remaining Privacy & Security exception.
- [Tauri macOS application bundle](https://v2.tauri.app/distribute/macos-application-bundle/)
  documents bundle layout, platform-specific Info.plist, minimum system version,
  and the absence of shell-profile PATH for GUI launches.
- [Tauri DMG distribution](https://v2.tauri.app/distribute/dmg/) documents the
  disk-image container and Applications-copy installation.
- [GitHub runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
  identifies `macos-15-intel`/`macos-26-intel` as Intel and
  `macos-15`/`macos-26` as ARM64; these are runner OS versions.
- [Apple safe application opening](https://support.apple.com/en-us/102445)
  documents the current per-app **Open Anyway** flow after attempting launch.
- [Apple code signing in depth](https://developer.apple.com/library/archive/technotes/tn2206/)
  documents strict recursive signature verification and distinguishes integrity
  verification from Gatekeeper policy assessment.
- [Apple removable-volume privacy key](https://developer.apple.com/documentation/bundleresources/information-property-list/nsremovablevolumesusagedescription),
  [network-volume privacy key](https://developer.apple.com/documentation/bundleresources/information-property-list/nsnetworkvolumesusagedescription),
  and [Desktop privacy key](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdesktopfolderusagedescription)
  document usage-description and user-consent behavior.
- [Tailwind browser compatibility](https://tailwindcss.com/docs/compatibility)
  declares Safari 16.4 as its v4 baseline;
  [Apple Safari 16.4 release notes](https://developer.apple.com/documentation/safari-release-notes/safari-16_4-release-notes)
  identify the corresponding macOS Ventura 13.3 release.

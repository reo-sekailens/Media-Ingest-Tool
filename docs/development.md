# Development guide

## Toolchain

- Node `24.14.0` (pinned in `.node-version`)
- Rust `1.98.0` with `clippy` and `rustfmt` (pinned in `rust-toolchain.toml`)
- npm

## Host prerequisites

Tauri requires platform-native build dependencies in addition to Node and Rust:

- Windows: Microsoft C++ Build Tools and Edge WebView2.
- macOS: Xcode Command Line Tools for desktop work.
- Linux: a supported WebKitGTK development stack for the chosen distribution.

Follow the current [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for exact packages. Building on an OS does not certify that OS's removable-media behavior.

## Commands

```powershell
npm install
npm run check
npm run tauri dev
```

`npm run check` runs Prettier, ESLint, strict TypeScript, Vitest, Vite production build, Rust formatting, Clippy with warnings denied, and Rust tests.

## Support status

The scaffold is developed on Windows. The CI configuration compiles and tests platform-independent paths on Windows, macOS, and Linux. Device enumeration, hotplug, physical-card identity, copy performance, verification, formatting, and SanDisk slot mapping require separate native hardware certification described in [TASK010](../memory-bank/tasks/TASK010-cross-platform-hardware-certification.md).

## macOS support logs

Every packaged macOS build writes local, verbose support logs automatically.
They are retained outside the app bundle, use local timestamps, rotate at 5 MiB,
and keep previous rotations so a tester can send the complete launch-to-failure
record. The log does not record source or destination paths, raw hardware
identifiers, or file names.

After reproducing an issue, quit the app and in Finder choose **Go > Go to
Folder…**, then enter:

```text
~/Library/Logs/com.mediaingest.tool/
```

Send the `media-ingest-tool.log` file and any rotated copies in that folder.
The same folder can be opened in Terminal with:

```sh
open "$HOME/Library/Logs/com.mediaingest.tool"
```

## Security boundary

Rust owns storage operations and only exposes typed commands registered in the Tauri application manifest. Do not add direct frontend filesystem, shell, or arbitrary-path permissions. See [TASK001](../memory-bank/tasks/TASK001-foundation.md) and [TASK009](../memory-bank/tasks/TASK009-security-destructive-safety.md).

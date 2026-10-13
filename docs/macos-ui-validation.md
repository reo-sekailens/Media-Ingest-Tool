# Native macOS UI validation

The native smoke harness starts a dedicated `.app` on an actual Intel or Apple
Silicon macOS runner. It tests the bundled React frontend inside WKWebView, calls
the real Tauri startup commands, and saves a native empty-state screenshot.
It requires no Apple Developer account, paid driver service, or Node dependency.

## Isolation and release boundary

Build only in debug mode with the optional `macos-ui-tests` Cargo feature and
`tests/native-macos/tauri.test.conf.json`. The config gives the test app its own
`com.mediaingest.tool.macos-ui-tests` identifier and local store, enables the
global Tauri API for read-only assertions, and loads the embedded driver's empty
ACL set. The shipping identifier, frontend, CSP, permissions, and release
packages use the normal configuration.

The Rust test feature uses the pinned official WebdriverIO project's
`tauri-plugin-wdio-webdriver` 1.5.0. Its desktop HTTP server binds to loopback.
The feature additionally requires the dedicated test environment and identifier
before starting the driver. The Rust compile guard rejects feature-enabled
macOS release builds. A release bundle must never include the test configuration.
`cargo --all-features` by itself does not authorize starting an automation server.

The JavaScript harness uses Node's built-in `fetch` and `node:test` with the
driver's standard W3C session, execute, element-click, screenshot, and cleanup
endpoints. It does not install `tauri-plugin-wdio`, import a frontend test bridge,
mock native commands, or substitute a browser/Vite rendering session.

## Run on a Mac

Use the repository's pinned Node and Rust toolchains and locked dependencies.
The CI matrix runs both CPU architectures on the macOS 15 and macOS 26 images;
`run.json` records the actual OS version, architecture, source commit, dirty-tree
state, and test executable SHA-256. The exact runner OS version is the scope of
that run's evidence; it does not establish a minimum supported OS.

Intel:

```sh
npm ci
npm run tauri -- build --ci --debug --target x86_64-apple-darwin --bundles app \
  --features macos-ui-tests --config tests/native-macos/tauri.test.conf.json -- --locked
node scripts/run-macos-ui.mjs \
  --app 'src-tauri/target/x86_64-apple-darwin/debug/bundle/macos/Media Ingest Tool.app' \
  --output "${TMPDIR}/media-ingest-native-ui-intel-$(date +%s)" \
  --expected-arch x86_64
```

Apple Silicon:

```sh
npm ci
npm run tauri -- build --ci --debug --target aarch64-apple-darwin --bundles app \
  --features macos-ui-tests --config tests/native-macos/tauri.test.conf.json -- --locked
node scripts/run-macos-ui.mjs \
  --app 'src-tauri/target/aarch64-apple-darwin/debug/bundle/macos/Media Ingest Tool.app' \
  --output "${TMPDIR}/media-ingest-native-ui-arm64-$(date +%s)" \
  --expected-arch arm64
```

The output directory must be new, have an existing parent, and be outside the
repository. The harness verifies the dedicated bundle identifier and exact
single-architecture executable before launch, rejects Rosetta-translated runs,
selects a fresh loopback port, and stops its own app after the suite. It refuses
non-macOS execution, a shipping bundle, or a runner with removable media. No
source, media, profile, or destination is created by the tests; normal app startup
does initialize the separate test app's empty local database and support log.

The transport adapter has independent, portable contract checks:

```sh
node --test tests/native-macos/webdriver.contract.mjs
```

Those checks validate request envelopes, session cleanup, element references,
sanitized error handling, and bounded waits. They are not native runtime evidence.

## Assertions and artifacts

| Check                | Evidence                                                                                                                          |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Native app startup   | Actual `.app` executable, native CPU match, WKWebView user agent and bridge, dedicated app identifier, bundled `tauri:` origin    |
| Frontend/CSP startup | Rendered `main`, loaded stylesheet, computed background, declared minimum width, no horizontal overflow                           |
| Device inventory     | Real `get_device_snapshot` result with contract version and positive native sequence; rendered inventory agrees after startup     |
| Fixture separation   | Preview A-Cam/B-Cam cards disappear after native initialization                                                                   |
| Local history        | Real `get_ingest_history` succeeds against the isolated empty store                                                               |
| Empty UI             | Visible refresh action is an enabled focusable HTML button, driver click succeeds, and the native empty state remains consistent  |
| Screenshot           | `native-empty-state.png` comes from the embedded driver's native WKWebView snapshot and has a valid desktop-size PNG header       |
| Result/provenance    | `smoke-summary.json` contains counts, passed/skipped checks and limits; `run.json` records the test build and overall exit result |

The theme toggle and identity-detail panel are currently hidden when there is no
selected media. The suite explicitly skips these checks in the normal empty
runner; it cannot claim their macOS validation from empty-state success. The
conditional code verifies focusability, driver clicks, theme state, and panel
ARIA state only if a separately authorized test invokes the native suite with
real observed media. The production launcher deliberately requires no media, and
the screenshot test avoids retaining media details. Native stdout/stderr and raw
device snapshots are not copied into published evidence.

## Limits

The embedded driver implements clicks through DOM actions and keyboard events
through synthetic `KeyboardEvent` dispatch. In version 1.5.0, dispatching Tab
does not perform the browser's default focus traversal, and Enter/Space does not
perform default HTML-button activation. This suite verifies focusability and
driver click behavior; physical keyboard navigation remains unverified.

A passing native smoke is evidence for startup, rendering, and read-only IPC on
the recorded runner. It is a separate instrumented build, not the shipping app's
download/quarantine/Gatekeeper installation certificate. It does not validate
removable readers/cards, Full Disk Access or removable-volume permission prompts,
native picker dialogs, copying, recovery, eject, formatting, sleep/wake, minimum
macOS versions, or real device performance. Those remain separate certification
rows. A Windows transport-test pass cannot replace a macOS smoke run.

## Primary sources reviewed on 2026-10-13

- [Tauri WebDriver overview](https://v2.tauri.app/develop/tests/webdriver/) now documents native macOS through the embedded driver; the manual external `tauri-driver` path remains Windows/Linux only.
- [WebdriverIO plugin setup](https://webdriver.io/docs/desktop-testing/tauri/plugin-setup) describes the two separate plugins and keeping automation out of production builds.
- [Official embedded driver source](https://github.com/webdriverio/desktop-mobile/tree/main/packages/tauri-plugin-webdriver) documents the published crate's server, native WKWebView snapshot/evaluation, and DOM action implementation.
- [Published crate](https://crates.io/crates/tauri-plugin-wdio-webdriver/1.5.0) pins the implementation checked here. Its Tauri dependency starts at 2.10.0 and its Rust minimum is 1.77; the repository's locked Tauri 2.11.5 and Rust 1.98 satisfy those version requirements.
- [Tauri configuration reference](https://v2.tauri.app/reference/config/) defines the isolated identifier, global API switch, and inline capability configuration.
- [W3C WebDriver standard](https://www.w3.org/TR/webdriver2/) defines the session/command/value envelopes used by the dependency-free adapter.

# TASK008 — React operator workflow and accessible device UI

- **Status:** in progress — fixture/native inventory distinction, a card-scoped native source-media scan, card-scoped session destinations and sort controls, a native read-only organization-preview control, controlled reader-slot calibration, card-keyed concurrent native operation/cancellation state, and typed queued/copying/verifying/completed/failed/cancelled progress stages are implemented. The active native copy counter advances from actual 1 MiB writes and carries the one-based planned-file ordinal and total file count without exposing paths; the same path-free per-file byte stream reports the fresh destination digest readback as `verifying`. The UI shows the ordinal for manual, recovery, and auto-ingest copies and verification, and derives copy-stage average rate and ETA from aggregate bytes. The preview shows totals and representative paths without copying and is discarded when its destination or rule changes. A native local-history panel shows only run state, verified counts/bytes, and receipt availability after media removal. The React surface now uses a Tailwind-first minimalist blue system with an accessible light/dark mode toggle, plus a four-step card configuration dialog with isolated drafts, explicit saving, and the typed native registration boundary and marker limitation. Automatic-run state is now persisted by exact identity/generation rather than only in the webview: a fresh session visibly reports that this mount has already been auto-ingested and does not start another copy. Browser coverage verifies this suppression, a recognized profile's interval, and copying/verification ordinal updates. After any successful ingest, the client refreshes native inventory before accepting another action, reconciling marker-created mutable keys without weakening Rust's exact current-source revalidation. The freshly rebuilt package then completed concurrent full-size-SD and microSD UI ingests. Destination fields now prefer a permitted Tauri native directory picker and retain manual typing only as fallback. Durable strong-identity destination recall UI, accessible format confirmation, physical-remount auto-ingest, recovery verification progress for files already published before a crash, and cross-platform native rendered evidence remain pending.
- **Depends on:** TASK001, TASK002, TASK003, TASK004, TASK007
- **Unlocks:** TASK005 operator flow, TASK010
- **Latest visual evidence:** 2026-10-13 browser fixtures capture the four-step
  configuration dialog and saved workspace in light/dark responsive layouts.
  This is browser evidence only; no
  packaged-desktop rendering claim is made.
- **Latest accessibility evidence:** Secondary actions render with visible
  button boundaries; keyboard skip/focus treatment, named destination fields,
  modal overscroll containment, dark color-scheme support, and reduced motion
  are source- and browser-fixture-checked. Formal WCAG AA conformance and
  packaged-platform verification remain pending.
- **Control-spacing evidence:** 2026-08-27 browser QA confirms visible rounded
  secondary controls (for example, `Change` measures 80 by 40 px with 12 px
  horizontal padding), replacing the prior cramped link-like presentation.

## Objective

### 2026-10-13 capture-time choice and clock warning

Organization uses a keyboard-operable radio group for Camera time (default) and
Computer local time. Review and the applied workspace show the choice. A shared
warning tells users to synchronize camera/computer date, time, timezone, and DST
settings before capture-time sorting. Unknown offsets keep their recorded clock
in both modes and are explained. Normal Original tree hides clock controls and
warnings; an invalid saved basis exposes controls so it can be corrected.

Draft cancel leaves applied settings unchanged. Card saves, legacy hydration,
computer-local hydration, preview, manual ingest, and future-mount automatic
ingest are covered by typed integration tests. All 99 frontend tests, TypeScript,
ESLint, production build, and source formatting pass. Chromium fixture QA at
1440x1000, 390x844, and 720x500 covers keyboard selection, review/save/reopen,
Escape discard, scrolling and visible actions, with no horizontal overflow or
console errors. Light desktop, dark mobile, review, and saved-workspace screenshots are retained
outside Git. Preview paths/status are scoped to the exact source, applied
settings, and latest request; saving invalidates pending tokens. Eight regressions
cover deferred stale responses/errors, newer previews, and source changes. Playwright CLI was used because the Browser plugin was unavailable.
This is fixture-rendered evidence, not packaged/native IPC or physical-card proof.

### 2026-10-13 step configuration

The selected card now uses one draft-based dialog for Destination, Organization,
Automation, and Review, with a final Save configuration action. Inline editors
and the separate automatic-ingest setup were removed. The workspace renders an
applied summary and Configure/Edit configuration button. Drafts survive Back,
failed saves, and source removal; Cancel/Escape discards edits. Applied state is
scoped to source key, insertion generation, and source root, so one card's setup
does not become another card's setup.

The existing native directory picker updates only the draft. Step validation
covers destination, supported intervals, portable custom folders, folder order,
and automation dependencies. Pointer/drag and keyboard Left/Right/Home/End
ordering retain focus. Save/picker busy states block dismissal, while removed or
busy cards cannot save. A shared modal isolates background controls and restores
trigger focus. Body scrolling keeps actions visible on narrow/short windows.

Remember-on-card saving sends the reviewed payload through the existing typed
registration command and closes only after acknowledgement. Lookup failure does
not substitute defaults for unknown saved preferences; failed saves retain the
draft for retry. Session saving leaves persistent settings untouched. Automatic
ingest/format require persistence, and Save does not launch the current mount's
copy. Native profile rehydration handles mutable-marker key reconciliation.
Rust continues to own identity, path safety, copying, and format authorization.

Verification passes: 77 frontend tests, strict TypeScript, ESLint, production
build, formatting, and memory-bank validation. Chromium fixtures at 1440x1000,
390x844, and 720x500 cover all steps, save/reopen, cancel/discard, focus, folder
ordering, picker errors, scrolling, footer visibility, and long-content wrapping
in light/dark modes. No console errors or horizontal overflow were observed.
Screenshots are outside Git. Playwright CLI supplied rendered checks because the
Browser plugin was unavailable. Mocked native integration covers persistence,
failure/retry, profile hydration, disconnects, and future-mount automation.
Actual removable-card saving, the native picker, packaged rendering, and formal
accessibility conformance are not established by these checks.

### 2026-10-09 operator review

Dialogs now share focus containment, background isolation, trigger-focus
restoration, Escape handling, safe Cancel focus for destructive actions, and
busy-operation dismissal protection. Short windows scroll within the overlay.
Folder tags accept Left/Right/Home/End keys without losing focus; reordering
continues to discard stale previews. Identity details now expand rather than
acting as a dead control. Receipt opening stays disabled with an explanation
and the existing destination receipt location. Connected sources use a labelled
native list, and disabling automatic ingest clears its automatic-format option.
Theme metadata and favicon match the app. All 23 frontend tests, TypeScript,
ESLint, and production build pass. Production-fixture Chromium QA passes at
1440x1000, 390x844, and 720x500, including keyboard ordering/focus, details,
modal isolation/Tab/Escape/restore, destructive Cancel focus, theme metadata,
sticky toolbar, overflow, and short-window action visibility. The toolbar uses
overflow clipping with the prior hidden fallback for older engines. Screenshots
were captured outside the repository. Playwright CLI was used because the
Browser plugin was unavailable. Packaged/native and formal accessibility
evidence remain separate gates.

Build a calm, high-information desktop workflow in React and TypeScript with Tailwind CSS that makes source identity, destination, sort preview, copy progress, verification, errors, and formatting safety unambiguous.

## Required surfaces

- **Source list:** one card per detected source with status, media type, label, capacity/free space, filesystem, mount state, read-only state, reader, physical slot, and identity-confidence badge.
- **Source detail:** raw identifiers grouped by card, volume, reader, slot/topology, and session; mutable identifiers must be labeled as such.
- **Destination and rule editor:** native folder picker, free-space preflight, remembered-vs-session-only state, camera profile, timezone, and composable folder-template controls.
- **Card registration:** an explicit per-card setup action to assign a label, create/read the app marker, show that it is mutable continuity evidence, and opt into or out of automatic ingest on future mounts.
- **Sort preview:** representative destination paths plus collision/unknown-metadata warnings before ingest starts.
- **Ingest queue:** per-source and aggregate bytes/files progress, current stage, throughput, and ETA as separate values; pause/cancel semantics must match what the Rust pipeline can actually guarantee.
- **Verification result:** file counts, byte counts, full-digest outcome, skipped identical files, failures, and downloadable/openable receipt.
- **Format flow:** disabled until eligible, with explicit device re-identification, verification receipt, filesystem/profile, destructive warning, and post-format validation.
- **History:** prior sessions and outcomes without requiring the source card to be mounted.

## Interaction rules

- Rust owns device enumeration, file access, path safety, identity matching, copy state, verification, and format authorization. The webview only requests typed operations and renders state.
- Automatic ingest is opt-in per registered card and is visibly announced with its ordinary stop control. It starts only after Rust freshly observes the exact registered marker on one mounted source, accepts the configured destination through the normal preflight, and finds no active operation. It is a personal-workflow convenience: marker-only matching never upgrades hardware confidence and cannot authorize format, recovery, or strong destination recall.
- The UI must not claim that a marker is immutable or protected from other software. It may expose read-only/hidden marker status as a best-effort convenience, but must explain that only a physical full-size SD write-lock broadly prevents external writes and that neither setting prevents reformat.
- Device removal, sleep/wake, destination loss, low space, write-protect, permission denial, and metadata ambiguity receive specific recoverable states.
- Never show a low-confidence slot/card guess as confirmed. Permit an operator label without upgrading evidence confidence.
- Keyboard operation, visible focus, semantic status announcements, reduced motion, high contrast, and 200% zoom are required.
- Native and fixture modes must be visually distinguishable. Fixtures cannot certify native device or formatting behavior.

## Acceptance criteria

- A user can configure and run two simultaneous source ingests with different destinations without confusing their identities or progress.
- Same-model cameras are visibly disambiguated by body serial or operator label.
- Every destructive action identifies the exact current card and cannot be confirmed accidentally by pressing Enter on dialog open.
- An operator can register a card, later see its registration/auto-ingest status on mount, cancel an announced automatic ingest, and understand why a possible match was held for review.
- Layout remains usable at the minimum supported window size and at 200% scaling.
- TypeScript strict checks, component tests, accessibility checks, and browser fixture flows pass.
- Windows, macOS, and Linux native screenshots are captured for the source, ingest, verified, failure, and format-confirmation states.

## Latest UI evidence

- 2026-08-27 — Connected-media cards now present observed mounted drive letters
  and filesystem details with capacity/free-space and reader-slot details; the
  selected source repeats the drive information. History outcome labels are
  title-cased. Drive letters are mutable presentation details and cannot serve
  as source identity, destination-recall, or format-authorization evidence.

- 2026-08-27 — Light mode no longer relies on slate-400/500 or reduced opacity
  for readable interface text. Secondary labels, input placeholders, warning
  text, and disabled-button states now use explicit AA-safe foreground and
  surface tokens. Browser fixture checks measured the changed pairings at
  6.15:1–7.58:1.

- 2026-08-27 — The application now uses `lucide-react` rather than textual
  placeholder glyphs for the product mark, device cards, rescan, verification,
  warning, and close affordances. Browser fixture evidence confirms the icons
  render without overflow or console errors.

- 2026-08-27 — The packaged desktop icon now matches the app's Lucide-style
  hard-drive/download media-ingest mark. Tauri generated the platform icon
  variants from `src-tauri/icons/media-ingest.svg`; the rebuilt Windows NSIS
  installer carries that icon set.

- 2026-08-27 — Corrected the full operator text hierarchy. A global font
  shorthand had forced all controls to 16 px; controls now retain their own
  utility sizes and the rendered hierarchy explicitly separates selected-device
  title (36 px), section titles (18 px), status values (16 px), labels (11 px),
  secondary actions (12 px), and primary action (14 px).

- 2026-08-27 — Simplified the header identity to one `Media Ingest Tool` title
  beside the app mark, removing redundant secondary product naming.

- 2026-08-27 — Simplified the selected-device/status transition to one visual
  divider by removing the header's redundant bottom border.

- 2026-08-27 — Added a fixed, expandable aggregate-ingest dock fed by real
  active-operation progress: total progress/ETA while collapsed and
  per-operation stage, file position, bytes, rate, and bounded event logs when
  expanded. The workspace adds bottom scroll clearance for both dock states.

- 2026-08-27 — Refined the aggregate dock to appear only for active or queued
  work, use a compact collapsed summary, and align to the desktop main
  workspace instead of crossing the source-media rail.

- 2026-08-27 — Removed the redundant non-functional destination `Change`
  control, leaving the labelled directory field and folder chooser as the
  accessible functional controls.

- 2026-08-27 — Distinguished editable destination inputs from disabled dark
  panels with a darker active field surface, stronger border, and focus-ready
  visual treatment.

- 2026-08-27 — Matched destination-memory controls to native identity
  confidence: recall/save remain unavailable for session-only media, with a
  visible explanation and tooltip, while direct destination editing is retained.

## Research sources

- [Tauri IPC model](https://v2.tauri.app/concept/inter-process-communication/)
- [Tauri dialog plugin](https://v2.tauri.app/plugin/dialog/)
- [React with TypeScript](https://react.dev/learn/typescript)
- [Tailwind CSS with Vite](https://tailwindcss.com/docs/installation/using-vite)
- [WAI-ARIA live regions](https://www.w3.org/WAI/WCAG22/Techniques/aria/ARIA22)

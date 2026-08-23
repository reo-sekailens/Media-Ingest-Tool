# TASK008 — React operator workflow and accessible device UI

- **Status:** in progress — fixture/native inventory distinction, a card-scoped native source-media scan, card-scoped session destinations and sort controls, a native read-only organization-preview control, controlled reader-slot calibration, card-keyed concurrent native operation/cancellation state, and typed queued/copying/verifying/completed/failed/cancelled progress stages are implemented. The active native copy counter advances from actual 1 MiB writes; the UI derives a copy-stage average rate and ETA from those bytes. The preview shows totals and representative paths without copying and is discarded when its destination or sort rule changes. A native local-history panel shows only run state, verified counts/bytes, and receipt availability after media removal. The React surface now uses a Tailwind-first minimalist blue system with an accessible light/dark mode toggle, plus a modal-first auto-ingest setup flow that preserves the typed native registration boundary and explains the marker limitation. Destination fields now prefer a permitted Tauri native directory picker and retain manual typing only as fallback. Per-file progress, verification-stage byte progress, durable strong-identity destination recall UI, accessible format confirmation, and cross-platform native rendered evidence remain pending.
- **Depends on:** TASK001, TASK002, TASK003, TASK004, TASK007
- **Unlocks:** TASK005 operator flow, TASK010

## Objective

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

## Research sources

- [Tauri IPC model](https://v2.tauri.app/concept/inter-process-communication/)
- [Tauri dialog plugin](https://v2.tauri.app/plugin/dialog/)
- [React with TypeScript](https://react.dev/learn/typescript)
- [Tailwind CSS with Vite](https://tailwindcss.com/docs/installation/using-vite)
- [WAI-ARIA live regions](https://www.w3.org/WAI/WCAG22/Techniques/aria/ARIA22)

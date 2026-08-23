# Decisions

## 2026-08-23 — Initialize a repository-local memory bank

**Status:** accepted

**Decision:** Maintain durable project context in `memory-bank/` using focused Markdown files for product, technical, active-work, progress, decisions, tasks, and certification evidence.

**Rationale:** The repository had no documented project context. Templates preserve unknowns explicitly and provide a stable place for verified context as work begins.

**Consequences:** Contributors must follow the [maintenance rule](README.md#maintenance-rule) and update relevant records as work changes.

## 2026-08-23 — Use Tauri, Rust, React, TypeScript, and Tailwind CSS

**Status:** accepted

**Decision:** Use Tauri 2 as the native desktop shell, Rust for trusted device/file operations, React with strict TypeScript for UI state, Vite for the SPA build, and Tailwind CSS through its Vite integration.

**Rationale:** This stack was explicitly selected for a cross-platform desktop ingest tool and supports a narrow native/webview trust boundary.

**Consequences:** Device discovery, copy, verification, and formatting remain in Rust; UI code cannot be the authority for filesystem or destructive operations.

## 2026-08-23 — Preserve identity evidence and confidence

**Status:** accepted

**Decision:** Store namespaced raw identity evidence with provenance and confidence. Prefer exposed SD CID or disk hardware IDs; treat reader/slot, volume UUID, mount path, label, and content fingerprints as progressively weaker evidence. Never invent an immutable identifier.

**Rationale:** USB bridges and operating systems do not always expose the physical SD CID. Drive letters, mount paths, labels, and volume identifiers can change or be reused.

**Consequences:** Automatic per-card destination restoration and destructive authorization require an accepted exact match. Lower-confidence cases remain session-only or require operator confirmation.

## 2026-08-23 — Verify with a fresh destination read

**Status:** accepted

**Decision:** Hash source bytes with BLAKE3 during copy, close/flush the temporary destination file, then fully re-read and hash the destination before publishing and marking it verified.

**Rationale:** Size/timestamp checks and a digest computed only from the write stream do not prove the stored destination bytes can be read back intact. BLAKE3 is designed for high-throughput parallel hashing.

**Consequences:** Published end-to-end throughput includes the verification read. Faster but weaker verification modes are not considered complete ingest.

## 2026-08-23 — Keep formatting fail-closed

**Status:** accepted

**Decision:** Quick format is available only for an exact currently connected removable source whose current ingest receipt is complete and verified. Re-resolve identity immediately before execution and permanently exclude system/internal/destination/ambiguous disks.

**Rationale:** Formatting is destructive, mount paths can be reused, and OS formatting may not provide the SD-specific layout or camera customization expected by every device.

**Consequences:** Camera-ready claims require an exact tested profile. The UI must disclose that quick format is not secure erase and may require elevated authorization.

## 2026-08-23 — Require contextual Git commit messages

**Status:** accepted

**Decision:** Every generated commit message must have a concise, context-specific, imperative title followed by a blank-line-separated body that describes the purpose and every material addition, change, and removal. Include relevant verification, migration, or compatibility notes.

**Rationale:** A title alone does not preserve enough context for reviewers or future maintainers to understand the scope and intent of a commit.

**Consequences:** Generated commit messages must follow the repository rule in `AGENTS.md`; vague or title-only messages are not acceptable.

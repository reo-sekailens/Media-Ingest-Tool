# TASK007 — Local state, device destinations, and camera profiles

- **Status:** In progress — Rust-owned SQLite migrations, safe exact-match destination profiles exposed only through currently observed native strong-medium commands, audited native ingest-run lifecycle, committed verified-file evidence, and exact reader-LUN slot calibration persistence are implemented with local tests; camera profiles and cross-platform/live strong-card profile recall remain pending.
- **Depends on:** TASK001, TASK002
- **Unlocks:** TASK003, TASK008, TASK010

## Objective

Persist device observations, exact identity evidence, per-device destination rules, camera assignments, ingest sessions, file states, and verification receipts locally without turning mutable labels or paths into identity.

## Scope

- Use a versioned SQLite database owned exclusively by Rust. The frontend receives typed projections and never issues SQL.
- Define records for `device_observation`, `source_identity`, `destination_profile`, `card_registration`, `camera_profile`, `ingest_session`, `ingest_file`, `verification_receipt`, and `format_receipt`.
- Store every raw identifier with its namespace, normalized value, provenance, confidence, first/last observation, and relationship to the current connection generation.
- Key destination profiles to exact accepted `source_identity` values. Support a session-only destination when identity confidence is insufficient.
- Store an explicit per-card registration label, app-marker token/fingerprint, auto-ingest opt-in, and marker-restoration intent. Registration is a local preference plus mutable continuity evidence, never a hardware-identity upgrade or format authorization. Keep enough non-sensitive identity evidence to explain why a post-format restoration or mount automation was refused.
- Allow multiple destinations in the future while requiring exactly one primary destination per source in the initial UI.
- Store camera make, model, body serial, operator label, and evidence source independently. Never use camera model alone as the camera key.
- Use schema migrations, transactions, foreign keys, and corruption-safe backup/recovery behavior. Do not store media payloads or secrets.
- Export a human-readable, versioned JSON ingest receipt alongside the database-backed record.

## Acceptance criteria

- Reconnecting a fixture with the same exact high-confidence identity restores its destination; a near match does not.
- Formatting or changing a volume label/UUID cannot silently create a false hardware match.
- A user can explicitly opt into auto-ingest for an exact currently observed app marker. This is a personal-workflow shortcut, not a security decision: missing markers do not match, cloned markers remain a known limitation, and marker evidence cannot authorize format, strong destination recall, or recovery.
- A card registration survives reformat in local state, but restoration writes its saved marker only through TASK005's post-format exact-identity gate; loss of that gate leaves the registration recoverable by manual operator setup, not silently reassigned.
- Two cameras with the same model and distinct body serials remain separate profiles and sorting roots.
- An absent body serial produces an explicit `unresolved` or operator-assigned identity, never an invented value.
- Session and file transitions are transactional and reject invalid regressions such as `verified -> copying`.
- Database migrations are tested from every checked-in schema version, including rollback/backup recovery behavior.
- Exported receipts contain source identity evidence, file paths, sizes, BLAKE3 digests, timestamps, outcomes, and app/schema versions without leaking unrelated host data.

## Verification evidence

- Rust migration and repository tests using a temporary database.
- Identity exact-match/near-match fixture tests.
- Crash/reopen test proving the active ingest resumes from durable state.
- Sanitized JSON receipt fixture reviewed against the schema.

## Research sources

- [SQLite transactions](https://www.sqlite.org/lang_transaction.html)
- [SQLite Write-Ahead Logging](https://www.sqlite.org/wal.html)
- [Tauri application data directory API](https://v2.tauri.app/reference/javascript/path/#appdatadir)

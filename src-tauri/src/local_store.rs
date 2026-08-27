//! Rust-owned durable configuration for trusted ingest state.

use crate::identity::IdentityStrength;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use std::path::Path;

const SCHEMA_VERSION: i64 = 11;

pub struct LocalStore {
    connection: Connection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentityRecord {
    pub identity_key: String,
    pub source: String,
    pub normalized_value: String,
    pub strength: IdentityStrength,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestRunState {
    Queued,
    Copying,
    RecoveryRequired,
    Completed,
    Failed,
}

/// Native-only recovery information. These filesystem paths must not cross
/// the webview boundary: recovery always revalidates the currently observed
/// medium before they can be used.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableIngestRun {
    pub run_id: String,
    pub source_identity_key: String,
    pub source_generation: u64,
    pub source_root: String,
    pub destination_root: String,
    /// Preserves whether the interrupted run was started by the registered
    /// mount workflow, so recovery retains its native-only auto-format gate.
    pub auto_ingest_triggered: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReaderSlotKind {
    Sd,
    MicroSd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedFileRecord {
    pub entry_id: String,
    pub source_relative_path: String,
    pub destination_relative_path: String,
    pub byte_length: u64,
    pub source_blake3: String,
    pub destination_blake3: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFileRecord {
    pub entry_id: String,
    pub source_relative_path: String,
    pub destination_relative_path: String,
    pub byte_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestHistoryEntry {
    pub run_id: String,
    /// Opaque comparison key only; it is used to restore safe-eject eligibility
    /// for the currently re-observed device, not rendered to the operator.
    pub source_identity_key: String,
    pub source_generation: u64,
    pub state: String,
    pub updated_at: String,
    pub verified_file_count: u64,
    pub verified_bytes: u64,
    pub receipt_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerIngestProfile {
    pub marker_token: String,
    pub destination_path: String,
    pub sort_mode: String,
    /// The interval selected for `camera_interval`; no guessed value is used
    /// when a registered card mounts again.
    pub interval_minutes: Option<u16>,
    pub auto_ingest_enabled: bool,
    pub auto_format_enabled: bool,
}

impl LocalStore {
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let connection = Connection::open(path)?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    fn in_memory() -> rusqlite::Result<Self> {
        let store = Self {
            connection: Connection::open_in_memory()?,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            CREATE TABLE IF NOT EXISTS schema_migration (
              version INTEGER PRIMARY KEY,
              applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS source_identity (
              identity_key TEXT PRIMARY KEY,
              source TEXT NOT NULL,
              normalized_value TEXT NOT NULL,
              strength TEXT NOT NULL,
              first_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS destination_profile (
              identity_key TEXT PRIMARY KEY REFERENCES source_identity(identity_key) ON DELETE CASCADE,
              destination_path TEXT NOT NULL,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS ingest_run (
              run_id TEXT PRIMARY KEY,
              source_identity_key TEXT NOT NULL REFERENCES source_identity(identity_key),
              source_generation INTEGER NOT NULL,
              source_root TEXT,
              destination_root TEXT,
              auto_ingest_triggered INTEGER NOT NULL DEFAULT 0 CHECK(auto_ingest_triggered IN (0, 1)),
              state TEXT NOT NULL,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS ingest_file (
              entry_id TEXT PRIMARY KEY,
              run_id TEXT NOT NULL REFERENCES ingest_run(run_id) ON DELETE CASCADE,
              source_relative_path TEXT NOT NULL,
              destination_relative_path TEXT NOT NULL,
              byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
              source_blake3 TEXT,
              destination_blake3 TEXT,
              state TEXT NOT NULL,
              updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS state_event (
              event_id INTEGER PRIMARY KEY,
              run_id TEXT NOT NULL REFERENCES ingest_run(run_id) ON DELETE CASCADE,
              previous_state TEXT,
              next_state TEXT NOT NULL,
              reason TEXT NOT NULL,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS reader_slot_calibration (
              reader_fingerprint TEXT NOT NULL,
              logical_unit INTEGER NOT NULL CHECK(logical_unit >= 0 AND logical_unit <= 255),
              slot_kind TEXT NOT NULL CHECK(slot_kind IN ('sd', 'micro_sd')),
              evidence_note TEXT NOT NULL,
              calibrated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              PRIMARY KEY(reader_fingerprint, logical_unit)
            );
            CREATE TABLE IF NOT EXISTS ingest_receipt (
              run_id TEXT PRIMARY KEY REFERENCES ingest_run(run_id) ON DELETE CASCADE,
              manifest_algorithm TEXT NOT NULL,
              manifest_root_blake3 TEXT NOT NULL,
              verified_file_count INTEGER NOT NULL CHECK(verified_file_count >= 0),
              verified_bytes INTEGER NOT NULL CHECK(verified_bytes >= 0),
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS marker_ingest_profile (
              marker_token TEXT PRIMARY KEY,
              destination_path TEXT NOT NULL,
              sort_mode TEXT NOT NULL CHECK(sort_mode IN ('original_tree', 'camera_day', 'camera_interval')),
              interval_minutes INTEGER CHECK(interval_minutes IS NULL OR (interval_minutes >= 1 AND interval_minutes <= 1440)),
              auto_ingest_enabled INTEGER NOT NULL CHECK(auto_ingest_enabled IN (0, 1)),
              auto_format_enabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_format_enabled IN (0, 1)),
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS format_receipt (
              run_id TEXT PRIMARY KEY REFERENCES ingest_run(run_id) ON DELETE CASCADE,
              source_identity_key TEXT NOT NULL,
              source_generation INTEGER NOT NULL,
              profile_id TEXT NOT NULL,
              marker_restored INTEGER NOT NULL CHECK(marker_restored = 1),
              completed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            INSERT OR IGNORE INTO schema_migration(version) VALUES (1);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (2);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (3);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (4);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (5);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (6);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (7);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (8);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (9);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (10);
            INSERT OR IGNORE INTO schema_migration(version) VALUES (11);
            ",
        )?;
        self.add_column_if_missing("ingest_run", "source_root", "TEXT")?;
        self.add_column_if_missing("ingest_run", "destination_root", "TEXT")?;
        self.add_column_if_missing(
            "ingest_run",
            "auto_ingest_triggered",
            "INTEGER NOT NULL DEFAULT 0 CHECK(auto_ingest_triggered IN (0, 1))",
        )?;
        self.connection.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS ingest_run_one_auto_mount
              ON ingest_run(source_identity_key, source_generation)
              WHERE auto_ingest_triggered = 1
                AND state IN ('queued', 'copying', 'recovery_required', 'completed');",
        )?;
        self.add_column_if_missing(
            "marker_ingest_profile",
            "auto_format_enabled",
            "INTEGER NOT NULL DEFAULT 0 CHECK(auto_format_enabled IN (0, 1))",
        )?;
        self.add_column_if_missing(
            "marker_ingest_profile",
            "interval_minutes",
            "INTEGER CHECK(interval_minutes IS NULL OR (interval_minutes >= 1 AND interval_minutes <= 1440))",
        )?;
        debug_assert_eq!(SCHEMA_VERSION, 11);
        Ok(())
    }

    fn add_column_if_missing(
        &self,
        table: &str,
        column: &str,
        declaration: &str,
    ) -> rusqlite::Result<()> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !columns.iter().any(|existing| existing == column) {
            self.connection.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
            ))?;
        }
        Ok(())
    }

    pub fn observe_identity(&mut self, identity: &SourceIdentityRecord) -> rusqlite::Result<()> {
        let transaction = self.connection.transaction()?;
        upsert_identity(&transaction, identity)?;
        transaction.commit()
    }

    /// Destination recall is permitted only for strong evidence that belongs to
    /// the actual medium. Filesystem and topology evidence can be observed but
    /// cannot create a silent remembered destination.
    pub fn set_primary_destination(
        &mut self,
        identity: &SourceIdentityRecord,
        destination_path: &str,
    ) -> rusqlite::Result<bool> {
        if !allows_persistent_destination(identity.strength) {
            return Ok(false);
        }
        let transaction = self.connection.transaction()?;
        upsert_identity(&transaction, identity)?;
        transaction.execute(
            "
            INSERT INTO destination_profile(identity_key, destination_path)
            VALUES (?1, ?2)
            ON CONFLICT(identity_key) DO UPDATE SET
              destination_path = excluded.destination_path,
              updated_at = CURRENT_TIMESTAMP
            ",
            params![identity.identity_key, destination_path],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn primary_destination(&self, identity_key: &str) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT destination_path FROM destination_profile WHERE identity_key = ?1",
                params![identity_key],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn save_marker_ingest_profile(
        &mut self,
        profile: &MarkerIngestProfile,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO marker_ingest_profile(marker_token, destination_path, sort_mode, interval_minutes, auto_ingest_enabled, auto_format_enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(marker_token) DO UPDATE SET destination_path = excluded.destination_path,
                 sort_mode = excluded.sort_mode, interval_minutes = excluded.interval_minutes,
                 auto_ingest_enabled = excluded.auto_ingest_enabled,
                 auto_format_enabled = excluded.auto_format_enabled,
                 updated_at = CURRENT_TIMESTAMP",
            params![profile.marker_token, profile.destination_path, profile.sort_mode, profile.interval_minutes, profile.auto_ingest_enabled, profile.auto_format_enabled],
        )?;
        Ok(())
    }

    pub fn marker_ingest_profile(
        &self,
        marker_token: &str,
    ) -> rusqlite::Result<Option<MarkerIngestProfile>> {
        self.connection
            .query_row(
                "SELECT marker_token, destination_path, sort_mode, interval_minutes, auto_ingest_enabled, auto_format_enabled
             FROM marker_ingest_profile WHERE marker_token = ?1",
                params![marker_token],
                |row| {
                    Ok(MarkerIngestProfile {
                        marker_token: row.get(0)?,
                        destination_path: row.get(1)?,
                        sort_mode: row.get(2)?,
                        interval_minutes: row.get(3)?,
                        auto_ingest_enabled: row.get(4)?,
                        auto_format_enabled: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    /// A format receipt is written only after the native provider has
    /// revalidated the filesystem and the same registered marker is restored.
    /// Repeating a completed run cannot overwrite the first destructive-event
    /// record, and a receipt cannot be created for an unsealed ingest.
    pub fn record_completed_format(
        &mut self,
        run_id: &str,
        source_identity_key: &str,
        source_generation: u64,
        profile_id: &str,
    ) -> rusqlite::Result<bool> {
        let inserted = self.connection.execute(
            "INSERT OR IGNORE INTO format_receipt(
                run_id, source_identity_key, source_generation, profile_id, marker_restored
              )
              SELECT run.run_id, run.source_identity_key, run.source_generation, ?4, 1
              FROM ingest_run AS run
              INNER JOIN ingest_receipt AS receipt ON receipt.run_id = run.run_id
              WHERE run.run_id = ?1
                AND run.source_identity_key = ?2
                AND run.source_generation = ?3
                AND run.state = 'completed'",
            params![run_id, source_identity_key, source_generation, profile_id],
        )?;
        Ok(inserted == 1)
    }

    /// Sanitized history projection for the operator UI. Paths, identity raw
    /// values, digests, and file names remain in the native local store.
    pub fn recent_ingest_runs(
        &self,
        requested_limit: u8,
    ) -> rusqlite::Result<Vec<IngestHistoryEntry>> {
        let limit = i64::from(requested_limit.clamp(1, 50));
        let mut statement = self.connection.prepare(
            "
            SELECT run.run_id, run.source_identity_key, run.source_generation,
                   run.state, run.updated_at,
                   COALESCE(receipt.verified_file_count, 0),
                   COALESCE(receipt.verified_bytes, 0),
                   receipt.run_id IS NOT NULL
            FROM ingest_run AS run
            LEFT JOIN ingest_receipt AS receipt ON receipt.run_id = run.run_id
            ORDER BY run.updated_at DESC, run.run_id DESC
            LIMIT ?1
            ",
        )?;
        let entries = statement
            .query_map(params![limit], |row| {
                Ok(IngestHistoryEntry {
                    run_id: row.get(0)?,
                    source_identity_key: row.get(1)?,
                    source_generation: row.get(2)?,
                    state: row.get(3)?,
                    updated_at: row.get(4)?,
                    verified_file_count: row.get(5)?,
                    verified_bytes: row.get(6)?,
                    receipt_available: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    /// Returns true only for the exact completed run whose immutable receipt
    /// seal belongs to the requested source identity and insertion generation.
    /// This narrow query is the persistence half of the format preflight; it
    /// intentionally exposes no paths, media names, or digest material.
    pub fn has_completed_receipt_for_source(
        &self,
        run_id: &str,
        source_identity_key: &str,
        source_generation: u64,
    ) -> rusqlite::Result<bool> {
        self.connection.query_row(
            "
            SELECT EXISTS(
              SELECT 1
              FROM ingest_run AS run
              INNER JOIN ingest_receipt AS receipt ON receipt.run_id = run.run_id
              WHERE run.run_id = ?1
                AND run.source_identity_key = ?2
                AND run.source_generation = ?3
                AND run.state = 'completed'
            )
            ",
            params![run_id, source_identity_key, source_generation],
            |row| row.get(0),
        )
    }

    pub fn integrity_check(&self) -> rusqlite::Result<bool> {
        self.connection
            .query_row("PRAGMA integrity_check", [], |row| {
                let result: String = row.get(0)?;
                Ok(result == "ok")
            })
    }

    pub fn begin_ingest_run(
        &mut self,
        run_id: &str,
        identity: &SourceIdentityRecord,
        source_generation: u64,
        source_root: &str,
        destination_root: &str,
    ) -> rusqlite::Result<()> {
        self.begin_ingest_run_with_mode(
            run_id,
            identity,
            source_generation,
            source_root,
            destination_root,
            false,
        )
    }

    pub fn begin_ingest_run_with_mode(
        &mut self,
        run_id: &str,
        identity: &SourceIdentityRecord,
        source_generation: u64,
        source_root: &str,
        destination_root: &str,
        auto_ingest_triggered: bool,
    ) -> rusqlite::Result<()> {
        let transaction = self.connection.transaction()?;
        upsert_identity(&transaction, identity)?;
        transaction.execute(
            "INSERT INTO ingest_run(
                run_id, source_identity_key, source_generation, source_root, destination_root,
                auto_ingest_triggered, state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run_id,
                identity.identity_key,
                source_generation,
                source_root,
                destination_root,
                auto_ingest_triggered,
                ingest_run_state_name(IngestRunState::Queued),
            ],
        )?;
        transaction.execute(
            "INSERT INTO state_event(run_id, previous_state, next_state, reason)
             VALUES (?1, NULL, ?2, 'created')",
            params![run_id, ingest_run_state_name(IngestRunState::Queued)],
        )?;
        transaction.commit()
    }

    /// A persisted completed automatic run suppresses another automatic copy
    /// for the same currently observed insertion generation, including after a
    /// desktop-app restart. A later physical removal/reinsert receives a new
    /// generation and is eligible again.
    pub fn has_completed_auto_ingest_for_source(
        &self,
        source_identity_key: &str,
        source_generation: u64,
    ) -> rusqlite::Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM ingest_run AS run
                INNER JOIN ingest_receipt AS receipt ON receipt.run_id = run.run_id
                WHERE run.source_identity_key = ?1
                  AND run.source_generation = ?2
                  AND run.auto_ingest_triggered = 1
                  AND run.state = 'completed'
             )",
            params![source_identity_key, source_generation],
            |row| row.get(0),
        )
    }

    /// A freshly formatted managed card can be rediscovered as a new
    /// connection generation while its marker is being restored. This record
    /// is only a post-format suppression hint; it never authorizes a format.
    pub fn has_completed_format_for_source(
        &self,
        source_identity_key: &str,
    ) -> rusqlite::Result<bool> {
        self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM format_receipt
                WHERE source_identity_key = ?1
                  AND marker_restored = 1
             )",
            params![source_identity_key],
            |row| row.get(0),
        )
    }

    /// Returns the newest observed insertion generation for each identity so
    /// the in-memory connection tracker can survive an app restart. The
    /// tracker still advances a generation only after it observes removal.
    pub fn latest_source_generations(&self) -> rusqlite::Result<Vec<(String, u64)>> {
        let mut statement = self.connection.prepare(
            "SELECT source_identity_key, MAX(source_generation)
             FROM ingest_run
             GROUP BY source_identity_key",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    pub fn transition_ingest_run(
        &mut self,
        run_id: &str,
        next: IngestRunState,
        reason: &str,
    ) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let current = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Ok(false);
        };
        let Some(previous) = ingest_run_state_from_name(&current) else {
            return Ok(false);
        };
        if !can_transition_ingest_run(previous, next) {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE ingest_run SET state = ?2, updated_at = CURRENT_TIMESTAMP WHERE run_id = ?1",
            params![run_id, ingest_run_state_name(next)],
        )?;
        transaction.execute(
            "INSERT INTO state_event(run_id, previous_state, next_state, reason)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                run_id,
                ingest_run_state_name(previous),
                ingest_run_state_name(next),
                reason,
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Records a non-state-changing, native-only outcome against an existing
    /// ingest. This preserves destructive-operation diagnostics even after a
    /// completed run no longer appears as an active UI operation.
    pub fn record_ingest_note(&mut self, run_id: &str, reason: &str) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(state) = state else {
            return Ok(false);
        };
        transaction.execute(
            "INSERT INTO state_event(run_id, previous_state, next_state, reason)
             VALUES (?1, ?2, ?2, ?3)",
            params![run_id, state, reason],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// A process cannot prove that active workers survived a restart. Preserve
    /// runs that have the complete native-only recovery location snapshot as
    /// explicit recovery candidates; legacy/incomplete rows are failed. This
    /// never resumes a mount path automatically.
    pub fn reconcile_interrupted_runs(&mut self) -> rusqlite::Result<u64> {
        let transaction = self.connection.transaction()?;
        let interrupted = {
            let mut statement = transaction.prepare(
                "SELECT run_id, state, source_root, destination_root
                 FROM ingest_run WHERE state IN ('queued', 'copying')",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (run_id, previous_state, source_root, destination_root) in &interrupted {
            let can_recover = source_root
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && destination_root
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty());
            let next_state = if can_recover {
                IngestRunState::RecoveryRequired
            } else {
                IngestRunState::Failed
            };
            let reason = if can_recover {
                "interrupted application session requires explicit identity-checked recovery"
            } else {
                "interrupted application session lacks a complete recovery location snapshot"
            };
            transaction.execute(
                "UPDATE ingest_run SET state = ?3, updated_at = CURRENT_TIMESTAMP
                 WHERE run_id = ?1 AND state = ?2",
                params![run_id, previous_state, ingest_run_state_name(next_state)],
            )?;
            transaction.execute(
                "INSERT INTO state_event(run_id, previous_state, next_state, reason)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    run_id,
                    previous_state,
                    ingest_run_state_name(next_state),
                    reason
                ],
            )?;
        }
        transaction.commit()?;
        Ok(interrupted.len() as u64)
    }

    pub fn recoverable_ingest_run(
        &self,
        run_id: &str,
    ) -> rusqlite::Result<Option<RecoverableIngestRun>> {
        self.connection
            .query_row(
                "SELECT run_id, source_identity_key, source_generation, source_root, destination_root,
                        auto_ingest_triggered
                 FROM ingest_run
                 WHERE run_id = ?1 AND state = 'recovery_required'
                   AND source_root IS NOT NULL AND destination_root IS NOT NULL",
                params![run_id],
                |row| {
                    Ok(RecoverableIngestRun {
                        run_id: row.get(0)?,
                        source_identity_key: row.get(1)?,
                        source_generation: row.get(2)?,
                        source_root: row.get(3)?,
                        destination_root: row.get(4)?,
                        auto_ingest_triggered: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn begin_explicit_recovery(
        &mut self,
        run_id: &str,
        source_generation: u64,
    ) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let updated = transaction.execute(
            "UPDATE ingest_run SET state = 'copying', source_generation = ?2, updated_at = CURRENT_TIMESTAMP
             WHERE run_id = ?1 AND state = 'recovery_required'",
            params![run_id, source_generation],
        )?;
        if updated == 1 {
            transaction.execute(
                "INSERT INTO state_event(run_id, previous_state, next_state, reason)
                 VALUES (?1, 'recovery_required', 'copying', 'explicit recovery started')",
                params![run_id],
            )?;
        }
        transaction.commit()?;
        Ok(updated == 1)
    }

    /// Recovery failures retain the frozen plan for another explicit attempt;
    /// they must not be silently restarted and are never completed/formatable.
    pub fn return_to_recovery_required(
        &mut self,
        run_id: &str,
        reason: &str,
    ) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let updated = transaction.execute(
            "UPDATE ingest_run SET state = 'recovery_required', updated_at = CURRENT_TIMESTAMP
             WHERE run_id = ?1 AND state = 'copying'",
            params![run_id],
        )?;
        if updated == 1 {
            transaction.execute(
                "INSERT INTO state_event(run_id, previous_state, next_state, reason)
                 VALUES (?1, 'copying', 'recovery_required', ?2)",
                params![run_id, reason],
            )?;
        }
        transaction.commit()?;
        Ok(updated == 1)
    }

    /// Persists evidence only after the copy primitive has independently
    /// reopened and hashed the destination.  The run must still be copying;
    /// terminal runs cannot acquire new file records.
    pub fn record_verified_file(
        &mut self,
        run_id: &str,
        file: &VerifiedFileRecord,
    ) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if state.as_deref() != Some(ingest_run_state_name(IngestRunState::Copying)) {
            return Ok(false);
        }
        let updated = transaction.execute(
            "UPDATE ingest_file SET
                source_blake3 = ?3,
                destination_blake3 = ?4,
                state = 'committed',
                updated_at = CURRENT_TIMESTAMP
             WHERE entry_id = ?1 AND run_id = ?2 AND state = 'planned'
               AND source_relative_path = ?5
               AND destination_relative_path = ?6
               AND byte_length = ?7",
            params![
                file.entry_id,
                run_id,
                file.source_blake3,
                file.destination_blake3,
                file.source_relative_path,
                file.destination_relative_path,
                file.byte_length,
            ],
        )?;
        transaction.commit()?;
        Ok(updated == 1)
    }

    pub fn record_planned_file(
        &mut self,
        run_id: &str,
        file: &PlannedFileRecord,
    ) -> rusqlite::Result<bool> {
        let transaction = self.connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if state.as_deref() != Some(ingest_run_state_name(IngestRunState::Queued)) {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO ingest_file(
                entry_id, run_id, source_relative_path, destination_relative_path,
                byte_length, state
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'planned')",
            params![
                file.entry_id,
                run_id,
                file.source_relative_path,
                file.destination_relative_path,
                file.byte_length,
            ],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn planned_file_count(&self, run_id: &str) -> rusqlite::Result<u64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM ingest_file WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
    }

    /// Returns the frozen plan only for a recovery-required run. No caller may
    /// reconstruct a plan from a current directory scan, which could silently
    /// include a different card's files after a crash.
    pub fn recovery_planned_files(&self, run_id: &str) -> rusqlite::Result<Vec<PlannedFileRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT entry_id, source_relative_path, destination_relative_path, byte_length
             FROM ingest_file
             WHERE run_id = ?1 AND state = 'planned'
               AND EXISTS(
                 SELECT 1 FROM ingest_run
                 WHERE run_id = ?1 AND state = 'recovery_required'
               )
             ORDER BY entry_id",
        )?;
        let files = statement
            .query_map(params![run_id], |row| {
                Ok(PlannedFileRecord {
                    entry_id: row.get(0)?,
                    source_relative_path: row.get(1)?,
                    destination_relative_path: row.get(2)?,
                    byte_length: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(files)
    }

    pub fn verified_file_count(&self, run_id: &str) -> rusqlite::Result<u64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM ingest_file
             WHERE run_id = ?1 AND state = 'committed'",
            params![run_id],
            |row| row.get(0),
        )
    }

    /// Seals the exact committed file set for a copying run. Completion is
    /// deliberately impossible until this immutable receipt row exists.
    pub fn seal_ingest_receipt(
        &mut self,
        run_id: &str,
        manifest_algorithm: &str,
        manifest_root_blake3: &str,
        expected_file_count: u64,
        expected_bytes: u64,
    ) -> rusqlite::Result<bool> {
        if manifest_algorithm.trim().is_empty() || manifest_root_blake3.trim().is_empty() {
            return Ok(false);
        }
        let transaction = self.connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if state.as_deref() != Some(ingest_run_state_name(IngestRunState::Copying)) {
            return Ok(false);
        }
        let (planned_count, committed_count, committed_bytes): (u64, u64, u64) = transaction
            .query_row(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN state = 'committed' THEN 1 ELSE 0 END),
                    COALESCE(SUM(CASE WHEN state = 'committed' THEN byte_length ELSE 0 END), 0)
                 FROM ingest_file WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if planned_count != expected_file_count
            || committed_count != expected_file_count
            || committed_bytes != expected_bytes
        {
            return Ok(false);
        }
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO ingest_receipt(
                run_id, manifest_algorithm, manifest_root_blake3,
                verified_file_count, verified_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                manifest_algorithm,
                manifest_root_blake3,
                expected_file_count,
                expected_bytes,
            ],
        )?;
        transaction.commit()?;
        Ok(inserted == 1)
    }

    /// Returns only the path-free manifest root for a sealed receipt. Native
    /// managed-card formatting uses it to verify the compact marker witness;
    /// it never exposes a source path or file list to IPC.
    pub fn receipt_manifest_root(&self, run_id: &str) -> rusqlite::Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT manifest_root_blake3 FROM ingest_receipt WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Atomically records every verified plan entry, seals its immutable
    /// receipt, and advances the run to completed. A mismatch rolls back the
    /// complete group so interrupted runs cannot contain a partly committed
    /// completion manifest that looks authoritative during recovery.
    pub fn commit_verified_completion(
        &mut self,
        run_id: &str,
        files: &[VerifiedFileRecord],
        manifest_algorithm: &str,
        manifest_root_blake3: &str,
        expected_file_count: u64,
        expected_bytes: u64,
    ) -> rusqlite::Result<bool> {
        if manifest_algorithm.trim().is_empty() || manifest_root_blake3.trim().is_empty() {
            return Ok(false);
        }
        let transaction = self.connection.transaction()?;
        let state = transaction
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = ?1",
                params![run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if state.as_deref() != Some(ingest_run_state_name(IngestRunState::Copying)) {
            return Ok(false);
        }
        for file in files {
            let updated = transaction.execute(
                "UPDATE ingest_file SET
                    source_blake3 = ?3,
                    destination_blake3 = ?4,
                    state = 'committed',
                    updated_at = CURRENT_TIMESTAMP
                 WHERE entry_id = ?1 AND run_id = ?2 AND state = 'planned'
                   AND source_relative_path = ?5
                   AND destination_relative_path = ?6
                   AND byte_length = ?7",
                params![
                    file.entry_id,
                    run_id,
                    file.source_blake3,
                    file.destination_blake3,
                    file.source_relative_path,
                    file.destination_relative_path,
                    file.byte_length,
                ],
            )?;
            if updated != 1 {
                return Ok(false);
            }
        }
        let (planned_count, committed_count, committed_bytes): (u64, u64, u64) = transaction
            .query_row(
                "SELECT
                    COUNT(*),
                    SUM(CASE WHEN state = 'committed' THEN 1 ELSE 0 END),
                    COALESCE(SUM(CASE WHEN state = 'committed' THEN byte_length ELSE 0 END), 0)
                 FROM ingest_file WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if files.len() as u64 != expected_file_count
            || planned_count != expected_file_count
            || committed_count != expected_file_count
            || committed_bytes != expected_bytes
        {
            return Ok(false);
        }
        let inserted = transaction.execute(
            "INSERT INTO ingest_receipt(
                run_id, manifest_algorithm, manifest_root_blake3,
                verified_file_count, verified_bytes
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                run_id,
                manifest_algorithm,
                manifest_root_blake3,
                expected_file_count,
                expected_bytes,
            ],
        )?;
        if inserted != 1 {
            return Ok(false);
        }
        transaction.execute(
            "UPDATE ingest_run SET state = 'completed', updated_at = CURRENT_TIMESTAMP
             WHERE run_id = ?1 AND state = 'copying'",
            params![run_id],
        )?;
        transaction.execute(
            "INSERT INTO state_event(run_id, previous_state, next_state, reason)
             VALUES (?1, 'copying', 'completed', 'all verified files persisted and receipt sealed')",
            params![run_id],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    #[cfg(test)]
    fn receipt_seal_count(&self, run_id: &str) -> rusqlite::Result<u64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM ingest_receipt WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
    }

    pub fn save_reader_slot_calibration(
        &mut self,
        reader_fingerprint: &str,
        logical_unit: u8,
        slot_kind: ReaderSlotKind,
        evidence_note: &str,
    ) -> rusqlite::Result<()> {
        self.connection.execute(
            "INSERT INTO reader_slot_calibration(reader_fingerprint, logical_unit, slot_kind, evidence_note)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(reader_fingerprint, logical_unit) DO UPDATE SET
               slot_kind = excluded.slot_kind,
               evidence_note = excluded.evidence_note,
               calibrated_at = CURRENT_TIMESTAMP",
            params![
                reader_fingerprint,
                logical_unit,
                reader_slot_kind_name(slot_kind),
                evidence_note,
            ],
        )?;
        Ok(())
    }

    pub fn reader_slot_kind(
        &self,
        reader_fingerprint: &str,
        logical_unit: u8,
    ) -> rusqlite::Result<Option<ReaderSlotKind>> {
        self.connection
            .query_row(
                "SELECT slot_kind FROM reader_slot_calibration
                 WHERE reader_fingerprint = ?1 AND logical_unit = ?2",
                params![reader_fingerprint, logical_unit],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|value| value.and_then(|value| reader_slot_kind_from_name(&value)))
    }
}

pub fn can_transition_ingest_run(from: IngestRunState, to: IngestRunState) -> bool {
    matches!(
        (from, to),
        (IngestRunState::Queued, IngestRunState::Copying)
            | (IngestRunState::Queued, IngestRunState::Failed)
            | (IngestRunState::Copying, IngestRunState::Completed)
            | (IngestRunState::Copying, IngestRunState::Failed)
            | (IngestRunState::RecoveryRequired, IngestRunState::Copying)
            | (IngestRunState::RecoveryRequired, IngestRunState::Failed)
    )
}

fn ingest_run_state_name(state: IngestRunState) -> &'static str {
    match state {
        IngestRunState::Queued => "queued",
        IngestRunState::Copying => "copying",
        IngestRunState::RecoveryRequired => "recovery_required",
        IngestRunState::Completed => "completed",
        IngestRunState::Failed => "failed",
    }
}

fn ingest_run_state_from_name(value: &str) -> Option<IngestRunState> {
    match value {
        "queued" => Some(IngestRunState::Queued),
        "copying" => Some(IngestRunState::Copying),
        "recovery_required" => Some(IngestRunState::RecoveryRequired),
        "completed" => Some(IngestRunState::Completed),
        "failed" => Some(IngestRunState::Failed),
        _ => None,
    }
}

fn reader_slot_kind_name(value: ReaderSlotKind) -> &'static str {
    match value {
        ReaderSlotKind::Sd => "sd",
        ReaderSlotKind::MicroSd => "micro_sd",
    }
}

fn reader_slot_kind_from_name(value: &str) -> Option<ReaderSlotKind> {
    match value {
        "sd" => Some(ReaderSlotKind::Sd),
        "micro_sd" => Some(ReaderSlotKind::MicroSd),
        _ => None,
    }
}

fn upsert_identity(
    transaction: &Transaction<'_>,
    identity: &SourceIdentityRecord,
) -> rusqlite::Result<()> {
    transaction.execute(
        "
        INSERT INTO source_identity(identity_key, source, normalized_value, strength)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(identity_key) DO UPDATE SET
          last_seen_at = CURRENT_TIMESTAMP
        ",
        params![
            identity.identity_key,
            identity.source,
            identity.normalized_value,
            identity_strength_name(identity.strength),
        ],
    )?;
    Ok(())
}

fn allows_persistent_destination(strength: IdentityStrength) -> bool {
    matches!(
        strength,
        IdentityStrength::HardwareStrong | IdentityStrength::HardwareReported
    )
}

fn identity_strength_name(strength: IdentityStrength) -> &'static str {
    match strength {
        IdentityStrength::HardwareStrong => "hardware_strong",
        IdentityStrength::HardwareReported => "hardware_reported",
        IdentityStrength::Filesystem => "filesystem",
        IdentityStrength::Topology => "topology",
        IdentityStrength::Session => "session",
        IdentityStrength::Ambiguous => "ambiguous",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(key: &str, strength: IdentityStrength) -> SourceIdentityRecord {
        SourceIdentityRecord {
            identity_key: key.into(),
            source: "vpd.naa".into(),
            normalized_value: key.into(),
            strength,
        }
    }

    #[test]
    fn exact_hardware_identity_recalls_but_near_match_does_not() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        assert!(store
            .set_primary_destination(&source, "D:/Ingest/A")
            .expect("profile"));
        assert_eq!(
            store.primary_destination("v1:card-123").expect("query"),
            Some("D:/Ingest/A".into())
        );
        assert_eq!(
            store.primary_destination("v1:card-1234").expect("query"),
            None
        );
    }

    #[test]
    fn filesystem_identity_cannot_create_a_silent_destination_profile() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:formatted-volume", IdentityStrength::Filesystem);
        assert!(!store
            .set_primary_destination(&source, "D:/Ingest/A")
            .expect("profile"));
        assert_eq!(
            store
                .primary_destination(&source.identity_key)
                .expect("query"),
            None
        );
    }

    #[test]
    fn marker_profile_is_exact_and_preserves_auto_ingest_interval() {
        let mut store = LocalStore::in_memory().expect("store");
        let profile = MarkerIngestProfile {
            marker_token: "MIT1:00000000-0000-4000-8000-000000000000".into(),
            destination_path: "D:/Ingest/A".into(),
            sort_mode: "camera_interval".into(),
            interval_minutes: Some(1),
            auto_ingest_enabled: true,
            auto_format_enabled: false,
        };
        store.save_marker_ingest_profile(&profile).expect("save");
        assert_eq!(
            store
                .marker_ingest_profile(&profile.marker_token)
                .expect("load"),
            Some(profile)
        );
        assert_eq!(
            store
                .marker_ingest_profile("MIT1:00000000-0000-4000-8000-000000000001")
                .expect("near"),
            None
        );
    }

    #[test]
    fn completed_auto_ingest_is_scoped_to_the_exact_insertion_generation() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-auto", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run_with_mode("auto-run", &source, 7, "/source", "/destination", true)
            .expect("begin automatic run");
        assert!(store
            .transition_ingest_run("auto-run", IngestRunState::Copying, "worker started")
            .expect("copying"));
        assert!(store
            .transition_ingest_run("auto-run", IngestRunState::Completed, "receipt sealed")
            .expect("completed"));
        store
            .connection
            .execute(
                "INSERT INTO ingest_receipt(run_id, manifest_algorithm, manifest_root_blake3, verified_file_count, verified_bytes)
                 VALUES ('auto-run', 'blake3:test', 'root', 1, 1)",
                [],
            )
            .expect("receipt");

        assert!(store
            .has_completed_auto_ingest_for_source("v1:card-auto", 7)
            .expect("same generation"));
        assert!(store
            .begin_ingest_run_with_mode(
                "duplicate-auto-run",
                &source,
                7,
                "/source",
                "/destination",
                true,
            )
            .is_err());
        assert!(!store
            .has_completed_auto_ingest_for_source("v1:card-auto", 8)
            .expect("next insertion remains eligible"));
        assert!(store
            .begin_ingest_run_with_mode(
                "next-insertion-auto-run",
                &source,
                8,
                "/source",
                "/destination",
                true,
            )
            .is_ok());
    }

    #[test]
    fn completed_format_is_available_only_as_a_source_lookup() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:formatted-card", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run_with_mode(
                "formatted-run",
                &source,
                7,
                "/source",
                "/destination",
                true,
            )
            .expect("begin");
        assert!(store
            .transition_ingest_run("formatted-run", IngestRunState::Copying, "worker started")
            .expect("copying"));
        assert!(store
            .transition_ingest_run("formatted-run", IngestRunState::Completed, "receipt sealed")
            .expect("completed"));
        store
            .connection
            .execute(
                "INSERT INTO ingest_receipt(run_id, manifest_algorithm, manifest_root_blake3, verified_file_count, verified_bytes)
                 VALUES ('formatted-run', 'blake3:test', 'root', 1, 1)",
                [],
            )
            .expect("receipt");
        assert!(store
            .record_completed_format("formatted-run", "v1:formatted-card", 7, "sdxc-default")
            .expect("format receipt"));
        assert!(store
            .has_completed_format_for_source("v1:formatted-card")
            .expect("lookup"));
        assert!(!store
            .has_completed_format_for_source("v1:other-card")
            .expect("lookup"));
    }

    #[test]
    fn latest_source_generations_returns_the_newest_generation_per_identity() {
        let mut store = LocalStore::in_memory().expect("store");
        let first = identity("v1:first", IdentityStrength::HardwareStrong);
        let second = identity("v1:second", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("first-old", &first, 2, "/source", "/destination")
            .expect("first old");
        store
            .begin_ingest_run("first-new", &first, 5, "/source", "/destination")
            .expect("first new");
        store
            .begin_ingest_run("second", &second, 3, "/source", "/destination")
            .expect("second");

        let generations = store.latest_source_generations().expect("generations");
        assert_eq!(
            generations,
            vec![("v1:first".into(), 5), ("v1:second".into(), 3)]
        );
    }

    #[test]
    fn newly_migrated_database_passes_integrity_check() {
        let store = LocalStore::in_memory().expect("store");
        assert!(store.integrity_check().expect("integrity"));
    }

    #[test]
    fn run_transitions_are_monotonic_and_audited() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-1", &source, 9, "/source", "/destination")
            .expect("begin run");
        assert!(store
            .transition_ingest_run("run-1", IngestRunState::Copying, "worker started")
            .expect("copying"));
        assert!(store
            .transition_ingest_run("run-1", IngestRunState::Completed, "receipt sealed")
            .expect("complete"));
        assert!(!store
            .transition_ingest_run("run-1", IngestRunState::Copying, "illegal retry")
            .expect("rejected"));
        let events: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM state_event WHERE run_id = 'run-1'",
                [],
                |row| row.get(0),
            )
            .expect("events");
        assert_eq!(events, 3);
    }

    #[test]
    fn history_projection_exposes_only_operator_safe_run_summary() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-history", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-history", &source, 3, "/source", "/destination")
            .expect("begin run");
        store
            .transition_ingest_run("run-history", IngestRunState::Copying, "worker started")
            .expect("copying");
        store
            .transition_ingest_run("run-history", IngestRunState::Completed, "complete")
            .expect("complete");
        let history = store.recent_ingest_runs(20).expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].run_id, "run-history");
        assert_eq!(history[0].source_identity_key, "v1:card-history");
        assert_eq!(history[0].source_generation, 3);
        assert_eq!(history[0].state, "completed");
        assert_eq!(history[0].verified_file_count, 0);
        assert_eq!(history[0].verified_bytes, 0);
        assert!(!history[0].receipt_available);
    }

    #[test]
    fn slot_calibration_requires_the_exact_reader_fingerprint_and_lun() {
        let mut store = LocalStore::in_memory().expect("store");
        store
            .save_reader_slot_calibration(
                "reader:v1:abc",
                1,
                ReaderSlotKind::MicroSd,
                "controlled microSD insertion",
            )
            .expect("calibrate");
        assert_eq!(
            store.reader_slot_kind("reader:v1:abc", 1).expect("lookup"),
            Some(ReaderSlotKind::MicroSd)
        );
        assert_eq!(
            store.reader_slot_kind("reader:v1:abc", 0).expect("lookup"),
            None
        );
        assert_eq!(
            store
                .reader_slot_kind("reader:v1:other", 1)
                .expect("lookup"),
            None
        );
    }

    #[test]
    fn verified_files_are_refused_after_a_terminal_transition() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-1", &source, 1, "/source", "/destination")
            .expect("begin run");
        let file = VerifiedFileRecord {
            entry_id: "entry-1".into(),
            source_relative_path: "DCIM/clip.mov".into(),
            destination_relative_path: "camera/clip.mov".into(),
            byte_length: 42,
            source_blake3: "source-hash".into(),
            destination_blake3: "destination-hash".into(),
        };
        let planned = PlannedFileRecord {
            entry_id: file.entry_id.clone(),
            source_relative_path: file.source_relative_path.clone(),
            destination_relative_path: file.destination_relative_path.clone(),
            byte_length: file.byte_length,
        };
        assert!(store
            .record_planned_file("run-1", &planned)
            .expect("plan record"));
        assert_eq!(store.planned_file_count("run-1").expect("count"), 1);
        assert!(!store
            .record_verified_file("run-1", &file)
            .expect("not copying yet"));
        assert!(store
            .transition_ingest_run("run-1", IngestRunState::Copying, "worker started")
            .expect("copying"));
        let mismatched = VerifiedFileRecord {
            destination_relative_path: "other/clip.mov".into(),
            ..file.clone()
        };
        assert!(!store
            .record_verified_file("run-1", &mismatched)
            .expect("mismatched plan rejected"));
        assert!(store.record_verified_file("run-1", &file).expect("record"));
        assert_eq!(store.verified_file_count("run-1").expect("count"), 1);
        assert!(store
            .transition_ingest_run("run-1", IngestRunState::Completed, "receipt sealed")
            .expect("complete"));
        assert!(!store.record_verified_file("run-1", &file).expect("refused"));
    }

    #[test]
    fn receipt_seal_requires_the_complete_committed_plan_and_is_immutable() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-1", &source, 1, "/source", "/destination")
            .expect("begin run");
        let planned = PlannedFileRecord {
            entry_id: "entry-1".into(),
            source_relative_path: "DCIM/clip.mov".into(),
            destination_relative_path: "camera/clip.mov".into(),
            byte_length: 42,
        };
        assert!(store.record_planned_file("run-1", &planned).expect("plan"));
        assert!(store
            .transition_ingest_run("run-1", IngestRunState::Copying, "worker started")
            .expect("copying"));
        assert!(!store
            .seal_ingest_receipt("run-1", "blake3:test", "root", 1, 42)
            .expect("must wait for verification"));
        assert!(store
            .record_verified_file(
                "run-1",
                &VerifiedFileRecord {
                    entry_id: "entry-1".into(),
                    source_relative_path: planned.source_relative_path.clone(),
                    destination_relative_path: planned.destination_relative_path.clone(),
                    byte_length: planned.byte_length,
                    source_blake3: "source".into(),
                    destination_blake3: "destination".into(),
                },
            )
            .expect("verified"));
        assert!(store
            .seal_ingest_receipt("run-1", "blake3:test", "root", 1, 42)
            .expect("sealed"));
        assert_eq!(store.receipt_seal_count("run-1").expect("count"), 1);
        assert!(!store
            .seal_ingest_receipt("run-1", "blake3:test", "root", 1, 42)
            .expect("duplicate rejected"));
    }

    #[test]
    fn atomic_completion_rolls_back_every_file_when_any_plan_entry_mismatches() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-atomic", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-atomic", &source, 1, "/source", "/destination")
            .expect("begin run");
        let planned = [
            PlannedFileRecord {
                entry_id: "entry-one".into(),
                source_relative_path: "DCIM/one.mov".into(),
                destination_relative_path: "camera/one.mov".into(),
                byte_length: 5,
            },
            PlannedFileRecord {
                entry_id: "entry-two".into(),
                source_relative_path: "DCIM/two.mov".into(),
                destination_relative_path: "camera/two.mov".into(),
                byte_length: 7,
            },
        ];
        for file in &planned {
            assert!(store
                .record_planned_file("run-atomic", file)
                .expect("planned"));
        }
        assert!(store
            .transition_ingest_run("run-atomic", IngestRunState::Copying, "worker started")
            .expect("copying"));
        let files = [
            VerifiedFileRecord {
                entry_id: planned[0].entry_id.clone(),
                source_relative_path: planned[0].source_relative_path.clone(),
                destination_relative_path: planned[0].destination_relative_path.clone(),
                byte_length: planned[0].byte_length,
                source_blake3: "one".into(),
                destination_blake3: "one".into(),
            },
            VerifiedFileRecord {
                entry_id: planned[1].entry_id.clone(),
                source_relative_path: planned[1].source_relative_path.clone(),
                destination_relative_path: "camera/replaced.mov".into(),
                byte_length: planned[1].byte_length,
                source_blake3: "two".into(),
                destination_blake3: "two".into(),
            },
        ];
        assert!(!store
            .commit_verified_completion("run-atomic", &files, "blake3:test", "root", 2, 12)
            .expect("mismatch rejected"));
        assert_eq!(store.verified_file_count("run-atomic").expect("count"), 0);
        assert_eq!(store.receipt_seal_count("run-atomic").expect("receipt"), 0);
        let state: String = store
            .connection
            .query_row(
                "SELECT state FROM ingest_run WHERE run_id = 'run-atomic'",
                [],
                |row| row.get(0),
            )
            .expect("state");
        assert_eq!(state, "copying");
    }

    #[test]
    fn completed_receipt_is_bound_to_the_exact_source_and_generation() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("run-bound", &source, 4, "/source", "/destination")
            .expect("begin run");
        store
            .record_planned_file(
                "run-bound",
                &PlannedFileRecord {
                    entry_id: "entry-1".into(),
                    source_relative_path: "clip.mov".into(),
                    destination_relative_path: "clip.mov".into(),
                    byte_length: 8,
                },
            )
            .expect("plan");
        store
            .transition_ingest_run("run-bound", IngestRunState::Copying, "copying")
            .expect("copying");
        store
            .record_verified_file(
                "run-bound",
                &VerifiedFileRecord {
                    entry_id: "entry-1".into(),
                    source_relative_path: "clip.mov".into(),
                    destination_relative_path: "clip.mov".into(),
                    byte_length: 8,
                    source_blake3: "source".into(),
                    destination_blake3: "destination".into(),
                },
            )
            .expect("verified");
        assert!(store
            .seal_ingest_receipt("run-bound", "blake3:test", "root", 1, 8)
            .expect("seal"));
        assert!(store
            .transition_ingest_run("run-bound", IngestRunState::Completed, "completed")
            .expect("completed"));
        assert!(store
            .has_completed_receipt_for_source("run-bound", "v1:card-123", 4)
            .expect("matching receipt"));
        assert!(!store
            .has_completed_receipt_for_source("run-bound", "v1:card-123", 5)
            .expect("wrong generation"));
        assert!(!store
            .has_completed_receipt_for_source("run-bound", "v1:other-card", 4)
            .expect("wrong source"));
        assert!(!store
            .record_completed_format("run-bound", "v1:other-card", 4, "sdxc-default")
            .expect("wrong format source rejected"));
        assert!(store
            .record_completed_format("run-bound", "v1:card-123", 4, "sdxc-default")
            .expect("format receipt"));
        assert!(!store
            .record_completed_format("run-bound", "v1:card-123", 4, "sdxc-default")
            .expect("format receipt remains immutable"));
    }

    #[test]
    fn startup_reconciliation_preserves_only_runs_with_recovery_locations() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:card-123", IdentityStrength::HardwareStrong);
        store
            .begin_ingest_run("queued", &source, 1, "/source", "/destination")
            .expect("queued");
        store
            .begin_ingest_run("copying", &source, 2, "/source", "/destination")
            .expect("copying");
        assert!(store
            .transition_ingest_run("copying", IngestRunState::Copying, "worker started")
            .expect("transition"));
        store
            .begin_ingest_run("complete", &source, 3, "/source", "/destination")
            .expect("complete");
        store
            .begin_ingest_run("legacy", &source, 4, "", "")
            .expect("legacy");
        assert!(store
            .transition_ingest_run("complete", IngestRunState::Copying, "worker started")
            .expect("transition"));
        assert!(store
            .transition_ingest_run("complete", IngestRunState::Completed, "sealed")
            .expect("transition"));
        assert_eq!(store.reconcile_interrupted_runs().expect("reconcile"), 3);
        assert_eq!(store.reconcile_interrupted_runs().expect("idempotent"), 0);
        let states = ["queued", "copying", "complete", "legacy"].map(|run_id| {
            store
                .connection
                .query_row(
                    "SELECT state FROM ingest_run WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("state")
        });
        assert_eq!(
            states,
            [
                "recovery_required",
                "recovery_required",
                "completed",
                "failed"
            ]
        );
        assert_eq!(
            store
                .recoverable_ingest_run("queued")
                .expect("query")
                .expect("candidate"),
            RecoverableIngestRun {
                run_id: "queued".into(),
                source_identity_key: "v1:card-123".into(),
                source_generation: 1,
                source_root: "/source".into(),
                destination_root: "/destination".into(),
                auto_ingest_triggered: false,
            }
        );
        assert!(store
            .begin_explicit_recovery("queued", 2)
            .expect("recovery transition"));
        assert!(!store
            .begin_explicit_recovery("queued", 2)
            .expect("single recovery transition"));
        assert!(store
            .return_to_recovery_required("queued", "destination temporarily unavailable")
            .expect("return to recovery"));
        assert!(store
            .begin_explicit_recovery("queued", 3)
            .expect("retry recovery transition"));
    }

    #[test]
    fn recovery_record_retains_the_auto_ingest_origin() {
        let mut store = LocalStore::in_memory().expect("store");
        let source = identity("v1:auto-card", IdentityStrength::Filesystem);
        store
            .begin_ingest_run("automatic", &source, 4, "/source", "/destination")
            .expect("run");
        store
            .connection
            .execute(
                "UPDATE ingest_run SET auto_ingest_triggered = 1 WHERE run_id = 'automatic'",
                [],
            )
            .expect("mark automatic");
        store.reconcile_interrupted_runs().expect("reconcile");
        assert!(
            store
                .recoverable_ingest_run("automatic")
                .expect("query")
                .expect("record")
                .auto_ingest_triggered
        );
    }
}

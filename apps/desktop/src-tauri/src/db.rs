use std::path::Path;

use tauri_plugin_sql::{Migration, MigrationKind};

/// Pre-migration runtime patches. Currently only sweeps stale `recording`
/// sessions left by a prior crash; runtime *schema* patches (segments.speaker_id)
/// live in the frontend's `getDb()` so they run after migrations on fresh installs.
pub fn ensure_runtime_schema(db_path: &Path) {
    use rusqlite::Connection;

    if !db_path.exists() {
        return;
    }

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "ensure_runtime_schema: open({}) failed: {e}; skipping",
                db_path.display()
            );
            return;
        }
    };

    if !table_exists(&conn, "segments") {
        return;
    }

    close_orphaned_recordings(&conn);
}

/// At startup the app cannot have a real in-flight recording session, so any
/// row left at status='recording' is from a prior crash or force-quit. Empty
/// ones (no segments, no audio parts) are deleted; the rest are marked
/// completed with duration recomputed from their parts (or, as a final
/// fallback, from their segments' max offset).
fn close_orphaned_recordings(conn: &rusqlite::Connection) {
    // Newly-installed databases may not yet have the parts table; gate on
    // its presence so this sweep is forward- and backward-compatible.
    let has_parts_table = table_exists(conn, "session_audio_parts");

    let deleted = if has_parts_table {
        conn.execute(
            "DELETE FROM sessions \
             WHERE status = 'recording' \
               AND NOT EXISTS (SELECT 1 FROM segments WHERE session_id = sessions.id) \
               AND NOT EXISTS (SELECT 1 FROM session_audio_parts WHERE session_id = sessions.id)",
            [],
        )
    } else {
        conn.execute(
            "DELETE FROM sessions \
             WHERE status = 'recording' \
               AND NOT EXISTS (SELECT 1 FROM segments WHERE session_id = sessions.id)",
            [],
        )
    }
    .unwrap_or_else(|e| {
        tracing::warn!("close_orphaned_recordings: delete failed: {e}");
        0
    });

    let completed = if has_parts_table {
        conn.execute(
            "UPDATE sessions SET \
                status = 'completed', \
                total_segments = (SELECT COUNT(*) FROM segments WHERE session_id = sessions.id), \
                duration_seconds = COALESCE( \
                    (SELECT SUM(duration_seconds) FROM session_audio_parts WHERE session_id = sessions.id), \
                    (SELECT MAX(audio_offset_seconds + chunk_duration_seconds) \
                     FROM segments WHERE session_id = sessions.id), \
                    duration_seconds \
                ), \
                updated_at = datetime('now') \
             WHERE status = 'recording'",
            [],
        )
    } else {
        conn.execute(
            "UPDATE sessions SET \
                status = 'completed', \
                total_segments = (SELECT COUNT(*) FROM segments WHERE session_id = sessions.id), \
                duration_seconds = COALESCE( \
                    (SELECT MAX(audio_offset_seconds + chunk_duration_seconds) \
                     FROM segments WHERE session_id = sessions.id), \
                    duration_seconds \
                ), \
                updated_at = datetime('now') \
             WHERE status = 'recording'",
            [],
        )
    }
    .unwrap_or_else(|e| {
        tracing::warn!("close_orphaned_recordings: update failed: {e}");
        0
    });

    if deleted > 0 || completed > 0 {
        tracing::info!(
            "close_orphaned_recordings: deleted {deleted} empty, completed {completed} stale"
        );
    }
}

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

pub fn migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "create sessions and segments tables",
            sql: r#"
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                source TEXT NOT NULL DEFAULT 'Mixed',
                status TEXT NOT NULL DEFAULT 'recording',
                duration_seconds REAL,
                total_segments INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE segments (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                text TEXT NOT NULL,
                audio_offset_seconds REAL NOT NULL,
                chunk_duration_seconds REAL NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                chunk_index INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX idx_segments_session_id ON segments(session_id);
            CREATE INDEX idx_segments_offset ON segments(session_id, audio_offset_seconds);
            CREATE INDEX idx_sessions_created ON sessions(created_at DESC);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "add folders, pins, and folder_id to sessions",
            sql: r#"
            CREATE TABLE folders (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                parent_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_folders_parent ON folders(parent_id);

            ALTER TABLE sessions ADD COLUMN folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL;
            ALTER TABLE sessions ADD COLUMN is_pinned INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE sessions ADD COLUMN pinned_at TEXT;
            CREATE INDEX idx_sessions_folder ON sessions(folder_id);
            CREATE INDEX idx_sessions_pinned ON sessions(is_pinned, pinned_at DESC);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "add segment editing columns",
            sql: r#"
            ALTER TABLE segments ADD COLUMN original_text TEXT;
            ALTER TABLE segments ADD COLUMN edited_at TEXT;
            ALTER TABLE segments ADD COLUMN deleted_at TEXT;
            ALTER TABLE segments ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "add notes, note_versions, and session_type",
            sql: r#"
            CREATE TABLE notes (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE TABLE note_versions (
                id TEXT PRIMARY KEY,
                note_id TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
            );
            CREATE INDEX idx_note_versions_note ON note_versions(note_id, created_at DESC);

            ALTER TABLE sessions ADD COLUMN session_type TEXT NOT NULL DEFAULT 'transcription';
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "add wav file path and duration to sessions",
            sql: r#"
            ALTER TABLE sessions ADD COLUMN wav_file_path TEXT;
            ALTER TABLE sessions ADD COLUMN wav_duration_seconds REAL;
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 6,
            description: "add sort_order to sessions and shares table",
            sql: r#"
            ALTER TABLE sessions ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;

            CREATE TABLE shares (
                id TEXT PRIMARY KEY,
                folder_id TEXT NOT NULL,
                shared_with_email TEXT,
                permission TEXT NOT NULL DEFAULT 'viewer',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT,
                FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );
            CREATE INDEX idx_shares_folder ON shares(folder_id);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 7,
            description: "add chat_messages table for AI chat persistence",
            sql: r#"
            CREATE TABLE chat_messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                action TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            CREATE INDEX idx_chat_messages_session ON chat_messages(session_id, created_at ASC);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 8,
            description: "add context_key to chat_messages, make session_id nullable",
            sql: r#"
            CREATE TABLE chat_messages_new (
                id TEXT PRIMARY KEY,
                context_key TEXT NOT NULL,
                session_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                action TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            INSERT INTO chat_messages_new (id, context_key, session_id, role, content, action, created_at)
            SELECT id, session_id, session_id, role, content, action, created_at FROM chat_messages;
            DROP TABLE chat_messages;
            ALTER TABLE chat_messages_new RENAME TO chat_messages;
            CREATE INDEX idx_chat_messages_context ON chat_messages(context_key, created_at ASC);
            CREATE INDEX idx_chat_messages_session ON chat_messages(session_id);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 9,
            description: "add folder metadata and session_folders junction table",
            sql: r#"
            ALTER TABLE folders ADD COLUMN icon TEXT;
            ALTER TABLE folders ADD COLUMN color TEXT;
            ALTER TABLE folders ADD COLUMN description TEXT;

            CREATE TABLE session_folders (
                session_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (session_id, folder_id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );
            CREATE INDEX idx_session_folders_folder ON session_folders(folder_id);
            CREATE INDEX idx_session_folders_session ON session_folders(session_id);

            INSERT INTO session_folders (session_id, folder_id)
            SELECT id, folder_id FROM sessions WHERE folder_id IS NOT NULL;
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 10,
            description: "add dictation_history table",
            sql: r#"
            CREATE TABLE dictation_history (
                id TEXT PRIMARY KEY,
                slot_id TEXT NOT NULL,
                slot_name TEXT NOT NULL,
                input_text TEXT NOT NULL,
                output_text TEXT NOT NULL,
                ai_enabled INTEGER NOT NULL DEFAULT 0,
                ai_prompt TEXT,
                output_action TEXT NOT NULL,
                wav_file_path TEXT,
                wav_duration_seconds REAL,
                session_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_dictation_history_created ON dictation_history(created_at DESC);
            CREATE INDEX idx_dictation_history_slot ON dictation_history(slot_id);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 11,
            description: "add tags and session_tags tables",
            sql: r#"
            CREATE TABLE tags (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                color TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_tags_name ON tags(name);

            CREATE TABLE session_tags (
                session_id TEXT NOT NULL,
                tag_id TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'manual',
                confidence REAL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (session_id, tag_id),
                FOREIGN KEY (session_id) REFERENCES sessions(id) ON DELETE CASCADE,
                FOREIGN KEY (tag_id) REFERENCES tags(id) ON DELETE CASCADE
            );
            CREATE INDEX idx_session_tags_tag ON session_tags(tag_id);
            CREATE INDEX idx_session_tags_session ON session_tags(session_id);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 12,
            description: "add FTS5 search tables for segments, notes, sessions, dictation_history",
            sql: r#"
            -- Contentless FTS5 tables that own their searchable text plus the
            -- source-row TEXT primary key as UNINDEXED columns. Search queries
            -- return the PK directly; no rowid mapping needed for our UUID PKs.

            CREATE VIRTUAL TABLE segments_fts USING fts5(
                segment_id UNINDEXED,
                session_id UNINDEXED,
                text,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );

            CREATE VIRTUAL TABLE notes_fts USING fts5(
                note_id UNINDEXED,
                session_id UNINDEXED,
                content,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );

            CREATE VIRTUAL TABLE sessions_fts USING fts5(
                session_id UNINDEXED,
                title,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );

            CREATE VIRTUAL TABLE dictations_fts USING fts5(
                dictation_id UNINDEXED,
                output_text,
                input_text,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );

            -- Backfill from existing rows. segments_fts skips soft-deleted rows.
            INSERT INTO segments_fts (segment_id, session_id, text)
                SELECT id, session_id, text FROM segments WHERE deleted_at IS NULL;

            INSERT INTO notes_fts (note_id, session_id, content)
                SELECT id, session_id, content FROM notes;

            INSERT INTO sessions_fts (session_id, title)
                SELECT id, title FROM sessions;

            INSERT INTO dictations_fts (dictation_id, output_text, input_text)
                SELECT id, output_text, input_text FROM dictation_history;

            -- Triggers: keep FTS in sync with source tables.

            CREATE TRIGGER segments_ai AFTER INSERT ON segments
            WHEN new.deleted_at IS NULL
            BEGIN
                INSERT INTO segments_fts (segment_id, session_id, text)
                    VALUES (new.id, new.session_id, new.text);
            END;

            CREATE TRIGGER segments_ad AFTER DELETE ON segments BEGIN
                DELETE FROM segments_fts WHERE segment_id = old.id;
            END;

            CREATE TRIGGER segments_au AFTER UPDATE ON segments BEGIN
                DELETE FROM segments_fts WHERE segment_id = old.id;
                INSERT INTO segments_fts (segment_id, session_id, text)
                    SELECT new.id, new.session_id, new.text WHERE new.deleted_at IS NULL;
            END;

            CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN
                INSERT INTO notes_fts (note_id, session_id, content)
                    VALUES (new.id, new.session_id, new.content);
            END;

            CREATE TRIGGER notes_ad AFTER DELETE ON notes BEGIN
                DELETE FROM notes_fts WHERE note_id = old.id;
            END;

            CREATE TRIGGER notes_au AFTER UPDATE ON notes BEGIN
                DELETE FROM notes_fts WHERE note_id = old.id;
                INSERT INTO notes_fts (note_id, session_id, content)
                    VALUES (new.id, new.session_id, new.content);
            END;

            CREATE TRIGGER sessions_ai AFTER INSERT ON sessions BEGIN
                INSERT INTO sessions_fts (session_id, title)
                    VALUES (new.id, new.title);
            END;

            CREATE TRIGGER sessions_ad AFTER DELETE ON sessions BEGIN
                DELETE FROM sessions_fts WHERE session_id = old.id;
            END;

            CREATE TRIGGER sessions_au AFTER UPDATE OF title ON sessions BEGIN
                DELETE FROM sessions_fts WHERE session_id = old.id;
                INSERT INTO sessions_fts (session_id, title)
                    VALUES (new.id, new.title);
            END;

            CREATE TRIGGER dictations_ai AFTER INSERT ON dictation_history BEGIN
                INSERT INTO dictations_fts (dictation_id, output_text, input_text)
                    VALUES (new.id, new.output_text, new.input_text);
            END;

            CREATE TRIGGER dictations_ad AFTER DELETE ON dictation_history BEGIN
                DELETE FROM dictations_fts WHERE dictation_id = old.id;
            END;

            CREATE TRIGGER dictations_au AFTER UPDATE ON dictation_history BEGIN
                DELETE FROM dictations_fts WHERE dictation_id = old.id;
                INSERT INTO dictations_fts (dictation_id, output_text, input_text)
                    VALUES (new.id, new.output_text, new.input_text);
            END;
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 13,
            description: "add tool_calls JSON column to chat_messages",
            sql: r#"
            ALTER TABLE chat_messages ADD COLUMN tool_calls TEXT;
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 14,
            description: "per-LLM-response chat_messages rows for tool replay",
            // New columns:
            //   send_id      groups all rows derived from one user send
            //   sequence     ordering within send_id (0=user, 1+ = subsequent)
            //   tool_call_id required on role='tool', references the parent
            //                assistant row's tool_calls[].id
            //   observation  structured ToolObservation JSON, role='tool' only
            //   status       'done' | 'error', role='tool' only
            //
            // SQLite can't relax the implicit role check in-place, but the
            // CHECK was never declared explicitly — `role` is just TEXT, so
            // 'tool' is already accepted by the column type. No CHECK changes
            // needed.
            //
            // `content` was NOT NULL in the original CREATE TABLE. SQLite
            // can't drop NOT NULL via ALTER, so we keep writes valid by
            // storing an empty string for assistant rows that only emit
            // tool_calls. The TS layer treats `content === ""` as "no prose".
            //
            // Backfill: every existing row is its own send (one-row sends),
            // so set send_id = id, sequence = 0. Pre-v14 tool memory is left
            // dormant in the legacy `tool_calls` JSON column for read-side
            // soft fallback.
            sql: r#"
            ALTER TABLE chat_messages ADD COLUMN send_id TEXT;
            ALTER TABLE chat_messages ADD COLUMN sequence INTEGER;
            ALTER TABLE chat_messages ADD COLUMN tool_call_id TEXT;
            ALTER TABLE chat_messages ADD COLUMN observation TEXT;
            ALTER TABLE chat_messages ADD COLUMN status TEXT;
            UPDATE chat_messages SET send_id = id WHERE send_id IS NULL;
            UPDATE chat_messages SET sequence = 0 WHERE sequence IS NULL;
            CREATE INDEX IF NOT EXISTS idx_chat_messages_send
                ON chat_messages(context_key, send_id, sequence);
        "#,
            kind: MigrationKind::Up,
        },
        Migration {
            version: 15,
            description: "session_audio_parts table + backfill from sessions.wav_file_path",
            // Each recording run produces one part. A session's audio is the
            // ordered concatenation of its parts. Resume = append a new part,
            // never mutate prior parts.
            //
            // Backfill: every existing session with a wav_file_path becomes
            // part_index=0 in the new table. Sample rate isn't stored on the
            // legacy row; default to 48000 (only used for diagnostics).
            //
            // We deliberately do NOT drop the legacy `wav_file_path` /
            // `wav_duration_seconds` columns from `sessions` here — combining
            // ALTER TABLE DROP COLUMN with the backfill INSERT in the same
            // tauri-plugin-sql migration transaction has been seen to fail
            // on some SQLite builds, leaving the DB in a dead state. The
            // columns are unused at the TypeScript boundary; they stay
            // dormant until a future cleanup migration drops them.
            //
            // The migration is idempotent so it can re-run cleanly:
            //   - CREATE TABLE IF NOT EXISTS
            //   - INSERT OR IGNORE (the (session_id, part_index) UNIQUE
            //     constraint short-circuits duplicates).
            sql: r#"
            CREATE TABLE IF NOT EXISTS session_audio_parts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                part_index INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                format TEXT NOT NULL CHECK (format IN ('wav','mp3')),
                duration_seconds REAL NOT NULL,
                sample_rate INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE (session_id, part_index)
            );
            CREATE INDEX IF NOT EXISTS idx_audio_parts_session
                ON session_audio_parts(session_id, part_index);

            INSERT OR IGNORE INTO session_audio_parts (
                id, session_id, part_index, file_path, format,
                duration_seconds, sample_rate, created_at
            )
            SELECT
                lower(hex(randomblob(16))),
                id,
                0,
                wav_file_path,
                CASE WHEN wav_file_path LIKE '%.mp3' THEN 'mp3' ELSE 'wav' END,
                COALESCE(wav_duration_seconds, 0),
                48000,
                COALESCE(updated_at, created_at)
            FROM sessions
            WHERE wav_file_path IS NOT NULL;
        "#,
            kind: MigrationKind::Up,
        },
        // segments.speaker_id is added by `ensure_runtime_schema()` instead —
        // see that function for why.
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_not_empty() {
        let m = migrations();
        assert!(!m.is_empty());
    }

    #[test]
    fn test_migrations_sequential_versions() {
        let m = migrations();
        let actual_versions: Vec<i64> = m.iter().map(|x| x.version).collect();
        assert_eq!(actual_versions, (1..=15).collect::<Vec<_>>());
    }

    #[test]
    fn test_migrations_have_descriptions() {
        for m in migrations() {
            assert!(
                !m.description.is_empty(),
                "migration v{} should have a description",
                m.version
            );
        }
    }

    #[test]
    fn test_migrations_sql_not_empty() {
        for m in migrations() {
            let sql = m.sql.trim();
            assert!(
                !sql.is_empty(),
                "migration v{} should have non-empty SQL",
                m.version
            );
        }
    }

    #[test]
    fn test_migrations_sql_contains_expected_keywords() {
        let m = migrations();

        // v1 should create sessions and segments
        assert!(m[0].sql.contains("CREATE TABLE sessions"));
        assert!(m[0].sql.contains("CREATE TABLE segments"));

        // v2 should create folders and alter sessions
        assert!(m[1].sql.contains("CREATE TABLE folders"));
        assert!(m[1].sql.contains("ALTER TABLE sessions"));

        // v4 should create notes
        assert!(m[3].sql.contains("CREATE TABLE notes"));
        assert!(m[3].sql.contains("CREATE TABLE note_versions"));

        // v7 should create chat_messages
        assert!(m[6].sql.contains("CREATE TABLE chat_messages"));

        // v9 should create session_folders junction table
        assert!(m[8].sql.contains("CREATE TABLE session_folders"));

        // v10 should create dictation_history
        assert!(m[9].sql.contains("CREATE TABLE dictation_history"));

        // v11 should create tags and session_tags
        assert!(m[10].sql.contains("CREATE TABLE tags"));
        assert!(m[10].sql.contains("CREATE TABLE session_tags"));
    }

    #[test]
    fn test_all_migrations_are_up() {
        for m in migrations() {
            assert!(
                matches!(m.kind, MigrationKind::Up),
                "migration v{} should be an Up migration",
                m.version
            );
        }
    }

    #[test]
    fn test_migration_count() {
        assert_eq!(
            migrations().len(),
            15,
            "v1-v15; segments.speaker_id handled at runtime via ensure_runtime_schema"
        );
    }

    #[test]
    fn test_migration_v15_creates_session_audio_parts() {
        let m = migrations();
        let v15 = &m[14];
        assert_eq!(v15.version, 15);
        assert!(v15
            .sql
            .contains("CREATE TABLE IF NOT EXISTS session_audio_parts"));
        assert!(v15
            .sql
            .contains("INSERT OR IGNORE INTO session_audio_parts"));
    }

    #[test]
    fn test_migration_v15_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for m in migrations() {
            conn.execute_batch(m.sql)
                .unwrap_or_else(|e| panic!("migration v{} failed: {}", m.version, e));
        }
        conn.execute(
            "INSERT INTO sessions (id, title, source, status, wav_file_path, wav_duration_seconds) \
             VALUES ('s1', 't', 'MicOnly', 'completed', '/tmp/s1.mp3', 12.5)",
            [],
        )
        .unwrap();
        let v15_sql = migrations()[14].sql;
        // Re-applying the migration must not error or double-insert.
        conn.execute_batch(v15_sql).unwrap();
        conn.execute_batch(v15_sql).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_audio_parts WHERE session_id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "re-running v15 must not double-insert parts");
    }

    #[test]
    fn test_migration_v13_adds_tool_calls_column() {
        let m = migrations();
        let v13 = &m[12];
        assert_eq!(v13.version, 13);
        assert!(v13.sql.contains("ALTER TABLE chat_messages"));
        assert!(v13.sql.contains("tool_calls"));
    }

    #[test]
    fn test_all_migrations_execute_against_sqlite() {
        // Run the migration SQL against an in-memory rusqlite to catch
        // syntax errors (e.g. malformed FTS5 declarations) at test time
        // instead of waiting for a user to launch a fresh DB.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for m in migrations() {
            conn.execute_batch(m.sql)
                .unwrap_or_else(|e| panic!("migration v{} failed: {}", m.version, e));
        }
        // Sanity-check that the FTS5 tables are queryable post-migration.
        for fts_table in [
            "segments_fts",
            "notes_fts",
            "sessions_fts",
            "dictations_fts",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {fts_table}"), [], |row| {
                    row.get(0)
                })
                .unwrap_or_else(|e| panic!("query against {fts_table} failed: {e}"));
            assert_eq!(count, 0, "{fts_table} should be empty after migration");
        }
    }

    #[test]
    fn test_migration_v12_creates_fts_tables() {
        let m = migrations();
        let v12 = &m[11];
        assert_eq!(v12.version, 12);
        assert!(v12.sql.contains("CREATE VIRTUAL TABLE segments_fts"));
        assert!(v12.sql.contains("CREATE VIRTUAL TABLE notes_fts"));
        assert!(v12.sql.contains("CREATE VIRTUAL TABLE sessions_fts"));
        assert!(v12.sql.contains("CREATE VIRTUAL TABLE dictations_fts"));
        assert!(v12.sql.contains("USING fts5"));
        assert!(v12.sql.contains("INSERT INTO segments_fts"));
        assert!(v12.sql.contains("CREATE TRIGGER segments_ai"));
        assert!(v12.sql.contains("CREATE TRIGGER segments_ad"));
        assert!(v12.sql.contains("CREATE TRIGGER segments_au"));
    }
}

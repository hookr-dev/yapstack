use std::path::Path;

use tauri_plugin_sql::{Migration, MigrationKind};

/// Apply runtime schema patches that aren't well-served by the sqlx-style
/// migration system. Currently:
///
/// - Adds `segments.speaker_id INTEGER` if absent. Lives outside the
///   migration list because some local dev databases have a "ghost" v11
///   from another branch in `_sqlx_migrations`, which makes sqlx refuse to
///   apply any v12+ migration. A direct, idempotent `ALTER TABLE` sidesteps
///   that entirely.
///
/// Called from `lib.rs::run()` *before* tauri-plugin-sql wires up. Best-effort:
/// errors are logged but never abort startup, since by far the most common
/// failure here is "column already exists" which is precisely what we want.
pub fn ensure_runtime_schema(db_path: &Path) {
    use rusqlite::Connection;

    if !db_path.exists() {
        // Fresh install — tauri-plugin-sql's migrations will create the table.
        // We'll be invoked again on the next startup once it exists.
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
        // Migrations haven't run yet (first launch); nothing to patch.
        return;
    }

    if !column_exists(&conn, "segments", "speaker_id") {
        match conn.execute("ALTER TABLE segments ADD COLUMN speaker_id INTEGER", []) {
            Ok(_) => tracing::info!(
                "ensure_runtime_schema: added segments.speaker_id (Parakeet/Sortformer)"
            ),
            Err(e) => tracing::warn!(
                "ensure_runtime_schema: ALTER TABLE segments ADD COLUMN speaker_id failed: {e}"
            ),
        }
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

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map(|iter| iter.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    rows.iter().any(|c| c == column)
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
        // The Parakeet/Sortformer `speaker_id` column is *not* added via the
        // migration system. sqlx-style migrations refuse to apply new
        // versions when an unknown applied version (e.g. a "ghost" v11 from
        // another local dev branch) is in `_sqlx_migrations`, and there's no
        // ergonomic way to clean those up from inside a migration. Instead
        // we add the column via `ensure_runtime_schema()` at app startup
        // (idempotent ALTER TABLE that no-ops if the column already exists).
        // See `ensure_runtime_schema()` in this file.
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
        // v11+ are intentionally absent from the migration list; the
        // `speaker_id` column is added by `ensure_runtime_schema()` instead.
        let m = migrations();
        let actual_versions: Vec<i64> = m.iter().map(|x| x.version).collect();
        assert_eq!(actual_versions, (1..=10).collect::<Vec<_>>());
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

        // speaker_id is no longer added via the migration list — see
        // `ensure_runtime_schema()` in this file.
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
            10,
            "v1-v10; speaker_id handled at runtime"
        );
    }
}

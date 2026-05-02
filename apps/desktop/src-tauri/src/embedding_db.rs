//! Vector storage layer for the embedding pipeline.
//!
//! Owns a parallel `rusqlite::Connection` to the same SQLite database that
//! `tauri-plugin-sql` opens. The parallel connection has the `sqlite-vec`
//! extension loaded so it can read/write the `*_embeddings_vec` virtual
//! tables. The main sqlx-based connection knows nothing about vec0.
//!
//! All vector reads/writes go through this module's API. Higher layers
//! (Tauri commands, backfill) call `EmbeddingStore::upsert*` /
//! `delete*` / `search*` and never touch raw SQL.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{ffi::sqlite3_auto_extension, params, Connection};
use sha2::{Digest, Sha256};
use sqlite_vec::sqlite3_vec_init;
use tracing::{debug, info};

/// Vector dimensionality for BGE-small-en-v1.5. Pinned at the schema level
/// because changing dimensions requires re-creating every vec0 table.
pub const EMBEDDING_DIMENSIONS: usize = 384;

/// Source surface for a stored embedding. Determines which pair of tables
/// (`*_embeddings_vec` + `*_embeddings_meta`) the row lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Segment,
    Dictation,
    Note,
}

impl SourceKind {
    fn vec_table(self) -> &'static str {
        match self {
            SourceKind::Segment => "segment_embeddings_vec",
            SourceKind::Dictation => "dictation_embeddings_vec",
            SourceKind::Note => "note_embeddings_vec",
        }
    }

    fn meta_table(self) -> &'static str {
        match self {
            SourceKind::Segment => "segment_embeddings_meta",
            SourceKind::Dictation => "dictation_embeddings_meta",
            SourceKind::Note => "note_embeddings_meta",
        }
    }

    fn id_column(self) -> &'static str {
        match self {
            SourceKind::Segment => "segment_id",
            SourceKind::Dictation => "dictation_id",
            SourceKind::Note => "note_id",
        }
    }
}

/// Result of a KNN search hit.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub source_id: String,
    pub distance: f32,
}

/// Register the sqlite-vec extension globally. MUST be called once at
/// process startup, before any rusqlite connection that needs vec0 is
/// opened. Idempotent across calls within a process. Safe to call before
/// the database file exists.
pub fn register_vec_extension() {
    // Safety: `sqlite3_vec_init` is a valid SQLite extension entry point
    // exported by the linked sqlite-vec library. `sqlite3_auto_extension`
    // stores the function pointer in a global registry that SQLite invokes
    // for every newly-opened connection. The transmute matches the C
    // signature `int (*)(sqlite3*, char**, const sqlite3_api_routines*)`
    // required by `sqlite3_auto_extension`.
    type VecInitFn = unsafe extern "C" fn(
        *mut rusqlite::ffi::sqlite3,
        *mut *mut std::os::raw::c_char,
        *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::os::raw::c_int;
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute::<*const (), VecInitFn>(
            sqlite3_vec_init as *const (),
        )));
    }
    info!("sqlite-vec extension registered for new rusqlite connections");
}

fn vec_to_bytes(v: &[f32]) -> &[u8] {
    // Safety: `f32` has no padding; the resulting byte view is valid for
    // the lifetime of the slice. sqlite-vec expects little-endian f32; we
    // rely on the host being LE (every supported platform is).
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Store handle. Cheap to clone (wraps an `Arc<Mutex<Connection>>`).
#[derive(Clone)]
pub struct EmbeddingStore {
    conn: Arc<Mutex<Connection>>,
}

impl EmbeddingStore {
    /// Open the parallel rusqlite connection and ensure the vec0 virtual
    /// tables exist. The `*_embeddings_meta` tables are created via the
    /// regular tauri-plugin-sql migration path (they're plain SQLite).
    pub fn open(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;
        // PRAGMAs that match what tauri-plugin-sql sets — keeps WAL/journal
        // semantics consistent across both connections.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "foreign_keys", true);

        ensure_vec_tables(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Idempotent insert/replace of an embedding for a source row.
    ///
    /// Checks `content_hash` against the existing meta row first — if
    /// the hash matches AND the model_name/version matches, no work is
    /// done and `Ok(false)` is returned. Returns `Ok(true)` when a new
    /// or updated vector was written.
    pub fn upsert(
        &self,
        kind: SourceKind,
        source_id: &str,
        text: &str,
        vector: &[f32],
        model_name: &str,
        model_version: &str,
    ) -> rusqlite::Result<bool> {
        if vector.len() != EMBEDDING_DIMENSIONS {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "expected {}-dim vector, got {}",
                EMBEDDING_DIMENSIONS,
                vector.len()
            )));
        }

        let hash = content_hash(text);
        let mut conn = self.conn.lock().expect("embedding_db mutex poisoned");
        let tx = conn.transaction()?;

        // Look up existing meta row.
        let existing: Option<(i64, String, String, String)> = tx
            .query_row(
                &format!(
                    "SELECT rowid, content_hash, model_name, model_version \
                     FROM {} WHERE {} = ?",
                    kind.meta_table(),
                    kind.id_column()
                ),
                params![source_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .ok();

        let now = now_iso();

        match existing {
            Some((_, prev_hash, prev_model, prev_version))
                if prev_hash == hash
                    && prev_model == model_name
                    && prev_version == model_version =>
            {
                debug!("embedding upsert noop: kind={:?} id={}", kind, source_id);
                tx.commit()?;
                Ok(false)
            }
            Some((rowid, _, _, _)) => {
                tx.execute(
                    &format!("DELETE FROM {} WHERE rowid = ?", kind.vec_table()),
                    params![rowid],
                )?;
                tx.execute(
                    &format!(
                        "INSERT INTO {} (rowid, embedding) VALUES (?, ?)",
                        kind.vec_table()
                    ),
                    params![rowid, vec_to_bytes(vector)],
                )?;
                tx.execute(
                    &format!(
                        "UPDATE {} SET content_hash = ?, model_name = ?, \
                         model_version = ?, dimensions = ?, updated_at = ? \
                         WHERE rowid = ?",
                        kind.meta_table()
                    ),
                    params![
                        hash,
                        model_name,
                        model_version,
                        EMBEDDING_DIMENSIONS as i64,
                        now,
                        rowid
                    ],
                )?;
                tx.commit()?;
                Ok(true)
            }
            None => {
                // Fresh insert. Reserve a rowid in vec0, mirror it in meta.
                tx.execute(
                    &format!("INSERT INTO {} (embedding) VALUES (?)", kind.vec_table()),
                    params![vec_to_bytes(vector)],
                )?;
                let rowid = tx.last_insert_rowid();
                tx.execute(
                    &format!(
                        "INSERT INTO {} (rowid, {}, content_hash, model_name, \
                         model_version, dimensions, created_at, updated_at) \
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                        kind.meta_table(),
                        kind.id_column()
                    ),
                    params![
                        rowid,
                        source_id,
                        hash,
                        model_name,
                        model_version,
                        EMBEDDING_DIMENSIONS as i64,
                        now,
                        now,
                    ],
                )?;
                tx.commit()?;
                Ok(true)
            }
        }
    }

    /// Delete the embedding for a single source row. No-op if absent.
    pub fn delete(&self, kind: SourceKind, source_id: &str) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().expect("embedding_db mutex poisoned");
        let tx = conn.transaction()?;

        let rowid: Option<i64> = tx
            .query_row(
                &format!(
                    "SELECT rowid FROM {} WHERE {} = ?",
                    kind.meta_table(),
                    kind.id_column()
                ),
                params![source_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(rowid) = rowid {
            tx.execute(
                &format!("DELETE FROM {} WHERE rowid = ?", kind.vec_table()),
                params![rowid],
            )?;
            tx.execute(
                &format!("DELETE FROM {} WHERE rowid = ?", kind.meta_table()),
                params![rowid],
            )?;
        }
        tx.commit()
    }

    /// Cascade delete for a session — removes embeddings for all segments,
    /// dictations, and notes whose source row links to this session.
    /// Reads the source IDs from the regular tables (segments/notes/etc.)
    /// rather than tracking session_id on the meta tables, so we don't
    /// duplicate ownership info.
    pub fn delete_by_session(&self, session_id: &str) -> rusqlite::Result<usize> {
        let mut conn = self.conn.lock().expect("embedding_db mutex poisoned");
        let tx = conn.transaction()?;
        let mut total = 0usize;

        for (kind, source_table, source_id_col) in [
            (SourceKind::Segment, "segments", "id"),
            (SourceKind::Dictation, "dictation_history", "id"),
            (SourceKind::Note, "notes", "id"),
        ] {
            let sql = format!(
                "SELECT m.rowid FROM {} m \
                 JOIN {} s ON s.{} = m.{} \
                 WHERE s.session_id = ?",
                kind.meta_table(),
                source_table,
                source_id_col,
                kind.id_column(),
            );
            let mut stmt = tx.prepare(&sql)?;
            let rowids: Vec<i64> = stmt
                .query_map(params![session_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);
            for rowid in rowids {
                tx.execute(
                    &format!("DELETE FROM {} WHERE rowid = ?", kind.vec_table()),
                    params![rowid],
                )?;
                tx.execute(
                    &format!("DELETE FROM {} WHERE rowid = ?", kind.meta_table()),
                    params![rowid],
                )?;
                total += 1;
            }
        }
        tx.commit()?;
        Ok(total)
    }

    /// KNN search against a single surface, with optional scope filter
    /// applied at the SQL JOIN layer.
    ///
    /// `allowed_session_ids = None` is the unscoped path — returns the
    /// top `k` by distance, no lifecycle filtering. The Tauri command
    /// uses this for the no-scope case.
    ///
    /// `allowed_session_ids = Some(&[…])` clamps results to rows whose
    /// source row's `session_id` is in the list, AND applies surface-
    /// specific lifecycle filters (segments: `hidden = 0 AND deleted_at
    /// IS NULL`). To avoid the truncation pitfall — KNN returns top-k
    /// globally, then scope filter drops most — we over-fetch by a
    /// large factor before clamping. For the realistic case (folder
    /// with N sessions out of M, N << M) this returns enough scoped
    /// hits without iterating.
    pub fn search(
        &self,
        kind: SourceKind,
        query: &[f32],
        k: usize,
        allowed_session_ids: Option<&[String]>,
    ) -> rusqlite::Result<Vec<SearchHit>> {
        if query.len() != EMBEDDING_DIMENSIONS {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "expected {}-dim query vector, got {}",
                EMBEDDING_DIMENSIONS,
                query.len()
            )));
        }
        let conn = self.conn.lock().expect("embedding_db mutex poisoned");

        // Per-surface JOIN clause + lifecycle predicate. Segments are
        // the only surface with soft-delete + hidden columns.
        let (source_table, source_id_col, lifecycle_pred): (&str, &str, &str) = match kind {
            SourceKind::Segment => (
                "segments",
                "id",
                " AND s.deleted_at IS NULL AND s.hidden = 0",
            ),
            SourceKind::Dictation => ("dictation_history", "id", ""),
            SourceKind::Note => ("notes", "id", ""),
        };

        // Effective inner-k: when scoping, over-fetch heavily so the
        // scope clamp doesn't strand valid hits behind out-of-scope
        // ones at the top of the global ranking. Cap at 500 to bound
        // KNN cost.
        let inner_k = match allowed_session_ids {
            Some(_) => (k * 32).clamp(k, 500),
            None => k,
        };

        // Build the IN-list placeholders. Empty allow-list means "no
        // session-id clamp" — we still apply lifecycle filters but
        // don't restrict by session (matches the dictation-chat case
        // where allowedSessionIds is intentionally empty).
        let session_clause: String = match allowed_session_ids {
            Some(ids) if !ids.is_empty() => {
                let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
                format!(" AND s.session_id IN ({})", placeholders.join(","))
            }
            _ => String::new(),
        };

        // When scoped, JOIN to the source table for both the scope
        // filter and lifecycle. When unscoped we keep the cheap shape
        // (meta-only) since we don't need source columns.
        let sql = if allowed_session_ids.is_some() {
            format!(
                "SELECT m.{idcol}, v.distance \
                 FROM {vec} v \
                 JOIN {meta} m ON m.rowid = v.rowid \
                 JOIN {src} s ON s.{src_id} = m.{idcol} \
                 WHERE v.embedding MATCH ? AND k = ?{lifecycle}{session} \
                 ORDER BY v.distance \
                 LIMIT ?",
                idcol = kind.id_column(),
                vec = kind.vec_table(),
                meta = kind.meta_table(),
                src = source_table,
                src_id = source_id_col,
                lifecycle = lifecycle_pred,
                session = session_clause,
            )
        } else {
            format!(
                "SELECT m.{idcol}, v.distance \
                 FROM {vec} v \
                 JOIN {meta} m ON m.rowid = v.rowid \
                 WHERE v.embedding MATCH ? AND k = ? \
                 ORDER BY v.distance \
                 LIMIT ?",
                idcol = kind.id_column(),
                vec = kind.vec_table(),
                meta = kind.meta_table(),
            )
        };

        let mut stmt = conn.prepare(&sql)?;
        // Build params: query bytes, inner_k, then any session-id
        // strings from the IN clause (in declaration order), then the
        // outer LIMIT.
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> =
            Vec::with_capacity(3 + allowed_session_ids.map_or(0, |s| s.len()));
        bind.push(Box::new(vec_to_bytes(query).to_vec()));
        bind.push(Box::new(inner_k as i64));
        if let Some(ids) = allowed_session_ids {
            for id in ids {
                bind.push(Box::new(id.clone()));
            }
        }
        bind.push(Box::new(k as i64));
        let bind_refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(bind_refs.as_slice(), |r| {
            Ok(SearchHit {
                source_id: r.get::<_, String>(0)?,
                distance: r.get::<_, f64>(1)? as f32,
            })
        })?;
        rows.collect()
    }

    /// Returns rows in the named source table that lack an embedding.
    /// Used by the backfill worker on app launch to catch rows from
    /// before the feature shipped, plus any sessions where the live
    /// fire-and-forget embed didn't land. Empty content is excluded so
    /// empty notes (`<p></p>` from Tiptap, etc.) don't hog every batch.
    pub fn missing(
        &self,
        kind: SourceKind,
        batch_size: usize,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().expect("embedding_db mutex poisoned");
        let sql = match kind {
            SourceKind::Segment => format!(
                "SELECT s.id, s.text FROM segments s \
                 LEFT JOIN {} m ON m.{} = s.id \
                 WHERE m.{} IS NULL AND s.deleted_at IS NULL AND s.text != '' \
                 LIMIT ?",
                kind.meta_table(),
                kind.id_column(),
                kind.id_column(),
            ),
            SourceKind::Dictation => format!(
                "SELECT d.id, COALESCE(NULLIF(d.output_text, ''), d.input_text) \
                 FROM dictation_history d \
                 LEFT JOIN {} m ON m.{} = d.id \
                 WHERE m.{} IS NULL \
                   AND COALESCE(NULLIF(d.output_text, ''), d.input_text) != '' \
                 LIMIT ?",
                kind.meta_table(),
                kind.id_column(),
                kind.id_column(),
            ),
            SourceKind::Note => format!(
                "SELECT n.id, n.content FROM notes n \
                 LEFT JOIN {} m ON m.{} = n.id \
                 WHERE m.{} IS NULL \
                   AND trim(n.content) NOT IN ( \
                     '', '<p></p>', '<p><br></p>', '<p><br/></p>', \
                     '<p><br /></p>', '<p> </p>', '<p>&nbsp;</p>' \
                   ) \
                 LIMIT ?",
                kind.meta_table(),
                kind.id_column(),
                kind.id_column(),
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![batch_size as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// Run with an exclusive lock. Used by tests.
    #[cfg(test)]
    pub fn with_conn<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
    }
}

/// Idempotently create the vec0 virtual tables. Called from `EmbeddingStore::open`.
fn ensure_vec_tables(conn: &Connection) -> rusqlite::Result<()> {
    for table in &[
        "segment_embeddings_vec",
        "dictation_embeddings_vec",
        "note_embeddings_vec",
    ] {
        conn.execute(
            &format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS {} USING vec0(embedding FLOAT[{}])",
                table, EMBEDDING_DIMENSIONS
            ),
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, EmbeddingStore) {
        register_vec_extension();
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        // Create the source tables + meta tables that the store joins
        // against. Real app uses the migration path; tests inline the
        // minimum schema needed.
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY);
             CREATE TABLE segments (
                id TEXT PRIMARY KEY, session_id TEXT, text TEXT,
                deleted_at TEXT, hidden INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE notes (
                id TEXT PRIMARY KEY, session_id TEXT, content TEXT
             );
             CREATE TABLE dictation_history (
                id TEXT PRIMARY KEY, session_id TEXT,
                input_text TEXT, output_text TEXT
             );
             CREATE TABLE segment_embeddings_meta (
                rowid INTEGER PRIMARY KEY, segment_id TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL, model_name TEXT NOT NULL,
                model_version TEXT NOT NULL, dimensions INTEGER NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE dictation_embeddings_meta (
                rowid INTEGER PRIMARY KEY, dictation_id TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL, model_name TEXT NOT NULL,
                model_version TEXT NOT NULL, dimensions INTEGER NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE note_embeddings_meta (
                rowid INTEGER PRIMARY KEY, note_id TEXT NOT NULL UNIQUE,
                content_hash TEXT NOT NULL, model_name TEXT NOT NULL,
                model_version TEXT NOT NULL, dimensions INTEGER NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );",
        )
        .unwrap();
        drop(conn);
        let store = EmbeddingStore::open(&db_path).unwrap();
        (dir, store)
    }

    fn fake_vec(seed: f32) -> Vec<f32> {
        (0..EMBEDDING_DIMENSIONS)
            .map(|i| seed + (i as f32) * 0.001)
            .collect()
    }

    #[test]
    fn upsert_idempotent_on_same_text() {
        let (_d, store) = setup();
        store.with_conn(|c| {
            c.execute(
                "INSERT INTO segments (id, session_id, text) VALUES ('s1', 'S', 'hello')",
                [],
            )
            .unwrap();
        });
        let v = fake_vec(0.1);
        let wrote = store
            .upsert(
                SourceKind::Segment,
                "s1",
                "hello",
                &v,
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        assert!(wrote);
        let wrote2 = store
            .upsert(
                SourceKind::Segment,
                "s1",
                "hello",
                &v,
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        assert!(!wrote2, "same hash + model should be a no-op");
    }

    #[test]
    fn upsert_replaces_on_text_change() {
        let (_d, store) = setup();
        store.with_conn(|c| {
            c.execute(
                "INSERT INTO segments (id, session_id, text) VALUES ('s1', 'S', 'a')",
                [],
            )
            .unwrap();
        });
        store
            .upsert(
                SourceKind::Segment,
                "s1",
                "a",
                &fake_vec(0.1),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        let wrote = store
            .upsert(
                SourceKind::Segment,
                "s1",
                "b",
                &fake_vec(0.2),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        assert!(wrote);
    }

    #[test]
    fn knn_returns_closest_first() {
        let (_d, store) = setup();
        store.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO segments (id, session_id, text) VALUES ('s1', 'S', 'a');
                 INSERT INTO segments (id, session_id, text) VALUES ('s2', 'S', 'b');
                 INSERT INTO segments (id, session_id, text) VALUES ('s3', 'S', 'c');",
            )
            .unwrap();
        });
        store
            .upsert(
                SourceKind::Segment,
                "s1",
                "a",
                &fake_vec(0.0),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        store
            .upsert(
                SourceKind::Segment,
                "s2",
                "b",
                &fake_vec(0.5),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        store
            .upsert(
                SourceKind::Segment,
                "s3",
                "c",
                &fake_vec(1.0),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        let hits = store
            .search(SourceKind::Segment, &fake_vec(0.0), 3, None)
            .unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].source_id, "s1");
    }

    #[test]
    fn delete_by_session_cascades() {
        let (_d, store) = setup();
        store.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO sessions (id) VALUES ('S');
                 INSERT INTO segments (id, session_id, text) VALUES ('s1', 'S', 'a');
                 INSERT INTO segments (id, session_id, text) VALUES ('s2', 'S', 'b');
                 INSERT INTO notes (id, session_id, content) VALUES ('n1', 'S', 'note');",
            )
            .unwrap();
        });
        store
            .upsert(
                SourceKind::Segment,
                "s1",
                "a",
                &fake_vec(0.0),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        store
            .upsert(
                SourceKind::Segment,
                "s2",
                "b",
                &fake_vec(0.5),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        store
            .upsert(
                SourceKind::Note,
                "n1",
                "note",
                &fake_vec(0.7),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        let removed = store.delete_by_session("S").unwrap();
        assert_eq!(removed, 3);
    }

    #[test]
    fn missing_returns_unembedded_rows() {
        let (_d, store) = setup();
        store.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO segments (id, session_id, text) VALUES ('s1', 'S', 'a');
                 INSERT INTO segments (id, session_id, text) VALUES ('s2', 'S', 'b');",
            )
            .unwrap();
        });
        store
            .upsert(
                SourceKind::Segment,
                "s1",
                "a",
                &fake_vec(0.0),
                "bge-small-en-v1.5",
                "1.5.0",
            )
            .unwrap();
        let missing = store.missing(SourceKind::Segment, 10).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "s2");
    }
}

use super::{CacheEntry, GraphPatchEntry};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(super) fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    let mut value = database.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

pub(super) fn base_scope(base_branch: Option<&str>) -> &str {
    base_branch.unwrap_or("")
}

pub(super) fn base_tip_meta_key(base_branch: Option<&str>) -> String {
    format!("base_tip:{}", base_scope(base_branch))
}

pub(super) fn open_conn(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    // Wait out concurrent writers (phase-1 threads + squash loader share this file)
    // instead of failing fast with SQLITE_BUSY.
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
    Ok(conn)
}

pub(super) fn ensure_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS branch_cache (
            base_branch  TEXT NOT NULL,
            branch_name  TEXT NOT NULL,
            merge_status TEXT NOT NULL,
            commit_hash  TEXT NOT NULL,
            PRIMARY KEY (base_branch, branch_name)
        );
        CREATE TABLE IF NOT EXISTS ahead_behind (
            key    TEXT PRIMARY KEY,
            ahead  INTEGER NOT NULL,
            behind INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS merge_base (
            key        TEXT PRIMARY KEY,
            merge_base TEXT
        );
        CREATE TABLE IF NOT EXISTS graph_patch (
            key        TEXT PRIMARY KEY,
            patch_id   TEXT,
            diff_text  BLOB
        );
        CREATE TABLE IF NOT EXISTS commit_patch_id (
            key TEXT PRIMARY KEY,
            patch_id TEXT
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
}

#[allow(clippy::type_complexity)]
pub(super) fn read_all(
    path: &Path,
    base_branch: Option<&str>,
) -> (
    HashMap<String, CacheEntry>,
    HashMap<String, [u32; 2]>,
    HashMap<String, Option<String>>,
    Option<String>,
    HashMap<String, GraphPatchEntry>,
) {
    if !path.exists() {
        return (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            None,
            HashMap::new(),
        );
    }
    let Ok(conn) = open_conn(path) else {
        return (
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            None,
            HashMap::new(),
        );
    };
    (
        read_entries(&conn, base_scope(base_branch)),
        read_ahead_behind(&conn),
        read_merge_base(&conn),
        read_base_tip(&conn, &base_tip_meta_key(base_branch)),
        read_graph_patch(&conn),
    )
}

fn read_entries(conn: &Connection, base_branch: &str) -> HashMap<String, CacheEntry> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT branch_name, merge_status, commit_hash FROM branch_cache WHERE base_branch = ?1",
    ) else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([base_branch], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CacheEntry {
                merge_status: row.get(1)?,
                commit_hash: row.get(2)?,
            },
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_ahead_behind(conn: &Connection) -> HashMap<String, [u32; 2]> {
    let Ok(mut stmt) = conn.prepare("SELECT key, ahead, behind FROM ahead_behind") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            [row.get::<_, u32>(1)?, row.get::<_, u32>(2)?],
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_merge_base(conn: &Connection) -> HashMap<String, Option<String>> {
    let Ok(mut stmt) = conn.prepare("SELECT key, merge_base FROM merge_base") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_graph_patch(conn: &Connection) -> HashMap<String, GraphPatchEntry> {
    let Ok(mut stmt) = conn.prepare("SELECT key, patch_id, diff_text FROM graph_patch") else {
        return HashMap::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            GraphPatchEntry {
                patch_id: row.get::<_, Option<String>>(1)?,
                diff_text: row.get::<_, Option<Vec<u8>>>(2)?,
            },
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten().collect()
}

fn read_base_tip(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .ok()
}

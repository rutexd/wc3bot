use rusqlite::Connection;

pub fn run(conn: &Connection) {
    // --- schema ---
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS users (
            chat_id INTEGER PRIMARY KEY,
            notifications_enabled INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS subs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            chat_id INTEGER NOT NULL REFERENCES users(chat_id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            pattern TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            UNIQUE (chat_id, kind, pattern)
         );
         CREATE INDEX IF NOT EXISTS idx_subs_chat ON subs(chat_id);
         CREATE TABLE IF NOT EXISTS map_mutes (
            chat_id INTEGER NOT NULL REFERENCES users(chat_id) ON DELETE CASCADE,
            map TEXT NOT NULL,
            until INTEGER,
            PRIMARY KEY (chat_id, map)
         );
         CREATE TABLE IF NOT EXISTS sub_hosts (
            sub_id INTEGER NOT NULL REFERENCES subs(id) ON DELETE CASCADE,
            host TEXT NOT NULL,
            PRIMARY KEY (sub_id, host)
         );",
    )
    .expect("failed to create schema");

    // --- incremental migrations ---
    // 001: language column for users
    let _ = conn.execute(
        "ALTER TABLE users ADD COLUMN lang TEXT NOT NULL DEFAULT 'en'",
        [],
    );

    // 002: clean orphaned map_mutes left behind when subscriptions were deleted
    let _ = conn.execute(
        "DELETE FROM map_mutes WHERE NOT EXISTS (
            SELECT 1 FROM subs
            WHERE subs.chat_id = map_mutes.chat_id
              AND subs.kind = 'map'
              AND subs.pattern = map_mutes.map
        )",
        [],
    );

    // 003: host filter support for map/name subscriptions
    let _ = conn.execute(
        "ALTER TABLE subs ADD COLUMN host_filter TEXT NOT NULL DEFAULT 'off'",
        [],
    );
}

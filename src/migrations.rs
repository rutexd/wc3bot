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
            kind TEXT NOT NULL,
            pattern TEXT NOT NULL,
            until INTEGER,
            PRIMARY KEY (chat_id, kind, pattern)
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

    // 002: legacy schema cleanup. Old map_mutes was keyed by full game map string;
    //      the new schema keys it by (kind, pattern). Drop the old table if present.
    let legacy_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(map_mutes)")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .map(|rows| rows.filter_map(Result::ok).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    if legacy_columns.iter().any(|c| c == "map") {
        let _ = conn.execute("DROP TABLE map_mutes", []);
    }
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS map_mutes (
            chat_id INTEGER NOT NULL REFERENCES users(chat_id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            pattern TEXT NOT NULL,
            until INTEGER,
            PRIMARY KEY (chat_id, kind, pattern)
         )",
        [],
    );

    // 003: host filter support for map/name subscriptions
    let _ = conn.execute(
        "ALTER TABLE subs ADD COLUMN host_filter TEXT NOT NULL DEFAULT 'off'",
        [],
    );

    // 004: quiet hours — UTC offset in minutes plus start/end minutes-of-day in that offset.
    //      NULL columns mean the feature is disabled for that user.
    let _ = conn.execute(
        "ALTER TABLE users ADD COLUMN qh_tz_offset INTEGER",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE users ADD COLUMN qh_start_min INTEGER",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE users ADD COLUMN qh_end_min INTEGER",
        [],
    );
}

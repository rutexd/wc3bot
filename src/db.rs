use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

pub const KIND_MAP: &str = "map";
pub const KIND_HOST: &str = "host";
pub const KIND_NAME: &str = "name";

pub const SNOOZE_SECS: i64 = 12 * 60 * 60;

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A per-map notification suppression. `until == None` means forever.
#[derive(Debug, Clone)]
pub struct MapMute {
    pub map: String,
    pub until: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Sub {
    pub id: i64,
    pub chat_id: i64,
    pub kind: String,
    pub pattern: String,
    pub enabled: bool,
}

impl Sub {
}

pub struct Db(Mutex<Connection>);

pub struct ActiveSub {
    pub chat_id: i64,
    pub sub: Sub,
}

impl Db {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
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
             );",
        )?;
        // Migration for DBs created before the language setting existed.
        let _ = conn.execute(
            "ALTER TABLE users ADD COLUMN lang TEXT NOT NULL DEFAULT 'en'",
            [],
        );
        Ok(Db(Mutex::new(conn)))
    }

    pub fn ensure_user(&self, chat_id: i64) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute("INSERT OR IGNORE INTO users(chat_id) VALUES (?1)", params![chat_id]);
    }

    pub fn notifications_enabled(&self, chat_id: i64) -> bool {
        self.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT notifications_enabled FROM users WHERE chat_id = ?1",
                params![chat_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    pub fn set_notifications(&self, chat_id: i64, on: bool) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO users(chat_id, notifications_enabled) VALUES (?1, ?2)
             ON CONFLICT(chat_id) DO UPDATE SET notifications_enabled = ?2",
            params![chat_id, on as i64],
        );
    }

    /// Suppress notifications for `map` for SNOOZE_SECS, then auto re-enabled.
    pub fn snooze_map(&self, chat_id: i64, map: &str) {
        let until = now_ts() + SNOOZE_SECS;
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO map_mutes(chat_id, map, until) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat_id, map) DO UPDATE SET until = ?3",
            params![chat_id, map, until],
        );
    }

    /// Suppress notifications for `map` forever (manual re-enable needed).
    pub fn mute_map(&self, chat_id: i64, map: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO map_mutes(chat_id, map, until) VALUES (?1, ?2, NULL)
             ON CONFLICT(chat_id, map) DO UPDATE SET until = NULL",
            params![chat_id, map],
        );
    }

    /// Re-enable suppressed maps for users whose snooze expired.
    pub fn release_expired_map_snoozes(&self) {
        let now = now_ts();
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM map_mutes WHERE until IS NOT NULL AND until <= ?1",
            params![now],
        );
    }

    pub fn is_map_muted(&self, chat_id: i64, map: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM map_mutes WHERE chat_id = ?1 AND map = ?2",
                params![chat_id, map],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn list_map_mutes(&self, chat_id: i64) -> Vec<MapMute> {
        let conn = self.0.lock().unwrap();
        let mut stmt = match conn.prepare_cached(
            "SELECT map, until FROM map_mutes WHERE chat_id = ?1 ORDER BY map",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![chat_id], |row| {
            Ok(MapMute {
                map: row.get(0)?,
                until: row.get(1)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }

    /// Returns Err(true) if duplicate.
    pub fn add_sub(&self, chat_id: i64, kind: &str, pattern: &str) -> Result<bool> {
        let conn = self.0.lock().unwrap();
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO subs(chat_id, kind, pattern) VALUES (?1, ?2, ?3)",
                params![chat_id, kind, pattern],
            )
            .map(|n| n > 0)?;
        if !inserted {
            anyhow::bail!("duplicate");
        }
        Ok(inserted)
    }

    pub fn list_subs(&self, chat_id: i64) -> Vec<Sub> {
        let conn = self.0.lock().unwrap();
        let mut stmt = match conn.prepare_cached(
            "SELECT id, chat_id, kind, pattern, enabled FROM subs WHERE chat_id = ?1 ORDER BY id",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![chat_id], row_to_sub)
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default()
    }

    pub fn get_sub(&self, id: i64) -> Option<Sub> {
        self.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT id, chat_id, kind, pattern, enabled FROM subs WHERE id = ?1",
                params![id],
                row_to_sub,
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_sub_enabled(&self, id: i64, on: bool) -> bool {
        self.0
            .lock()
            .unwrap()
            .execute("UPDATE subs SET enabled = ?2 WHERE id = ?1", params![id, on as i64])
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    pub fn rename_sub(&self, id: i64, pattern: &str) -> Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE subs SET pattern = ?2 WHERE id = ?1",
            params![id, pattern],
        )?;
        Ok(())
    }

    pub fn delete_sub(&self, id: i64) -> bool {
        self.0
            .lock()
            .unwrap()
            .execute("DELETE FROM subs WHERE id = ?1", params![id])
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    pub fn delete_user(&self, chat_id: i64) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute("DELETE FROM users WHERE chat_id = ?1", params![chat_id]);
    }

    pub fn lang(&self, chat_id: i64) -> crate::loc::Lang {
        let code: String = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT lang FROM users WHERE chat_id = ?1",
                params![chat_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| "en".to_string());
        crate::loc::Lang::parse(&code)
    }

    pub fn set_lang(&self, chat_id: i64, lang: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO users(chat_id, lang) VALUES (?1, ?2)
             ON CONFLICT(chat_id) DO UPDATE SET lang = ?2",
            params![chat_id, lang],
        );
    }

    /// All enabled subscriptions of users with global notifications on.
    pub fn all_active_subs(&self) -> Vec<ActiveSub> {
        let conn = self.0.lock().unwrap();
        let mut stmt = match conn.prepare_cached(
            "SELECT s.id, s.chat_id, s.kind, s.pattern, s.enabled
             FROM subs s JOIN users u ON u.chat_id = s.chat_id
             WHERE s.enabled = 1 AND u.notifications_enabled = 1
             ORDER BY s.id",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |row| {
            Ok(ActiveSub {
                chat_id: row.get(1)?,
                sub: row_to_sub(row)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }
}

fn row_to_sub(row: &rusqlite::Row<'_>) -> rusqlite::Result<Sub> {
    Ok(Sub {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        kind: row.get(2)?,
        pattern: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
    })
}

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Mutex;

pub const KIND_MAP: &str = "map";
pub const KIND_HOST: &str = "host";
pub const KIND_NAME: &str = "name";

pub const HF_OFF: &str = "off";
pub const HF_WHITELIST: &str = "whitelist";
pub const HF_BLACKLIST: &str = "blacklist";

pub const SNOOZE_SECS: i64 = 12 * 60 * 60;

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Настройки тихого часа. `None` означает, что фича выключена
/// (любой из трёх столбцов = NULL).
#[derive(Debug, Clone, Copy)]
pub struct QuietHours {
    pub tz_offset_min: i32,
    pub start_min: i32,
    pub end_min: i32,
}

/// A per-subscription notification suppression. `until == None` means forever.
#[derive(Debug, Clone)]
pub struct MapMute {
    pub kind: String,
    pub pattern: String,
    pub until: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Sub {
    pub id: i64,
    pub chat_id: i64,
    pub kind: String,
    pub pattern: String,
    pub enabled: bool,
    #[allow(dead_code)]
    pub host_filter: String,
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
        crate::migrations::run(&conn);
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

    /// Suppress notifications for subscriptions matching `(kind, pattern)` for SNOOZE_SECS,
    /// then auto re-enabled.
    pub fn snooze_sub(&self, chat_id: i64, kind: &str, pattern: &str) {
        let until = now_ts() + SNOOZE_SECS;
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO map_mutes(chat_id, kind, pattern, until) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat_id, kind, pattern) DO UPDATE SET until = ?4",
            params![chat_id, kind, pattern, until],
        );
    }

    /// Suppress notifications for subscriptions matching `(kind, pattern)` forever
    /// (manual re-enable needed).
    pub fn mute_sub(&self, chat_id: i64, kind: &str, pattern: &str) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO map_mutes(chat_id, kind, pattern, until) VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(chat_id, kind, pattern) DO UPDATE SET until = NULL",
            params![chat_id, kind, pattern],
        );
    }

    /// Re-enable suppressed selectors whose snooze expired.
    pub fn release_expired_map_snoozes(&self) {
        let now = now_ts();
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM map_mutes WHERE until IS NOT NULL AND until <= ?1",
            params![now],
        );
    }

    pub fn is_sub_muted(&self, chat_id: i64, kind: &str, pattern: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .query_row(
                "SELECT 1 FROM map_mutes WHERE chat_id = ?1 AND kind = ?2 AND pattern = ?3",
                params![chat_id, kind, pattern],
                |_| Ok(()),
            )
            .is_ok()
    }

    pub fn list_map_mutes(&self, chat_id: i64) -> Vec<MapMute> {
        let conn = self.0.lock().unwrap();
        let mut stmt = match conn.prepare_cached(
            "SELECT kind, pattern, until FROM map_mutes WHERE chat_id = ?1 ORDER BY kind, pattern",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![chat_id], |row| {
            Ok(MapMute {
                kind: row.get(0)?,
                pattern: row.get(1)?,
                until: row.get(2)?,
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
            "SELECT id, chat_id, kind, pattern, enabled, host_filter FROM subs WHERE chat_id = ?1 ORDER BY id",
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
                "SELECT id, chat_id, kind, pattern, enabled, host_filter FROM subs WHERE id = ?1",
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

    /// Delete the mute entry for a specific (chat_id, kind, pattern) selector.
    pub fn delete_map_mute(&self, chat_id: i64, kind: &str, pattern: &str) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM map_mutes WHERE chat_id = ?1 AND kind = ?2 AND pattern = ?3",
                params![chat_id, kind, pattern],
            );
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

    /// Настройки тихого часа. `None` означает, что фича выключена
    /// (любой из трёх столбцов = NULL).
    pub fn get_quiet_hours(&self, chat_id: i64) -> Option<QuietHours> {
        let conn = self.0.lock().unwrap();
        let res: Option<(Option<i32>, Option<i32>, Option<i32>)> = conn
            .query_row(
                "SELECT qh_tz_offset, qh_start_min, qh_end_min FROM users WHERE chat_id = ?1",
                params![chat_id],
                |row| {
                    let tz: Option<i32> = row.get(0)?;
                    let start: Option<i32> = row.get(1)?;
                    let end: Option<i32> = row.get(2)?;
                    Ok((tz, start, end))
                },
            )
            .optional()
            .ok()
            .flatten();
        res.and_then(|(tz, start, end)| match (tz, start, end) {
            (Some(tz), Some(start), Some(end)) => Some(QuietHours { tz_offset_min: tz, start_min: start, end_min: end }),
            _ => None,
        })
    }

    pub fn set_quiet_hours(&self, chat_id: i64, tz_offset_min: i32, start_min: i32, end_min: i32) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "INSERT INTO users(chat_id, qh_tz_offset, qh_start_min, qh_end_min)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat_id) DO UPDATE SET
                qh_tz_offset = ?2,
                qh_start_min = ?3,
                qh_end_min   = ?4",
            params![chat_id, tz_offset_min, start_min, end_min],
        );
    }

    pub fn disable_quiet_hours(&self, chat_id: i64) {
        let conn = self.0.lock().unwrap();
        let _ = conn.execute(
            "UPDATE users SET qh_tz_offset = NULL, qh_start_min = NULL, qh_end_min = NULL
             WHERE chat_id = ?1",
            params![chat_id],
        );
    }

    /// True, если прямо сейчас для пользователя активно окно уведомлений
    /// (либо окно не настроено).
    pub fn is_in_notification_window(&self, chat_id: i64) -> bool {
        let Some(qh) = self.get_quiet_hours(chat_id) else {
            return true;
        };
        crate::quiet::is_in_notification_window(now_ts(), qh.tz_offset_min, qh.start_min, qh.end_min)
    }

    pub fn get_host_filter(&self, sub_id: i64) -> (String, Vec<String>) {
        let conn = self.0.lock().unwrap();
        let mode: String = conn
            .query_row(
                "SELECT host_filter FROM subs WHERE id = ?1",
                params![sub_id],
                |r| r.get(0),
            )
            .unwrap_or_else(|_| HF_OFF.to_string());
        let mut stmt = match conn.prepare_cached(
            "SELECT host FROM sub_hosts WHERE sub_id = ?1 ORDER BY host",
        ) {
            Ok(s) => s,
            Err(_) => return (mode, Vec::new()),
        };
        let hosts = stmt
            .query_map(params![sub_id], |row| row.get(0))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        (mode, hosts)
    }

    pub fn set_host_filter_mode(&self, sub_id: i64, mode: &str) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute(
                "UPDATE subs SET host_filter = ?2 WHERE id = ?1",
                params![sub_id, mode],
            );
    }

    pub fn add_sub_host(&self, sub_id: i64, host: &str) -> bool {
        self.0
            .lock()
            .unwrap()
            .execute(
                "INSERT OR IGNORE INTO sub_hosts(sub_id, host) VALUES (?1, ?2)",
                params![sub_id, host],
            )
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    pub fn remove_sub_host(&self, sub_id: i64, host: &str) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM sub_hosts WHERE sub_id = ?1 AND host = ?2",
                params![sub_id, host],
            );
    }

    /// Check if a game host passes the filter for a given subscription.
    /// Returns true if the game should be notified.
    pub fn host_filter_passes(&self, sub_id: i64, game_host: &str) -> bool {
        let (mode, hosts) = self.get_host_filter(sub_id);
        match mode.as_str() {
            HF_WHITELIST => hosts.iter().any(|h| crate::norm::matches(h, game_host)),
            HF_BLACKLIST => !hosts.iter().any(|h| crate::norm::matches(h, game_host)),
            _ => true,
        }
    }

    /// All enabled subscriptions of users with global notifications on.
    pub fn all_active_subs(&self) -> Vec<ActiveSub> {
        let conn = self.0.lock().unwrap();
        let mut stmt = match conn.prepare_cached(
            "SELECT s.id, s.chat_id, s.kind, s.pattern, s.enabled, s.host_filter
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

    /// Check if a game matches a subscription and should trigger a notification.
    pub fn should_notify(&self, sub: &Sub, game_map: &str, game_host: &str, game_name: &str) -> bool {
        let matched = match sub.kind.as_str() {
            KIND_HOST => crate::norm::matches(&sub.pattern, game_host),
            KIND_NAME => crate::norm::matches(&sub.pattern, game_name),
            _ => crate::norm::matches(&sub.pattern, game_map),
        };
        if !matched {
            return false;
        }
        if self.is_sub_muted(sub.chat_id, &sub.kind, &sub.pattern) {
            return false;
        }
        if (sub.kind == KIND_MAP || sub.kind == KIND_NAME)
            && !self.host_filter_passes(sub.id, game_host)
        {
            return false;
        }
        true
    }
}

fn row_to_sub(row: &rusqlite::Row<'_>) -> rusqlite::Result<Sub> {
    Ok(Sub {
        id: row.get(0)?,
        chat_id: row.get(1)?,
        kind: row.get(2)?,
        pattern: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        host_filter: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Db {
        Db::open(":memory:").unwrap()
    }

    fn temp_db() -> (Db, String) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = format!(
            "{}_test_{}_{}.db",
            env!("CARGO_PKG_NAME"),
            std::process::id(),
            n
        );
        let db = Db::open(&path).unwrap();
        (db, path)
    }

    #[test]
    fn delete_sub_cleans_map_mute() {
        let db = memory_db();
        let uid = 42;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "Tetris");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.mute_sub(uid, KIND_MAP, "Tetris");
        assert!(db.is_sub_muted(uid, KIND_MAP, "Tetris"));

        db.delete_sub(sub.id);
        db.delete_map_mute(uid, &sub.kind, &sub.pattern);

        assert!(!db.is_sub_muted(uid, KIND_MAP, "Tetris"));
    }

    #[test]
    fn delete_map_mute_removes_entry() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        db.mute_sub(uid, KIND_MAP, "SomeMap");
        assert!(db.is_sub_muted(uid, KIND_MAP, "SomeMap"));

        db.delete_map_mute(uid, KIND_MAP, "SomeMap");
        assert!(!db.is_sub_muted(uid, KIND_MAP, "SomeMap"));
    }

    #[test]
    fn delete_sub_does_not_touch_mutes_of_other_subs() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "MapA");
        let _ = db.add_sub(uid, KIND_HOST, "HostA");
        let subs = db.list_subs(uid);

        db.mute_sub(uid, KIND_MAP, "MapA");
        db.mute_sub(uid, KIND_HOST, "HostA");

        let map_sub = subs.iter().find(|s| s.kind == KIND_MAP).unwrap().clone();
        db.delete_sub(map_sub.id);
        db.delete_map_mute(uid, &map_sub.kind, &map_sub.pattern);

        // host mute survives
        assert!(!db.is_sub_muted(uid, KIND_MAP, "MapA"));
        assert!(db.is_sub_muted(uid, KIND_HOST, "HostA"));
    }

    #[test]
    fn migration_legacy_map_mutes_dropped() {
        // simulate an old database with the legacy `map` column
        let (db, path) = temp_db();
        db.0.lock()
            .unwrap()
            .execute("DROP TABLE map_mutes", [])
            .unwrap();
        db.0.lock()
            .unwrap()
            .execute(
                "CREATE TABLE map_mutes (
                    chat_id INTEGER NOT NULL REFERENCES users(chat_id) ON DELETE CASCADE,
                    map TEXT NOT NULL,
                    until INTEGER,
                    PRIMARY KEY (chat_id, map)
                 )",
                [],
            )
            .unwrap();
        db.ensure_user(1);
        db.0.lock()
            .unwrap()
            .execute(
                "INSERT INTO map_mutes(chat_id, map, until) VALUES (1, 'LegacyMap', NULL)",
                [],
            )
            .unwrap();
        drop(db);

        let db = Db::open(&path).unwrap();
        // legacy rows gone, no new-format entries were created
        assert!(db.list_map_mutes(1).is_empty());
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    // --- host filter tests ---

    #[test]
    fn host_filter_default_is_off() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();
        let (mode, hosts) = db.get_host_filter(sub.id);
        assert_eq!(mode, HF_OFF);
        assert!(hosts.is_empty());
    }

    #[test]
    fn host_filter_toggle_cycle() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        assert_eq!(db.get_host_filter(sub.id).0, HF_OFF);
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        assert_eq!(db.get_host_filter(sub.id).0, HF_WHITELIST);
        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        assert_eq!(db.get_host_filter(sub.id).0, HF_BLACKLIST);
        db.set_host_filter_mode(sub.id, HF_OFF);
        assert_eq!(db.get_host_filter(sub.id).0, HF_OFF);
    }

    #[test]
    fn host_filter_add_remove_hosts() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        assert!(db.add_sub_host(sub.id, "HellWolf"));
        assert!(db.add_sub_host(sub.id, "CHS_Bot"));
        // duplicate
        assert!(!db.add_sub_host(sub.id, "HellWolf"));

        let (_, hosts) = db.get_host_filter(sub.id);
        assert_eq!(hosts, vec!["CHS_Bot", "HellWolf"]); // ORDER BY host

        db.remove_sub_host(sub.id, "HellWolf");
        let (_, hosts) = db.get_host_filter(sub.id);
        assert_eq!(hosts, vec!["CHS_Bot"]);
    }

    #[test]
    fn host_filter_whitelist_passes() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");

        assert!(db.host_filter_passes(sub.id, "HellWolf"));
        assert!(db.host_filter_passes(sub.id, "HellWolf#12345"));
        assert!(!db.host_filter_passes(sub.id, "SomeOtherGuy"));
    }

    #[test]
    fn host_filter_blacklist_blocks() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "HellWolf");

        assert!(!db.host_filter_passes(sub.id, "HellWolf"));
        assert!(!db.host_filter_passes(sub.id, "HellWolf#12345"));
        assert!(db.host_filter_passes(sub.id, "SomeOtherGuy"));
    }

    #[test]
    fn host_filter_off_passes_all() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        assert!(db.host_filter_passes(sub.id, "Anyone"));
        assert!(db.host_filter_passes(sub.id, ""));
    }

    #[test]
    fn host_filter_empty_whitelist_blocks_all() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        // no hosts added
        assert!(!db.host_filter_passes(sub.id, "HellWolf"));
    }

    #[test]
    fn delete_sub_cascades_sub_hosts() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.add_sub_host(sub.id, "HellWolf");
        db.add_sub_host(sub.id, "CHS_Bot");
        let (_, hosts) = db.get_host_filter(sub.id);
        assert_eq!(hosts.len(), 2);

        db.delete_sub(sub.id);
        // sub_hosts should be gone (cascaded)
        let (_, hosts) = db.get_host_filter(sub.id);
        assert!(hosts.is_empty());
    }

    #[test]
    fn host_filter_works_for_name_subs() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_NAME, "MyGame");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "TrustedHost");

        assert!(db.host_filter_passes(sub.id, "TrustedHost"));
        assert!(!db.host_filter_passes(sub.id, "RandomGuy"));
    }

    #[test]
    fn host_filter_survives_reopen() {
        let (db, path) = temp_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "PersistMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "BadHost");
        drop(db);

        let db = Db::open(&path).unwrap();
        let (mode, hosts) = db.get_host_filter(sub.id);
        assert_eq!(mode, HF_BLACKLIST);
        assert_eq!(hosts, vec!["BadHost"]);
        assert!(!db.host_filter_passes(sub.id, "BadHost"));
        assert!(db.host_filter_passes(sub.id, "GoodHost"));
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn all_active_subs_includes_host_filter() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "FilteredMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "OnlyThisGuy");

        let active = db.all_active_subs();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].sub.host_filter, HF_WHITELIST);
    }

    #[test]
    fn host_filter_fuzzy_matching() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "TestMap");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "CHS");

        // fuzzy: "CHS" should match "CHS -", "(10) CHS 5x5", "chsbot" etc.
        assert!(db.host_filter_passes(sub.id, "CHS"));
        assert!(db.host_filter_passes(sub.id, "CHS - Player"));
        assert!(db.host_filter_passes(sub.id, "something*chs*"));
        assert!(!db.host_filter_passes(sub.id, "NoMatchHere"));
    }

    // ===================== user tests =====================

    #[test]
    fn ensure_user_is_idempotent() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(1);
        db.ensure_user(1);
        // no panic, no error
    }

    #[test]
    fn notifications_default_false_for_unknown() {
        let db = memory_db();
        assert!(!db.notifications_enabled(999));
    }

    #[test]
    fn notifications_default_true_after_ensure() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.notifications_enabled(1));
    }

    #[test]
    fn set_notifications_toggle() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.notifications_enabled(1));

        db.set_notifications(1, false);
        assert!(!db.notifications_enabled(1));

        db.set_notifications(1, true);
        assert!(db.notifications_enabled(1));
    }

    #[test]
    fn set_notifications_upserts_user() {
        let db = memory_db();
        // set_notifications on non-existent user creates the user row
        db.set_notifications(42, false);
        assert!(!db.notifications_enabled(42));
        assert!(!db.notifications_enabled(42));
        db.set_notifications(42, true);
        assert!(db.notifications_enabled(42));
    }

    // ===================== lang tests =====================

    #[test]
    fn lang_defaults_to_en() {
        let db = memory_db();
        db.ensure_user(1);
        assert_eq!(db.lang(1).code(), "en");
    }

    #[test]
    fn lang_unknown_user_defaults_to_en() {
        let db = memory_db();
        assert_eq!(db.lang(999).code(), "en");
    }

    #[test]
    fn set_lang_and_read_back() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_lang(1, "ru");
        assert_eq!(db.lang(1).code(), "ru");
        db.set_lang(1, "en");
        assert_eq!(db.lang(1).code(), "en");
    }

    // ===================== sub CRUD tests =====================

    #[test]
    fn add_sub_success() {
        let db = memory_db();
        db.ensure_user(1);
        let result = db.add_sub(1, KIND_MAP, "Pudge");
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    #[test]
    fn add_sub_duplicate_returns_err() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let result = db.add_sub(1, KIND_MAP, "Pudge");
        assert!(result.is_err());
    }

    #[test]
    fn add_sub_same_pattern_different_kind() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.add_sub(1, KIND_MAP, "X").unwrap());
        assert!(db.add_sub(1, KIND_HOST, "X").unwrap());
        assert!(db.add_sub(1, KIND_NAME, "X").unwrap());
        assert_eq!(db.list_subs(1).len(), 3);
    }

    #[test]
    fn add_sub_same_pattern_different_users() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        assert!(db.add_sub(1, KIND_MAP, "Pudge").unwrap());
        assert!(db.add_sub(2, KIND_MAP, "Pudge").unwrap());
        assert_eq!(db.list_subs(1).len(), 1);
        assert_eq!(db.list_subs(2).len(), 1);
    }

    #[test]
    fn list_subs_empty_for_unknown_user() {
        let db = memory_db();
        assert!(db.list_subs(999).is_empty());
    }

    #[test]
    fn list_subs_ordered_by_id() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "B_Map");
        let _ = db.add_sub(1, KIND_HOST, "A_Host");
        let _ = db.add_sub(1, KIND_NAME, "C_Name");
        let subs = db.list_subs(1);
        assert_eq!(subs.len(), 3);
        assert_eq!(subs[0].pattern, "B_Map");
        assert_eq!(subs[1].pattern, "A_Host");
        assert_eq!(subs[2].pattern, "C_Name");
    }

    #[test]
    fn get_sub_existing() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "TestMap");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        let fetched = db.get_sub(sub.id);
        assert!(fetched.is_some());
        let s = fetched.unwrap();
        assert_eq!(s.id, sub.id);
        assert_eq!(s.pattern, "TestMap");
        assert_eq!(s.kind, KIND_MAP);
        assert!(s.enabled);
    }

    #[test]
    fn get_sub_nonexistent() {
        let db = memory_db();
        assert!(db.get_sub(999).is_none());
    }

    #[test]
    fn set_sub_enabled() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "TestMap");
        let sub = db.list_subs(1).into_iter().next().unwrap();

        assert!(db.set_sub_enabled(sub.id, false));
        let s = db.get_sub(sub.id).unwrap();
        assert!(!s.enabled);

        assert!(db.set_sub_enabled(sub.id, true));
        let s = db.get_sub(sub.id).unwrap();
        assert!(s.enabled);
    }

    #[test]
    fn set_sub_enabled_nonexistent_returns_false() {
        let db = memory_db();
        assert!(!db.set_sub_enabled(999, true));
    }

    #[test]
    fn rename_sub() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "OldName");
        let sub = db.list_subs(1).into_iter().next().unwrap();

        assert!(db.rename_sub(sub.id, "NewName").is_ok());
        let s = db.get_sub(sub.id).unwrap();
        assert_eq!(s.pattern, "NewName");
    }

    #[test]
    fn rename_sub_nonexistent_is_ok() {
        let db = memory_db();
        // rename non-existent sub returns Ok (0 rows affected)
        assert!(db.rename_sub(999, "X").is_ok());
    }

    #[test]
    fn delete_sub_returns_true_when_exists() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "ToDelete");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        assert!(db.delete_sub(sub.id));
        assert!(db.get_sub(sub.id).is_none());
    }

    #[test]
    fn delete_sub_returns_false_when_nonexistent() {
        let db = memory_db();
        assert!(!db.delete_sub(999));
    }

    #[test]
    fn delete_sub_isolation_between_users() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        let _ = db.add_sub(1, KIND_MAP, "Shared");
        let _ = db.add_sub(2, KIND_MAP, "Shared");
        let sub1 = db.list_subs(1).into_iter().next().unwrap();
        db.delete_sub(sub1.id);
        assert!(db.list_subs(1).is_empty());
        assert_eq!(db.list_subs(2).len(), 1);
    }

    // ===================== map mute tests =====================

    #[test]
    fn snooze_sub_sets_until() {
        let db = memory_db();
        db.ensure_user(1);
        let before = now_ts();
        db.snooze_sub(1, KIND_MAP, "Snoozed");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 1);
        assert_eq!(mutes[0].kind, KIND_MAP);
        assert_eq!(mutes[0].pattern, "Snoozed");
        let until = mutes[0].until.unwrap();
        assert!(until > before);
        assert!(until <= before + SNOOZE_SECS + 1);
    }

    #[test]
    fn snooze_sub_overwrites_previous() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_sub(1, KIND_MAP, "X");
        let until1 = db.list_map_mutes(1)[0].until.unwrap();
        db.snooze_sub(1, KIND_MAP, "X");
        let until2 = db.list_map_mutes(1)[0].until.unwrap();
        assert!(until2 >= until1);
    }

    #[test]
    fn mute_sub_sets_null_until() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_sub(1, KIND_MAP, "PermMuted");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 1);
        assert_eq!(mutes[0].pattern, "PermMuted");
        assert!(mutes[0].until.is_none());
    }

    #[test]
    fn mute_sub_overwrites_snooze() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_sub(1, KIND_MAP, "X");
        assert!(db.list_map_mutes(1)[0].until.is_some());
        db.mute_sub(1, KIND_MAP, "X");
        assert!(db.list_map_mutes(1)[0].until.is_none());
    }

    #[test]
    fn list_map_mutes_empty() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.list_map_mutes(1).is_empty());
    }

    #[test]
    fn list_map_mutes_ordered_by_kind_pattern() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_sub(1, KIND_MAP, "Zebra");
        db.mute_sub(1, KIND_MAP, "Alpha");
        db.mute_sub(1, KIND_NAME, "Middle");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 3);
        // ORDER BY kind, pattern: "map" < "name" lexicographically.
        assert_eq!(mutes[0].kind, KIND_MAP);
        assert_eq!(mutes[0].pattern, "Alpha");
        assert_eq!(mutes[1].kind, KIND_MAP);
        assert_eq!(mutes[1].pattern, "Zebra");
        assert_eq!(mutes[2].kind, KIND_NAME);
        assert_eq!(mutes[2].pattern, "Middle");
    }

    #[test]
    fn mute_scoped_to_kind() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_sub(1, KIND_MAP, "Pudge");
        assert!(db.is_sub_muted(1, KIND_MAP, "Pudge"));
        // Same pattern under a different kind is independent
        assert!(!db.is_sub_muted(1, KIND_NAME, "Pudge"));
        assert!(!db.is_sub_muted(1, KIND_HOST, "Pudge"));
    }

    #[test]
    fn release_expired_snoozes() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_sub(1, KIND_MAP, "Expiring");

        // Manually set until to the past
        db.0.lock().unwrap().execute(
            "UPDATE map_mutes SET until = 1 WHERE chat_id = 1",
            [],
        ).unwrap();

        db.release_expired_map_snoozes();
        assert!(!db.is_sub_muted(1, KIND_MAP, "Expiring"));
    }

    #[test]
    fn release_expired_snoozes_keeps_active() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_sub(1, KIND_MAP, "StillActive");

        // until is far in the future
        db.release_expired_map_snoozes();
        assert!(db.is_sub_muted(1, KIND_MAP, "StillActive"));
    }

    #[test]
    fn release_expired_snoozes_keeps_permanent_mutes() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_sub(1, KIND_MAP, "Permanent");

        db.release_expired_map_snoozes();
        assert!(db.is_sub_muted(1, KIND_MAP, "Permanent"));
    }

    #[test]
    fn is_sub_muted_false_when_not_muted() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(!db.is_sub_muted(1, KIND_MAP, "NotMuted"));
    }

    #[test]
    fn is_sub_muted_isolation_between_users() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        db.mute_sub(1, KIND_MAP, "X");
        assert!(db.is_sub_muted(1, KIND_MAP, "X"));
        assert!(!db.is_sub_muted(2, KIND_MAP, "X"));
    }

    // ===================== delete_user tests =====================

    #[test]
    fn delete_user_removes_user() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.notifications_enabled(1));
        db.delete_user(1);
        // unknown user returns false
        assert!(!db.notifications_enabled(1));
    }

    #[test]
    fn delete_user_cascades_subs() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Map");
        let _ = db.add_sub(1, KIND_HOST, "Host");
        assert_eq!(db.list_subs(1).len(), 2);
        db.delete_user(1);
        assert!(db.list_subs(1).is_empty());
    }

    #[test]
    fn delete_user_cascades_map_mutes() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_sub(1, KIND_MAP, "Muted");
        assert!(db.is_sub_muted(1, KIND_MAP, "Muted"));
        db.delete_user(1);
        assert!(!db.is_sub_muted(1, KIND_MAP, "Muted"));
    }

    #[test]
    fn delete_user_cascades_sub_hosts() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Map");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.add_sub_host(sub.id, "Host");
        assert!(!db.get_host_filter(sub.id).1.is_empty());
        db.delete_user(1);
        // sub is gone, get_host_filter returns empty
        assert!(db.get_host_filter(sub.id).1.is_empty());
    }

    // ===================== all_active_subs tests =====================

    #[test]
    fn all_active_subs_filters_disabled_user() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        let _ = db.add_sub(1, KIND_MAP, "Map1");
        let _ = db.add_sub(2, KIND_MAP, "Map2");
        db.set_notifications(2, false);

        let active = db.all_active_subs();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].sub.pattern, "Map1");
    }

    #[test]
    fn all_active_subs_filters_disabled_sub() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Enabled");
        let _ = db.add_sub(1, KIND_MAP, "Disabled");
        let sub = db.list_subs(1).iter().find(|s| s.pattern == "Disabled").unwrap().clone();
        db.set_sub_enabled(sub.id, false);

        let active = db.all_active_subs();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].sub.pattern, "Enabled");
    }

    #[test]
    fn all_active_subs_empty() {
        let db = memory_db();
        assert!(db.all_active_subs().is_empty());
    }

    #[test]
    fn all_active_subs_multiple_kinds() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Map");
        let _ = db.add_sub(1, KIND_HOST, "Host");
        let _ = db.add_sub(1, KIND_NAME, "Name");

        let active = db.all_active_subs();
        assert_eq!(active.len(), 3);
    }

    #[test]
    fn all_active_subs_respects_notifications_flag() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Map");
        assert_eq!(db.all_active_subs().len(), 1);
        db.set_notifications(1, false);
        assert_eq!(db.all_active_subs().len(), 0);
        db.set_notifications(1, true);
        assert_eq!(db.all_active_subs().len(), 1);
    }

    // ===================== integration: notification pipeline =====================

    /// Simulates the poller matching logic for a single (game, subscription) pair.
    /// Returns true if the notification should be sent.
    fn should_notify(db: &Db, game: &crate::api::Game, sub: &Sub) -> bool {
        db.should_notify(sub, &game.map, &game.host, &game.name)
    }

    /// Returns the list of subscriptions that would trigger a notification for a given game.
    fn matching_subs(db: &Db, game: &crate::api::Game) -> Vec<Sub> {
        db.all_active_subs()
            .into_iter()
            .filter(|a| should_notify(db, game, &a.sub))
            .map(|a| a.sub)
            .collect()
    }

    fn game(id: i64, map: &str, host: &str, name: &str) -> crate::api::Game {
        crate::api::Game {
            id,
            name: name.to_string(),
            host: host.to_string(),
            map: map.to_string(),
            server: "europe".to_string(),
            slots_taken: 2,
            slots_total: 10,
            created: 1000,
        }
    }

    // --- basic matching ---

    #[test]
    fn integration_map_sub_matches_game() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "Pudge Wars v2", "Host", "My Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_map_sub_no_match() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Dota");
        let g = game(1, "Pudge Wars", "Host", "My Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_host_sub_matches_game() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_HOST, "HellWolf");
        let g = game(1, "Some Map", "HellWolf#31976", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_host_sub_no_match() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_HOST, "HellWolf");
        let g = game(1, "Some Map", "SomeOtherGuy", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_name_sub_matches_game() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_NAME, "fun");
        let g = game(1, "Map", "Host", "Fun Game 5v5");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_name_sub_no_match() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_NAME, "fun");
        let g = game(1, "Map", "Host", "Serious Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    // --- fuzzy matching ---

    #[test]
    fn integration_fuzzy_case_insensitive() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "CHS");
        let g = game(1, "chs - fast game", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_fuzzy_symbol_insensitive() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Legion TD");
        let g = game(1, "Legion_TD_11.4b.w3x", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    // --- disabled sub ---

    #[test]
    fn integration_disabled_sub_no_notify() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_sub_enabled(sub.id, false);
        let g = game(1, "Pudge Wars", "Host", "Game");
        // all_active_subs filters disabled subs
        assert!(matching_subs(&db, &g).is_empty());
    }

    // --- disabled notifications ---

    #[test]
    fn integration_disabled_notifications_no_notify() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.set_notifications(1, false);
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    // --- muted selector ---

    #[test]
    fn integration_muted_selector_no_notify() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.mute_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_muted_selector_different_selector_still_notifies() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_MAP, "Dota");
        db.mute_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "Dota 2", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_muted_selector_blocks_all_games_matching_selector() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.mute_sub(1, KIND_MAP, "Pudge");
        // fuzzy: "Pudge" matches both games
        let g1 = game(1, "Pudge Wars v2", "Host", "Game");
        let g2 = game(2, "Pudge Wars 3", "Host", "Game");
        assert!(matching_subs(&db, &g1).is_empty());
        assert!(matching_subs(&db, &g2).is_empty());
    }

    #[test]
    fn integration_mute_does_not_affect_other_kind() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_NAME, "Pudge");
        db.mute_sub(1, KIND_MAP, "Pudge");
        // name sub still fires
        let g = game(1, "Other Map", "Host", "Pudge Cup");
        assert_eq!(matching_subs(&db, &g).len(), 1);
        // map sub muted
        let g2 = game(2, "Pudge Wars", "Host", "Other Name");
        assert_eq!(matching_subs(&db, &g2).len(), 0);
    }

    // --- snooze ---

    #[test]
    fn integration_snooze_blocks_notification() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.snooze_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_snooze_expired_restores_notification() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.snooze_sub(1, KIND_MAP, "Pudge");
        // expire the snooze
        db.0.lock().unwrap().execute(
            "UPDATE map_mutes SET until = 1 WHERE chat_id = 1",
            [],
        ).unwrap();
        db.release_expired_map_snoozes();
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    // --- host filter ---

    #[test]
    fn integration_whitelist_allows_matching_host() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");
        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_whitelist_blocks_unknown_host() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");
        let g = game(1, "Pudge Wars", "RandomGuy", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_blacklist_blocks_listed_host() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "HellWolf");
        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_blacklist_allows_unknown_host() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "HellWolf");
        let g = game(1, "Pudge Wars", "RandomGuy", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_host_filter_does_not_apply_to_host_subs() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_HOST, "HellWolf");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        // host filter on a host sub should not affect matching
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "OtherHost");
        let g = game(1, "Map", "HellWolf#31976", "Game");
        // host sub matches by host name, host filter doesn't apply
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_empty_whitelist_blocks_all() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        // no hosts added
        let g = game(1, "Pudge Wars", "Anyone", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    // --- multi-user ---

    #[test]
    fn integration_multi_user_both_notified() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(2, KIND_MAP, "Pudge");
        let g = game(1, "Pudge Wars", "Host", "Game");
        let active = db.all_active_subs();
        let matching: Vec<_> = active
            .iter()
            .filter(|a| should_notify(&db, &g, &a.sub))
            .collect();
        assert_eq!(matching.len(), 2);
    }

    #[test]
    fn integration_multi_user_one_disabled() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(2, KIND_MAP, "Pudge");
        db.set_notifications(2, false);
        let g = game(1, "Pudge Wars", "Host", "Game");
        let active = db.all_active_subs();
        let matching: Vec<_> = active
            .iter()
            .filter(|a| should_notify(&db, &g, &a.sub))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].sub.chat_id, 1);
    }

    // --- multi-sub ---

    #[test]
    fn integration_game_matches_multiple_subs() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_HOST, "HellWolf");
        let _ = db.add_sub(1, KIND_NAME, "Fun");
        let g = game(1, "Pudge Wars", "HellWolf#31976", "Fun Game");
        let active = db.all_active_subs();
        let matching: Vec<_> = active
            .iter()
            .filter(|a| should_notify(&db, &g, &a.sub))
            .collect();
        assert_eq!(matching.len(), 3);
    }

    #[test]
    fn integration_game_matches_only_two_of_three() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_HOST, "HellWolf");
        let _ = db.add_sub(1, KIND_NAME, "Fun");
        let g = game(1, "Pudge Wars", "OtherHost", "Fun Game");
        let active = db.all_active_subs();
        let matching: Vec<_> = active
            .iter()
            .filter(|a| should_notify(&db, &g, &a.sub))
            .collect();
        assert_eq!(matching.len(), 2);
    }

    // --- combined filters ---

    #[test]
    fn integration_mute_plus_whitelist() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");
        db.mute_sub(1, KIND_MAP, "Pudge");

        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        // muted → blocked even though host matches whitelist
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_whitelist_plus_different_muted_selector() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_MAP, "Dota");
        let map_sub = db.list_subs(1).iter().find(|s| s.pattern == "Pudge").unwrap().clone();
        db.set_host_filter_mode(map_sub.id, HF_WHITELIST);
        db.add_sub_host(map_sub.id, "HellWolf");
        // mute a different selector
        db.mute_sub(1, KIND_MAP, "Dota");

        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        // Pudge not muted → notified
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_blacklist_plus_disabled_sub() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "HellWolf");
        db.set_sub_enabled(sub.id, false);

        let g = game(1, "Pudge Wars", "RandomGuy", "Game");
        // sub disabled → no notification
        assert!(matching_subs(&db, &g).is_empty());
    }

    // --- edge cases ---

    #[test]
    fn integration_empty_game_fields() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "", "", "");
        // empty map doesn't match "Pudge"
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_no_subs_no_notifications() {
        let db = memory_db();
        db.ensure_user(1);
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_delete_sub_stops_notification() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.delete_sub(sub.id);
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_toggle_sub_back_and_forth() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        let g = game(1, "Pudge Wars", "Host", "Game");

        assert_eq!(matching_subs(&db, &g).len(), 1);
        db.set_sub_enabled(sub.id, false);
        assert!(matching_subs(&db, &g).is_empty());
        db.set_sub_enabled(sub.id, true);
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    #[test]
    fn integration_rename_sub_changes_match() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        let g = game(1, "Pudge Wars", "Host", "Game");

        assert_eq!(matching_subs(&db, &g).len(), 1);
        db.rename_sub(sub.id, "Dota").unwrap();
        assert!(matching_subs(&db, &g).is_empty());
        // now matches a different game
        let g2 = game(2, "Dota 2", "Host", "Game");
        assert_eq!(matching_subs(&db, &g2).len(), 1);
    }

    #[test]
    fn integration_snooze_only_blocks_matching_selector() {
        let db = memory_db();
        db.ensure_user(1);
        // Two separate selectors; mute only one of them
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(1, KIND_MAP, "Dota");
        db.snooze_sub(1, KIND_MAP, "Pudge");

        let g1 = game(1, "Pudge Wars", "Host", "Game");
        let g2 = game(2, "Dota 2", "Host", "Game");
        assert!(matching_subs(&db, &g1).is_empty());
        assert_eq!(matching_subs(&db, &g2).len(), 1);
    }

    #[test]
    fn integration_multiple_users_different_mutes() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let _ = db.add_sub(2, KIND_MAP, "Pudge");
        db.mute_sub(1, KIND_MAP, "Pudge");
        // user 2 does not mute

        let g = game(1, "Pudge Wars", "Host", "Game");
        let active = db.all_active_subs();
        let matching: Vec<_> = active
            .iter()
            .filter(|a| should_notify(&db, &g, &a.sub))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].sub.chat_id, 2);
    }

    #[test]
    fn integration_host_filter_multiple_hosts() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "Host1");
        db.add_sub_host(sub.id, "Host2");

        let g1 = game(1, "Pudge", "Host1", "Game");
        let g2 = game(2, "Pudge", "Host2", "Game");
        let g3 = game(3, "Pudge", "Host3", "Game");
        assert_eq!(matching_subs(&db, &g1).len(), 1);
        assert_eq!(matching_subs(&db, &g2).len(), 1);
        assert!(matching_subs(&db, &g3).is_empty());
    }

    #[test]
    fn integration_blacklist_multiple_hosts() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_BLACKLIST);
        db.add_sub_host(sub.id, "BadHost1");
        db.add_sub_host(sub.id, "BadHost2");

        let g1 = game(1, "Pudge", "BadHost1", "Game");
        let g2 = game(2, "Pudge", "BadHost2", "Game");
        let g3 = game(3, "Pudge", "GoodHost", "Game");
        assert!(matching_subs(&db, &g1).is_empty());
        assert!(matching_subs(&db, &g2).is_empty());
        assert_eq!(matching_subs(&db, &g3).len(), 1);
    }

    #[test]
    fn integration_remove_host_from_whitelist() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");

        let g = game(1, "Pudge", "HellWolf#31976", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
        db.remove_sub_host(sub.id, "HellWolf");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_game_id_used_for_dedup_not_in_logic() {
        // game.id is used by the poller's `seen` set for dedup,
        // but should_notify doesn't care about it
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let g1 = game(42, "Pudge Wars", "Host", "Game");
        let g2 = game(42, "Pudge Wars", "Host", "Game");
        // same id, same data → both match (dedup is poller's job)
        assert_eq!(matching_subs(&db, &g1).len(), 1);
        assert_eq!(matching_subs(&db, &g2).len(), 1);
    }

    // ===================== quiet hours tests =====================

    #[test]
    fn notification_window_default_none() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.get_quiet_hours(1).is_none());
        // без настроенного окна — уведомления работают всегда
        assert!(db.is_in_notification_window(1));
    }

    #[test]
    fn notification_window_set_and_get() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);
        let qh = db.get_quiet_hours(1).unwrap();
        assert_eq!(qh.tz_offset_min, 180);
        assert_eq!(qh.start_min, 23 * 60);
        assert_eq!(qh.end_min, 7 * 60);
    }

    #[test]
    fn notification_window_disable_clears() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);
        assert!(db.get_quiet_hours(1).is_some());
        db.disable_quiet_hours(1);
        assert!(db.get_quiet_hours(1).is_none());
        // после disable — уведомления снова работают всегда
        assert!(db.is_in_notification_window(1));
    }

    #[test]
    fn notification_window_partial_clear() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);
        // очистим только tz_offset вручную через SQL
        db.0.lock().unwrap()
            .execute("UPDATE users SET qh_tz_offset = NULL WHERE chat_id = 1", [])
            .unwrap();
        // теперь это None, потому что не все три поля заданы
        assert!(db.get_quiet_hours(1).is_none());
    }

    #[test]
    fn notification_window_isolation_between_users() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);
        assert!(db.get_quiet_hours(1).is_some());
        assert!(db.get_quiet_hours(2).is_none());
    }

    #[test]
    fn integration_notification_window_zero_interval_allows_all() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.set_quiet_hours(1, 0, 0, 0); // 00:00–00:00 = фича выключена
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    /// Подделывает «сейчас» через приватный канал: у DB нет метода set_now,
    /// поэтому интеграционные тесты ниже зависят от системного времени.
    /// Чтобы тесты были детерминированы, мы зовём `is_in_notification_window`
    /// напрямую с конкретным таймстампом.
    fn notification_window_at(db: &Db, uid: i64, ts: i64) -> bool {
        let qh = match db.get_quiet_hours(uid) {
            Some(q) => q,
            None => return true,
        };
        crate::quiet::is_in_notification_window(ts, qh.tz_offset_min, qh.start_min, qh.end_min)
    }

    #[test]
    fn integration_notification_window_e2e_user_in_utc() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        // окно уведомлений 09:00–18:00 UTC
        db.set_quiet_hours(1, 0, 9 * 60, 18 * 60);

        // 12:00 UTC → внутри окна
        assert!(notification_window_at(&db, 1, 1_705_320_000));
        // 22:00 UTC → снаружи окна
        assert!(!notification_window_at(&db, 1, 1_705_356_000));
    }

    #[test]
    fn integration_notification_window_e2e_user_in_msk_cross_midnight() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        // 23:00–07:00 MSK = UTC+3
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);

        // 23:30 MSK = 20:30 UTC → внутри
        assert!(notification_window_at(&db, 1, 20 * 3600 + 30 * 60));
        // 02:00 MSK = 23:00 UTC предыдущего дня → внутри
        assert!(notification_window_at(&db, 1, 23 * 3600));
        // 03:00 MSK = 00:00 UTC → внутри
        assert!(notification_window_at(&db, 1, 0));
        // 07:00 MSK = 04:00 UTC → не внутри (конец не включается)
        assert!(!notification_window_at(&db, 1, 4 * 3600));
        // 08:00 MSK = 05:00 UTC → снаружи
        assert!(!notification_window_at(&db, 1, 5 * 3600));
        // 22:00 MSK = 19:00 UTC → снаружи
        assert!(!notification_window_at(&db, 1, 19 * 3600));
    }

    #[test]
    fn integration_notification_window_e2e_user_in_la_negative_offset() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        // 22:00–06:00 LA = UTC-7 для теста
        db.set_quiet_hours(1, -420, 22 * 60, 6 * 60);

        // 02:00 LA = 09:00 UTC → внутри
        assert!(notification_window_at(&db, 1, 9 * 3600));
        // 23:00 LA = 06:00 UTC предыдущего дня → внутри
        assert!(notification_window_at(&db, 1, 6 * 3600));
        // 12:00 LA = 19:00 UTC → снаружи
        assert!(!notification_window_at(&db, 1, 19 * 3600));
    }

    #[test]
    fn integration_notification_window_e2e_partial_timezone_fractional() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        // India: UTC+5:30
        db.set_quiet_hours(1, 5 * 60 + 30, 22 * 60, 6 * 60);

        // 02:00 IST = 20:30 UTC предыдущего дня → внутри
        assert!(notification_window_at(&db, 1, 20 * 3600 + 30 * 60));
        // 23:00 IST = 17:30 UTC → внутри
        assert!(notification_window_at(&db, 1, 17 * 3600 + 30 * 60));
        // 07:00 IST = 01:30 UTC → снаружи (после 06:00)
        assert!(!notification_window_at(&db, 1, 1 * 3600 + 30 * 60));
    }

    #[test]
    fn integration_notification_window_e2e_disabled_via_zero_interval() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 0, 0);
        // start == end → фича выключена → окно = всё время → всегда true
        assert!(notification_window_at(&db, 1, 0));
        assert!(notification_window_at(&db, 1, 12 * 3600));
        assert!(notification_window_at(&db, 1, 1_705_356_000));
    }

    #[test]
    fn integration_notification_window_e2e_disable_clears_at_any_time() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 0, 24 * 60 - 1);
        // что-то в течение суток → внутри
        assert!(notification_window_at(&db, 1, 12 * 3600));
        db.disable_quiet_hours(1);
        // после disable — окно не настроено → всегда true
        assert!(notification_window_at(&db, 1, 12 * 3600));
    }

    #[test]
    fn integration_notification_window_e2e_survives_reopen() {
        let (db, path) = temp_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 180, 23 * 60, 7 * 60);
        let stored = db.get_quiet_hours(1).unwrap();
        drop(db);

        let db = Db::open(&path).unwrap();
        let restored = db.get_quiet_hours(1).unwrap();
        assert_eq!(restored.tz_offset_min, stored.tz_offset_min);
        assert_eq!(restored.start_min, stored.start_min);
        assert_eq!(restored.end_min, stored.end_min);
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn integration_notification_window_e2e_independent_per_user() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        db.set_quiet_hours(1, 180, 0, 0); // фича выключена
        db.set_quiet_hours(2, 0, 0, 24 * 60); // весь день
        // 12:00 UTC
        let ts = 12 * 3600;
        assert!(notification_window_at(&db, 1, ts));
        assert!(notification_window_at(&db, 2, ts));
    }

    #[test]
    fn integration_quiet_hours_e2e_overwrite() {
        let db = memory_db();
        db.ensure_user(1);
        db.set_quiet_hours(1, 0, 9 * 60, 18 * 60);
        db.set_quiet_hours(1, 180, 22 * 60, 8 * 60); // перезаписали
        let qh = db.get_quiet_hours(1).unwrap();
        assert_eq!(qh.tz_offset_min, 180);
        assert_eq!(qh.start_min, 22 * 60);
        assert_eq!(qh.end_min, 8 * 60);
    }
}

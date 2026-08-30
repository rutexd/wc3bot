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

    /// Delete a per-map mute entry. Called when a map subscription is removed.
    pub fn delete_map_mute(&self, chat_id: i64, map: &str) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM map_mutes WHERE chat_id = ?1 AND map = ?2",
                params![chat_id, map],
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

        db.mute_map(uid, "Tetris");
        assert!(db.is_map_muted(uid, "Tetris"));

        db.delete_sub(sub.id);
        if sub.kind == KIND_MAP {
            db.delete_map_mute(uid, &sub.pattern);
        }

        assert!(!db.is_map_muted(uid, "Tetris"));
    }

    #[test]
    fn delete_map_mute_removes_entry() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        db.mute_map(uid, "SomeMap");
        assert!(db.is_map_muted(uid, "SomeMap"));

        db.delete_map_mute(uid, "SomeMap");
        assert!(!db.is_map_muted(uid, "SomeMap"));
    }

    #[test]
    fn delete_host_sub_does_not_touch_mutes() {
        let db = memory_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_HOST, "SomeHost");
        let sub = db.list_subs(uid).into_iter().next().unwrap();

        db.mute_map(uid, "SomeHost");

        db.delete_sub(sub.id);
        assert!(db.is_map_muted(uid, "SomeHost"));
    }

    #[test]
    fn migration_cleans_orphaned_map_mutes() {
        let (db, path) = temp_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "GhostTower");
        db.mute_map(uid, "GhostTower");

        // Force-delete sub bypassing delete_sub (simulates old buggy path)
        db.0.lock()
            .unwrap()
            .execute("DELETE FROM subs WHERE chat_id = ?1", params![uid])
            .unwrap();
        assert!(db.is_map_muted(uid, "GhostTower"));
        drop(db);

        // Re-open: migration should clean orphaned mutes
        let db = Db::open(&path).unwrap();
        assert!(!db.is_map_muted(uid, "GhostTower"));
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn migration_keeps_valid_map_mutes() {
        let (db, path) = temp_db();
        let uid = 1;
        db.ensure_user(uid);
        let _ = db.add_sub(uid, KIND_MAP, "ValidMap");
        db.mute_map(uid, "ValidMap");
        drop(db);

        // Re-open: valid mute should survive
        let db = Db::open(&path).unwrap();
        assert!(db.is_map_muted(uid, "ValidMap"));
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
    fn snooze_map_sets_until() {
        let db = memory_db();
        db.ensure_user(1);
        let before = now_ts();
        db.snooze_map(1, "SnoozedMap");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 1);
        assert_eq!(mutes[0].map, "SnoozedMap");
        let until = mutes[0].until.unwrap();
        assert!(until > before);
        assert!(until <= before + SNOOZE_SECS + 1);
    }

    #[test]
    fn snooze_map_overwrites_previous() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_map(1, "Map");
        let until1 = db.list_map_mutes(1)[0].until.unwrap();
        db.snooze_map(1, "Map");
        let until2 = db.list_map_mutes(1)[0].until.unwrap();
        assert!(until2 >= until1);
    }

    #[test]
    fn mute_map_sets_null_until() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_map(1, "PermMuted");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 1);
        assert_eq!(mutes[0].map, "PermMuted");
        assert!(mutes[0].until.is_none());
    }

    #[test]
    fn mute_map_overwrites_snooze() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_map(1, "Map");
        assert!(db.list_map_mutes(1)[0].until.is_some());
        db.mute_map(1, "Map");
        assert!(db.list_map_mutes(1)[0].until.is_none());
    }

    #[test]
    fn list_map_mutes_empty() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(db.list_map_mutes(1).is_empty());
    }

    #[test]
    fn list_map_mutes_ordered_by_map() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_map(1, "Zebra");
        db.mute_map(1, "Alpha");
        db.mute_map(1, "Middle");
        let mutes = db.list_map_mutes(1);
        assert_eq!(mutes.len(), 3);
        assert_eq!(mutes[0].map, "Alpha");
        assert_eq!(mutes[1].map, "Middle");
        assert_eq!(mutes[2].map, "Zebra");
    }

    #[test]
    fn release_expired_snoozes() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_map(1, "Expiring");

        // Manually set until to the past
        db.0.lock().unwrap().execute(
            "UPDATE map_mutes SET until = 1 WHERE chat_id = 1",
            [],
        ).unwrap();

        db.release_expired_map_snoozes();
        assert!(!db.is_map_muted(1, "Expiring"));
    }

    #[test]
    fn release_expired_snoozes_keeps_active() {
        let db = memory_db();
        db.ensure_user(1);
        db.snooze_map(1, "StillActive");

        // until is far in the future
        db.release_expired_map_snoozes();
        assert!(db.is_map_muted(1, "StillActive"));
    }

    #[test]
    fn release_expired_snoozes_keeps_permanent_mutes() {
        let db = memory_db();
        db.ensure_user(1);
        db.mute_map(1, "Permanent");

        db.release_expired_map_snoozes();
        assert!(db.is_map_muted(1, "Permanent"));
    }

    #[test]
    fn is_map_muted_false_when_not_muted() {
        let db = memory_db();
        db.ensure_user(1);
        assert!(!db.is_map_muted(1, "NotMuted"));
    }

    #[test]
    fn is_map_muted_isolation_between_users() {
        let db = memory_db();
        db.ensure_user(1);
        db.ensure_user(2);
        db.mute_map(1, "Map");
        assert!(db.is_map_muted(1, "Map"));
        assert!(!db.is_map_muted(2, "Map"));
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
        db.mute_map(1, "Muted");
        assert!(db.is_map_muted(1, "Muted"));
        db.delete_user(1);
        assert!(!db.is_map_muted(1, "Muted"));
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
        // 1. match subscription pattern
        let matched = match sub.kind.as_str() {
            KIND_HOST => crate::norm::matches(&sub.pattern, &game.host),
            KIND_NAME => crate::norm::matches(&sub.pattern, &game.name),
            _ => crate::norm::matches(&sub.pattern, &game.map),
        };
        if !matched {
            return false;
        }
        // 2. check map mute
        if db.is_map_muted(sub.chat_id, &game.map) {
            return false;
        }
        // 3. host filter (map/name only)
        if (sub.kind == KIND_MAP || sub.kind == KIND_NAME)
            && !db.host_filter_passes(sub.id, &game.host)
        {
            return false;
        }
        true
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

    // --- muted map ---

    #[test]
    fn integration_muted_map_no_notify() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.mute_map(1, "Pudge Wars");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_muted_map_different_map_still_notifies() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.mute_map(1, "Some Other Map");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert_eq!(matching_subs(&db, &g).len(), 1);
    }

    // --- snooze ---

    #[test]
    fn integration_snooze_blocks_notification() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.snooze_map(1, "Pudge Wars");
        let g = game(1, "Pudge Wars", "Host", "Game");
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_snooze_expired_restores_notification() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.snooze_map(1, "Pudge Wars");
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
        db.mute_map(1, "Pudge Wars");

        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        // muted → blocked even though host matches whitelist
        assert!(matching_subs(&db, &g).is_empty());
    }

    #[test]
    fn integration_whitelist_plus_different_muted_map() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        let sub = db.list_subs(1).into_iter().next().unwrap();
        db.set_host_filter_mode(sub.id, HF_WHITELIST);
        db.add_sub_host(sub.id, "HellWolf");
        db.mute_map(1, "Other Map");

        let g = game(1, "Pudge Wars", "HellWolf#31976", "Game");
        // different map muted, this map + host ok → notified
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
    fn integration_snooze_only_this_map() {
        let db = memory_db();
        db.ensure_user(1);
        let _ = db.add_sub(1, KIND_MAP, "Pudge");
        db.snooze_map(1, "Pudge Wars");

        let g1 = game(1, "Pudge Wars", "Host", "Game");
        let g2 = game(2, "Pudge Wars 2", "Host", "Game");
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
        db.mute_map(1, "Pudge Wars");
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
}

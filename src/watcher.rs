use std::collections::{HashMap, HashSet};

use crate::api::Game;
use crate::db::UserSettings;

/// Snapshot of a game kept alongside watch state so the monitor screen
/// can show "Map by Host вЂ” taken/total" without re-querying the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedGame {
    pub id: i64,
    pub map: String,
    pub host: String,
    pub taken: i32,
    pub total: i32,
}

/// Per-user state for the watcher.
#[derive(Debug, Default)]
pub struct UserWatch {
    /// `game_id` в†’ `slots_taken` at the previous tick. Missing entry means
    /// "we haven't seen this game yet" вЂ” first tick will emit `Started`.
    pub last_taken: HashMap<i64, i32>,
    /// `game_id` в†’ `slots_total` at the previous tick. Needed to detect
    /// transitions to "full" in `WatchEvent::Filled`.
    pub last_total: HashMap<i64, i32>,
    /// Games the user has explicitly subscribed to (via the "Watch" button
    /// on a notification). Independent of nickname-mode.
    pub explicit_games: HashMap<i64, WatchedGame>,
}

/// State of the entire watcher (across all users).
#[derive(Debug, Default)]
pub struct WatchState {
    pub users: HashMap<i64, UserWatch>,
}

impl WatchState {
    pub fn user_mut(&mut self, user_id: i64) -> &mut UserWatch {
        self.users.entry(user_id).or_default()
    }
}

/// What to render to a user as a result of a single tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// First time we see a game this user is monitoring.
    /// `nickname` = true if triggered by nickname-mode, false if explicit.
    Started {
        game: Game,
        nickname: bool,
    },
    /// `delta` slots changed in a single tick. Always aggregated.
    /// `delta > 0` = players joined, `delta < 0` = players left.
    SlotsDelta {
        game: Game,
        delta: i32,
    },
    /// The lobby just filled up. Sent exactly once per (user, game) вЂ” even
    /// if slots fluctuate later (a player leaves and rejoins), we don't
    /// re-emit. Tracked implicitly: once both `last_taken == slots_total`
    /// and `slots_taken == slots_total`, the next time we see the game not
    /// full we allow another Filled emission.
    Filled {
        game: Game,
    },
}

/// Parse a WC3 host identity of the form "Name#12345" into (name, id).
/// Validation rules:
/// - exactly one '#'
/// - non-empty name (no '#' allowed inside the name)
/// - digits-only id, length 1..=10
/// - id must be > 0
pub fn parse_identity(s: &str) -> Result<(String, u32), &'static str> {
    let (name, id) = s.split_once('#').ok_or("expected format Name#12345")?;
    if name.is_empty() {
        return Err("name is empty");
    }
    if name.contains('#') {
        return Err("name must not contain '#'");
    }
    if id.is_empty() || id.len() > 10 {
        return Err("id length out of range");
    }
    if !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("id must contain digits only");
    }
    let parsed: u32 = id.parse().map_err(|_| "id parse error")?;
    if parsed == 0 {
        return Err("id must be > 0");
    }
    Ok((name.to_string(), parsed))
}

/// Should the "рџ‘Ѓ Watch" button be shown for this (user, game) pair in the
/// notification keyboard? It's hidden when the user has a watched identity
/// that matches the game host (since nickname-mode already covers it) or
/// when monitoring is disabled but the identity is set without enabling
/// (so we don't pester the user with a button that does nothing).
pub fn should_show_watch_button(settings: &UserSettings, game: &Game) -> bool {
    let Some(identity) = settings.watched_identity.as_deref() else {
        return true;
    };
    if !settings.monitoring_enabled {
        return true;
    }
    // identity is set AND monitoring enabled в†’ hide if game host matches.
    let Ok((name, id)) = parse_identity(identity) else {
        return true;
    };
    !host_matches(&name, id, &game.host)
}

/// Case-insensitive host identity match against `Game.host`.
/// The host may be "Rutex#2561" or just "Rutex" (no id), so we accept either:
/// - full match: "Rutex#2561" == "Rutex#2561"
/// - name-only match: "Rutex" matches "Rutex#2561" (when no id was given вЂ” but
///   the user always gives both, so we use full match here).
fn host_matches(name: &str, id: u32, host: &str) -> bool {
    let host_lc = host.to_ascii_lowercase();
    let name_lc = name.to_ascii_lowercase();
    let target = format!("{}#{}", name_lc, id);
    host_lc == target
}

/// Compute watch events for a single tick. `current_games` is the freshly
/// fetched gamelist. `settings` is the loaded `user_settings` map.
/// `state` is mutated in-place: last_taken/last_total/explicit_games are
/// updated so the next tick can compute deltas.
pub fn tick(
    state: &mut WatchState,
    current_games: &[Game],
    settings: &HashMap<i64, UserSettings>,
) -> Vec<(i64, WatchEvent)> {
    let mut out: Vec<(i64, WatchEvent)> = Vec::new();
    let mut seen_ids: HashSet<i64> = HashSet::new();

    for game in current_games {
        seen_ids.insert(game.id);

        for (user_id, user_settings) in settings {
            // Determine why this user is interested in this game.
            let is_nickname = user_settings.monitoring_enabled
                && user_settings
                    .watched_identity
                    .as_deref()
                    .and_then(|id| parse_identity(id).ok())
                    .map(|(name, num)| host_matches(&name, num, &game.host))
                    .unwrap_or(false);

            let user = state.user_mut(*user_id);
            let is_explicit = user.explicit_games.contains_key(&game.id);

            if !is_nickname && !is_explicit {
                // ensure we don't carry stale state for this user/game
                user.last_taken.remove(&game.id);
                user.last_total.remove(&game.id);
                continue;
            }

            // Explicit takes priority over nickname (avoids double-send).
            let via_nickname = is_nickname && !is_explicit;
            let taken = game.slots_taken;
            let total = game.slots_total;

            // Always refresh the explicit_games snapshot to the current state
            // (so the monitor screen shows up-to-date taken/total).
            if is_explicit {
                user.explicit_games.insert(
                    game.id,
                    WatchedGame {
                        id: game.id,
                        map: game.map.clone(),
                        host: game.host.clone(),
                        taken,
                        total,
                    },
                );
            }

            match (user.last_taken.get(&game.id), user.last_total.get(&game.id)) {
                (None, _) => {
                    // First time we see this (user, game).
                    out.push((
                        *user_id,
                        WatchEvent::Started {
                            game: game.clone(),
                            nickname: via_nickname,
                        },
                    ));
                    user.last_taken.insert(game.id, taken);
                    user.last_total.insert(game.id, total);

                    // If the game happens to be full at first sight, also
                    // emit Filled. (Unlikely but possible.)
                    if taken > 0 && taken == total {
                        out.push((*user_id, WatchEvent::Filled { game: game.clone() }));
                    }
                }
                (Some(&prev_taken), Some(&prev_total)) => {
                    let delta = taken - prev_taken;
                    if delta != 0 {
                        out.push((
                            *user_id,
                            WatchEvent::SlotsDelta {
                                game: game.clone(),
                                delta,
                            },
                        ));
                    }
                    // Filled transition: was below, now at total.
                    if prev_taken < prev_total && taken == total && total > 0 {
                        out.push((*user_id, WatchEvent::Filled { game: game.clone() }));
                    }
                    user.last_taken.insert(game.id, taken);
                    user.last_total.insert(game.id, total);
                }
                (Some(_), None) => {
                    // Shouldn't happen in practice (we always insert both),
                    // but handle defensively: treat as first sight.
                    user.last_taken.insert(game.id, taken);
                    user.last_total.insert(game.id, total);
                }
            }
        }
    }

    // Cleanup: any game the user is no longer interested in OR that is no
    // longer in the gamelist should have its state cleared.
    let mut to_remove: Vec<(i64, i64)> = Vec::new();
    for (user_id, user_state) in state.users.iter_mut() {
        for (&gid, _) in user_state.last_taken.iter() {
            if !seen_ids.contains(&gid) {
                to_remove.push((*user_id, gid));
            }
        }
        // Drop explicit_games entries that no longer exist in the gamelist.
        user_state.explicit_games.retain(|gid, _| seen_ids.contains(gid));
    }
    for (uid, gid) in to_remove {
        if let Some(u) = state.users.get_mut(&uid) {
            u.last_taken.remove(&gid);
            u.last_total.remove(&gid);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: i64, host: &str, taken: i32, total: i32) -> Game {
        Game {
            id,
            name: "Some game".to_string(),
            host: host.to_string(),
            map: "TestMap.w3x".to_string(),
            server: "eu".to_string(),
            slots_taken: taken,
            slots_total: total,
            created: 0,
        }
    }

    fn settings_with(user_id: i64, identity: Option<&str>, enabled: bool) -> UserSettings {
        UserSettings {
            user_id,
            watched_identity: identity.map(String::from),
            monitoring_enabled: enabled,
        }
    }

    // --- parse_identity ---

    #[test]
    fn parse_identity_valid() {
        let (n, i) = parse_identity("Rutex#2561").unwrap();
        assert_eq!(n, "Rutex");
        assert_eq!(i, 2561);
    }

    #[test]
    fn parse_identity_with_underscored_name() {
        let (n, i) = parse_identity("Hell_Wolf#31976").unwrap();
        assert_eq!(n, "Hell_Wolf");
        assert_eq!(i, 31976);
    }

    #[test]
    fn parse_identity_rejects_no_hash() {
        assert!(parse_identity("Rutex2561").is_err());
    }

    #[test]
    fn parse_identity_rejects_empty_name() {
        assert!(parse_identity("#12345").is_err());
    }

    #[test]
    fn parse_identity_rejects_empty_id() {
        assert!(parse_identity("Rutex#").is_err());
    }

    #[test]
    fn parse_identity_rejects_non_digit() {
        assert!(parse_identity("Rutex#abcd").is_err());
        assert!(parse_identity("Rutex#1234a").is_err());
    }

    #[test]
    fn parse_identity_rejects_zero_id() {
        assert!(parse_identity("Rutex#0").is_err());
    }

    #[test]
    fn parse_identity_rejects_too_long_id() {
        assert!(parse_identity("Rutex#12345678901").is_err()); // 11 С†РёС„СЂ
    }

    #[test]
    fn parse_identity_rejects_double_hash() {
        // РїРµСЂРІС‹Р№ split_once РґР°СЃС‚ ("X", "Y#1"), name СЃРѕРґРµСЂР¶РёС‚ # С‚РѕР»СЊРєРѕ РµСЃР»Рё
        // РІРЅСѓС‚СЂРё name РµСЃС‚СЊ #, РЅРѕ split_once Р±РµСЂС‘С‚ РїРµСЂРІРѕРµ #. РџРѕСЌС‚РѕРјСѓ
        // РїСЂРѕРІРµСЂСЏРµРј С‡С‚Рѕ РїРѕСЃР»Рµ РїРµСЂРІРѕРіРѕ split id РјРѕР¶РµС‚ СЃРѕРґРµСЂР¶Р°С‚СЊ # вЂ” СЌС‚Рѕ
        // РїСЂРѕР№РґС‘С‚, РЅРѕ РјС‹ С…РѕС‚РёРј СЃС‚СЂРѕРіРёР№ С„РѕСЂРјР°С‚ "СЂРѕРІРЅРѕ РѕРґРёРЅ #".
        // РЎРµР№С‡Р°СЃ parse_identity РїСЂРѕРїСѓСЃС‚РёС‚ "X#Y#1" (id="Y#1" вЂ” РЅРµ С†РёС„СЂС‹ в†’ fail)
        assert!(parse_identity("X#Y#1").is_err());
    }

    // --- should_show_watch_button ---

    #[test]
    fn watch_button_shown_when_no_identity() {
        let s = settings_with(1, None, false);
        let g = game(1, "Rutex#2561", 2, 10);
        assert!(should_show_watch_button(&s, &g));
    }

    #[test]
    fn watch_button_hidden_when_nickname_matches() {
        let s = settings_with(1, Some("Rutex#2561"), true);
        let g = game(1, "Rutex#2561", 2, 10);
        assert!(!should_show_watch_button(&s, &g));
    }

    #[test]
    fn watch_button_shown_when_nickname_disabled() {
        // identity set, РЅРѕ РјРѕРЅРёС‚РѕСЂРёРЅРі РІС‹РєР»СЋС‡РµРЅ в†’ РїРѕРєР°Р·С‹РІР°РµРј, РёРЅР°С‡Рµ РЅРµС‚
        // СЃРїРѕСЃРѕР±Р° РЅР°С‡Р°С‚СЊ РјРѕРЅРёС‚РѕСЂРёС‚СЊ СЃРІРѕСЋ РёРіСЂСѓ
        let s = settings_with(1, Some("Rutex#2561"), false);
        let g = game(1, "Rutex#2561", 2, 10);
        assert!(should_show_watch_button(&s, &g));
    }

    #[test]
    fn watch_button_shown_when_host_does_not_match() {
        let s = settings_with(1, Some("Rutex#2561"), true);
        let g = game(1, "Other#2561", 2, 10);
        assert!(should_show_watch_button(&s, &g));
    }

    #[test]
    fn watch_button_match_is_case_insensitive() {
        let s = settings_with(1, Some("Rutex#2561"), true);
        let g = game(1, "rutex#2561", 2, 10);
        assert!(!should_show_watch_button(&s, &g));
    }

    // --- tick: first sight ---

    #[test]
    fn tick_first_sight_explicit_emits_started() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games = vec![game(100, "Host#1", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let events = tick(&mut state, &games, &settings);
        assert_eq!(events.len(), 1);
        match &events[0].1 {
            WatchEvent::Started { nickname, .. } => assert!(!*nickname),
            _ => panic!("expected Started"),
        }
    }

    #[test]
    fn tick_first_sight_nickname_emits_started() {
        let mut state = WatchState::default();
        let games = vec![game(100, "Rutex#2561", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, Some("Rutex#2561"), true));
        let events = tick(&mut state, &games, &settings);
        assert_eq!(events.len(), 1);
        match &events[0].1 {
            WatchEvent::Started { nickname, .. } => assert!(*nickname),
            _ => panic!("expected Started"),
        }
    }

    #[test]
    fn tick_no_interest_no_events() {
        let mut state = WatchState::default();
        let games = vec![game(100, "Rutex#2561", 2, 10)];
        let mut settings = HashMap::new();
        // identity Р·Р°РґР°РЅ, РЅРѕ РјРѕРЅРёС‚РѕСЂРёРЅРі РІС‹РєР»СЋС‡РµРЅ
        settings.insert(1, settings_with(1, Some("Rutex#2561"), false));
        let events = tick(&mut state, &games, &settings);
        assert!(events.is_empty());
    }

    // --- tick: deltas ---

    #[test]
    fn tick_aggregates_positive_delta() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let _ = tick(&mut state, &games1, &settings); // РїРµСЂРІС‹Р№ С‚РёРє

        // 2 в†’ 5: delta=+3
        let games2 = vec![game(100, "Host#1", 5, 10)];
        let events = tick(&mut state, &games2, &settings);
        assert_eq!(events.len(), 1);
        match &events[0].1 {
            WatchEvent::SlotsDelta { delta, .. } => assert_eq!(*delta, 3),
            _ => panic!("expected SlotsDelta"),
        }
    }

    #[test]
    fn tick_aggregates_negative_delta() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 5, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let _ = tick(&mut state, &games1, &settings);

        // 5 в†’ 3: delta=-2
        let games2 = vec![game(100, "Host#1", 3, 10)];
        let events = tick(&mut state, &games2, &settings);
        assert_eq!(events.len(), 1);
        match &events[0].1 {
            WatchEvent::SlotsDelta { delta, .. } => assert_eq!(*delta, -2),
            _ => panic!("expected SlotsDelta"),
        }
    }

    #[test]
    fn tick_no_event_when_zero_delta() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 5, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let _ = tick(&mut state, &games1, &settings);

        // Р±РµР· РёР·РјРµРЅРµРЅРёР№
        let games2 = vec![game(100, "Host#1", 5, 10)];
        let events = tick(&mut state, &games2, &settings);
        assert!(events.is_empty());
    }

    // --- tick: filled ---

    #[test]
    fn tick_filled_sends_once() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 9, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let e1 = tick(&mut state, &games1, &settings);
        // 9/10 в†’ С‚РѕР»СЊРєРѕ Started
        assert_eq!(e1.len(), 1);
        assert!(matches!(e1[0].1, WatchEvent::Started { .. }));

        // 9 в†’ 10: Filled
        let games2 = vec![game(100, "Host#1", 10, 10)];
        let e2 = tick(&mut state, &games2, &settings);
        // 2 СЃРѕР±С‹С‚РёСЏ: SlotsDelta(+1) Рё Filled
        assert_eq!(e2.len(), 2);
        assert!(matches!(e2[0].1, WatchEvent::SlotsDelta { delta: 1, .. }));
        assert!(matches!(e2[1].1, WatchEvent::Filled { .. }));

        // 10 в†’ 10: РЅРёС‡РµРіРѕ
        let games3 = vec![game(100, "Host#1", 10, 10)];
        let e3 = tick(&mut state, &games3, &settings);
        assert!(e3.is_empty());
    }

    #[test]
    fn tick_filled_emits_again_after_dip() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));

        let _ = tick(&mut state, &[game(100, "Host#1", 9, 10)], &settings);
        // 9 в†’ 10: Filled
        let _ = tick(&mut state, &[game(100, "Host#1", 10, 10)], &settings);
        // 10 в†’ 8: СѓС€Р»Рё РёРіСЂРѕРєРё
        let _ = tick(&mut state, &[game(100, "Host#1", 8, 10)], &settings);
        // 8 в†’ 10: РѕРїСЏС‚СЊ Filled
        let e = tick(&mut state, &[game(100, "Host#1", 10, 10)], &settings);
        // SlotsDelta(+2) + Filled
        assert_eq!(e.len(), 2);
        assert!(matches!(e[1].1, WatchEvent::Filled { .. }));
    }

    #[test]
    fn tick_first_sight_full_emits_filled() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games = vec![game(100, "Host#1", 10, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let e = tick(&mut state, &games, &settings);
        // Started + Filled
        assert_eq!(e.len(), 2);
        assert!(matches!(e[0].1, WatchEvent::Started { .. }));
        assert!(matches!(e[1].1, WatchEvent::Filled { .. }));
    }

    // --- tick: cleanup ---

    #[test]
    fn tick_clears_state_when_game_removed() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let _ = tick(&mut state, &games1, &settings);
        assert!(state.users[&1].last_taken.contains_key(&100));

        // РРіСЂР° СѓС€Р»Р°
        let games2: Vec<Game> = vec![];
        let _ = tick(&mut state, &games2, &settings);
        // explicit_games С‚РѕР¶Рµ С‡РёСЃС‚РёС‚СЃСЏ
        assert!(!state.users[&1].explicit_games.contains_key(&100));
        assert!(!state.users[&1].last_taken.contains_key(&100));
    }

    // --- tick: explicit + nickname ---

    #[test]
    fn tick_explicit_takes_priority_over_nickname() {
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        // Game.hosted by Rutex, user has both explicit AND nickname match
        let games = vec![game(100, "Rutex#2561", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, Some("Rutex#2561"), true));
        let e = tick(&mut state, &games, &settings);
        // С‚РѕР»СЊРєРѕ РѕРґРёРЅ Started (explicit), nickname РЅРµ РґСѓР±Р»РёСЂСѓРµС‚
        assert_eq!(e.len(), 1);
        match &e[0].1 {
            WatchEvent::Started { nickname, .. } => assert!(!*nickname),
            _ => panic!("expected Started"),
        }
    }

    #[test]
    fn tick_nickname_does_not_match_case_sensitive() {
        // host СЃРѕРґРµСЂР¶РёС‚ РёРјСЏ РІ РґСЂСѓРіРѕРј СЂРµРіРёСЃС‚СЂРµ
        let mut state = WatchState::default();
        let games = vec![game(100, "RUTEX#2561", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, Some("Rutex#2561"), true));
        let e = tick(&mut state, &games, &settings);
        // case-insensitive в†’ РјР°С‚С‡
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn tick_user_uninterested_clears_stale_state() {
        // user Р±С‹Р» РІ explicit, РїРѕС‚РѕРј СѓР±СЂР°Р» РёР· explicit в†’ СЃРѕСЃС‚РѕСЏРЅРёРµ С‡РёСЃС‚РёС‚СЃСЏ
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games1 = vec![game(100, "Host#1", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let _ = tick(&mut state, &games1, &settings);
        assert!(state.users[&1].last_taken.contains_key(&100));

        // user РѕС‚РїРёСЃР°Р»СЃСЏ
        state.users.get_mut(&1).unwrap().explicit_games.remove(&100);
        let games2 = vec![game(100, "Host#1", 3, 10)];
        let e = tick(&mut state, &games2, &settings);
        assert!(e.is_empty());
        assert!(!state.users[&1].last_taken.contains_key(&100));
    }

    #[test]
    fn tick_multiple_users_independent() {
        let mut state = WatchState::default();
        state.user_mut(1).explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games = vec![game(100, "Host#1", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        // user 2 вЂ” Р±РµР· РЅР°СЃС‚СЂРѕРµРє
        settings.insert(2, settings_with(2, None, false));
        let e = tick(&mut state, &games, &settings);
        // С‚РѕР»СЊРєРѕ user 1
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, 1);
    }

    #[test]
    fn tick_nickname_user_discovers_game() {
        // Game.host == user.watched_identity (РЅРёР¶РЅРёР№ СЂРµРіРёСЃС‚СЂ) вЂ” РґРѕР»Р¶РЅРѕ РЅР°Р№С‚РёСЃСЊ
        let mut state = WatchState::default();
        let games = vec![game(100, "rutex#2561", 2, 10)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, Some("Rutex#2561"), true));
        let e = tick(&mut state, &games, &settings);
        assert_eq!(e.len(), 1);
    }

    #[test]
    fn tick_first_sight_does_not_emit_filled_when_zero_taken() {
        // taken=0 РЅРµ СЃС‡РёС‚Р°РµС‚СЃСЏ "filled" (total > 0)
        let mut state = WatchState::default();
        let user = state.user_mut(1);
        user.explicit_games.insert(100, WatchedGame { id: 100, map: "TestMap.w3x".into(), host: "Host#1".into(), taken: 0, total: 0 });
        let games = vec![game(100, "Host#1", 0, 0)];
        let mut settings = HashMap::new();
        settings.insert(1, settings_with(1, None, false));
        let e = tick(&mut state, &games, &settings);
        // С‚РѕР»СЊРєРѕ Started, РЅРµ Filled (total=0)
        assert_eq!(e.len(), 1);
        assert!(matches!(e[0].1, WatchEvent::Started { .. }));
    }
}

use std::time::Duration;

use crate::api::Game;
use crate::db::UserSettings;
use crate::loc::{tr, Lang};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Keyboard under a notification message — 2×2 grid:
/// `[12h] [Off]` / `[👁 Watch | 👁 Unwatch] [✅]`
///
/// Watch-button logic:
/// - hidden when the user already monitors this lobby via nickname mode
///   (see `crate::watcher::should_show_watch_button`)
/// - if `already_watching` is true, renders as "Unwatch" (with `unwatch:{game_id}`)
/// - otherwise renders as "Watch" (with `watch:{game_id}`)
pub fn notification_kb(
    lang: Lang,
    kind: &str,
    pattern: &str,
    game: &Game,
    user_settings: &UserSettings,
    already_watching: bool,
) -> InlineKeyboardMarkup {
    let t = tr(lang);
    let mut rows: Vec<Vec<InlineKeyboardButton>> = vec![
        vec![
            InlineKeyboardButton::callback(
                t.btn_snooze.to_string(),
                format!("snooze:{kind}:{pattern}"),
            ),
            InlineKeyboardButton::callback(
                t.btn_mute.to_string(),
                format!("mute:{kind}:{pattern}"),
            ),
        ],
    ];
    let mut second_row = Vec::new();
    if crate::watcher::should_show_watch_button(user_settings, game) {
        if already_watching {
            second_row.push(InlineKeyboardButton::callback(
                t.btn_monitor_unwatch.to_string(),
                format!("unwatch:{}", game.id),
            ));
        } else {
            second_row.push(InlineKeyboardButton::callback(
                t.btn_monitor_watch.to_string(),
                format!("watch:{}", game.id),
            ));
        }
    }
    second_row.push(InlineKeyboardButton::callback(
        t.btn_check.to_string(),
        "check".to_string(),
    ));
    rows.push(second_row);
    InlineKeyboardMarkup::new(rows)
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn game_age(created: i64) -> Duration {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.saturating_sub(Duration::from_secs(created as u64))
}

pub fn pinger_msg(game: &Game, lang: Lang) -> String {
    let t = tr(lang);
    let age = format_duration(game_age(game.created));
    let base = game.notification_text(lang);
    format!("{base}\n\n{}", t.ping_created.replace("{time}", &age))
}

pub fn pinger_final_msg(game: &Game, wait: Duration, lang: Lang) -> String {
    let t = tr(lang);
    let base = game.notification_text(lang);
    format!("{base}\n\n{}", t.ping_started.replace("{time}", &format_duration(wait)))
}

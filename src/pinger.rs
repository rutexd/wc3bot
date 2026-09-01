use std::time::Duration;

use crate::api::Game;
use crate::db::UserSettings;
use crate::loc::{tr, Lang};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Keyboard under a notification message.
/// 4-я кнопка "👁 Watch" появляется только если:
/// - у пользователя ещё НЕТ identity, совпадающей с `game.host` при включённом мониторинге
/// (см. `crate::watcher::should_show_watch_button`).
pub fn notification_kb(
    lang: Lang,
    kind: &str,
    pattern: &str,
    game: &Game,
    user_settings: &UserSettings,
) -> InlineKeyboardMarkup {
    let t = tr(lang);
    let mut row = vec![
        InlineKeyboardButton::callback(
            t.btn_snooze.to_string(),
            format!("snooze:{kind}:{pattern}"),
        ),
        InlineKeyboardButton::callback(
            t.btn_mute.to_string(),
            format!("mute:{kind}:{pattern}"),
        ),
        InlineKeyboardButton::callback(t.btn_check.to_string(), "check".to_string()),
    ];
    if crate::watcher::should_show_watch_button(user_settings, game) {
        row.push(InlineKeyboardButton::callback(
            t.btn_monitor_watch.to_string(),
            format!("watch:{}", game.id),
        ));
    }
    InlineKeyboardMarkup::new(vec![row])
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

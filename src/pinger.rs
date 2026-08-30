use std::time::Duration;

use crate::api::Game;
use crate::loc::{tr, Lang};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

pub fn notification_kb(lang: Lang, map: &str) -> InlineKeyboardMarkup {
    let t = tr(lang);
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            t.btn_snooze.to_string(),
            format!("snooze:{map}"),
        ),
        InlineKeyboardButton::callback(t.btn_mute.to_string(), format!("mute:{map}")),
        InlineKeyboardButton::callback(t.btn_check.to_string(), "check".to_string()),
    ]])
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

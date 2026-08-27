use std::time::Duration;

use crate::api::Game;
use crate::loc::{tr, Lang};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

pub fn notification_kb(lang: Lang) -> InlineKeyboardMarkup {
    let t = tr(lang);
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(t.btn_snooze.to_string(), "snooze".to_string()),
        InlineKeyboardButton::callback(t.btn_mute.to_string(), "mute".to_string()),
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
    let name = if game.name.trim().is_empty() { t.dash } else { &game.name };
    format!(
        "{map}\n{name}\n{host}\n{slots}\n\n{created}",
        map = t.ping_map.replace("{map}", &game.map),
        name = t.ping_name.replace("{name}", name).replace("{server}", &game.server.to_uppercase()),
        host = t.ping_host.replace("{host}", &game.host),
        slots = t.ping_slots.replace("{taken}", &game.slots_taken.to_string()).replace("{total}", &game.slots_total.to_string()),
        created = t.ping_created.replace("{time}", &age),
    )
}

pub fn pinger_final_msg(game: &Game, wait: Duration, lang: Lang) -> String {
    let t = tr(lang);
    let name = if game.name.trim().is_empty() { t.dash } else { &game.name };
    format!(
        "{map}\n{name}\n{host}\n{slots}\n\n{started}",
        map = t.ping_map.replace("{map}", &game.map),
        name = t.ping_name.replace("{name}", name).replace("{server}", &game.server.to_uppercase()),
        host = t.ping_host.replace("{host}", &game.host),
        slots = t.ping_slots.replace("{taken}", &game.slots_taken.to_string()).replace("{total}", &game.slots_total.to_string()),
        started = t.ping_started.replace("{time}", &format_duration(wait)),
    )
}

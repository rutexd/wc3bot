use std::time::Duration;

use crate::api::Game;

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

pub fn pinger_msg(game: &Game) -> String {
    let age = format_duration(game_age(game.created));
    format!(
        "🗺️ Карта: {}\n\
         🏠 Хост: {}\n\
         📛 Название: {}\n\
         👥 Игроки: {}/{}\n\
         🌍 Сервер: {}\n\
         🆔 ID: {}\n\
         ⏱ Создано: {} назад",
        game.map,
        game.host,
        if game.name.trim().is_empty() { "—" } else { &game.name },
        game.slots_taken,
        game.slots_total,
        game.server.to_uppercase(),
        game.id,
        age,
    )
}

pub fn pinger_final_msg(game: &Game) -> String {
    let age = format_duration(game_age(game.created));
    format!(
        "🗺️ Карта: {}\n\
         🏠 Хост: {}\n\
         📛 Название: {}\n\
         🌍 Сервер: {}\n\
         🆔 ID: {}\n\
         \n\
         🕐 Началось: {} назад\n\
         ✅ Игра началась",
        game.map,
        game.host,
        if game.name.trim().is_empty() { "—" } else { &game.name },
        game.server.to_uppercase(),
        game.id,
        age,
    )
}

use std::time::Duration;

use crate::api::Game;
use crate::loc::{tr, Lang};

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
    let name = if game.name.trim().is_empty() { "—" } else { &game.name };
    format!(
        "{map}\n{host}\n{name}\n{slots}\n{server}\n{id}\n{created}",
        map = t.ping_map.replace("{map}", &game.map),
        host = t.ping_host.replace("{host}", &game.host),
        name = t.ping_name.replace("{name}", name),
        slots = t.ping_slots.replace("{taken}", &game.slots_taken.to_string()).replace("{total}", &game.slots_total.to_string()),
        server = t.ping_server.replace("{server}", &game.server.to_uppercase()),
        id = t.ping_id.replace("{id}", &game.id.to_string()),
        created = t.ping_created.replace("{time}", &age),
    )
}

pub fn pinger_final_msg(game: &Game, wait: Duration, lang: Lang) -> String {
    let t = tr(lang);
    let name = if game.name.trim().is_empty() { "—" } else { &game.name };
    format!(
        "{map}\n{host}\n{name}\n{slots}\n{server}\n{id}\n\n{started}",
        map = t.ping_map.replace("{map}", &game.map),
        host = t.ping_host.replace("{host}", &game.host),
        name = t.ping_name.replace("{name}", name),
        slots = t.ping_slots.replace("{taken}", &game.slots_taken.to_string()).replace("{total}", &game.slots_total.to_string()),
        server = t.ping_server.replace("{server}", &game.server.to_uppercase()),
        id = t.ping_id.replace("{id}", &game.id.to_string()),
        started = t.ping_started.replace("{time}", &format_duration(wait)),
    )
}

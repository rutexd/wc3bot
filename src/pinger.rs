use std::time::{Duration, Instant};
use teloxide::prelude::*;
use teloxide::types::{ChatId, MessageId};

use crate::api::Game;

#[derive(Clone)]
pub struct PingerMsg {
    pub chat_id: ChatId,
    pub message_id: MessageId,
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

pub fn pinger_msg(game: &Game, elapsed: Duration) -> String {
    format!(
        "🗺️ Карта: {}\n\
         🏠 Хост: {}\n\
         📛 Название: {}\n\
         👥 Игроки: {}/{}\n\
         🌍 Сервер: {}\n\
         🆔 ID: {}\n\
         ⏱ Пинг: {} назад",
        game.map,
        game.host,
        if game.name.trim().is_empty() { "—" } else { &game.name },
        game.slots_taken,
        game.slots_total,
        game.server.to_uppercase(),
        game.id,
        format_duration(elapsed),
    )
}

pub fn pinger_final_msg(game: &Game, total: Duration) -> String {
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
        format_duration(total),
    )
}

pub async fn run_pinger(
    mut game: Game,
    messages: Vec<PingerMsg>,
    client: reqwest::Client,
    started: Instant,
    bot: teloxide::Bot,
) {
    let interval = Duration::from_secs(10);

    loop {
        tokio::time::sleep(interval).await;

        let current = match fetch_game(&client, game.id).await {
            Some(g) => g,
            None => {
                let total = started.elapsed();
                let text = pinger_final_msg(&game, total);
                for m in &messages {
                    let _ = bot
                        .edit_message_text(m.chat_id, m.message_id, text.clone())
                        .await;
                }
                return;
            }
        };

        let elapsed = started.elapsed();
        let text = pinger_msg(&current, elapsed);
        game = current;

        for m in &messages {
            let _ = bot
                .edit_message_text(m.chat_id, m.message_id, text.clone())
                .await;
        }
    }
}

async fn fetch_game(client: &reqwest::Client, id: i64) -> Option<Game> {
    let games = crate::api::fetch_gamelist(client).await.ok()?;
    games.into_iter().find(|g| g.id == id)
}

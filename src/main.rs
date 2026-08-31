mod api;
mod db;
mod handlers;
mod loc;
mod migrations;
mod norm;
mod pinger;
mod quiet;

use anyhow::{bail, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    env,
    time::Duration,
};
use teloxide::{prelude::*, types::ChatId};

fn sanitize_name(name: &str) -> String {
    name.trim_start_matches('@')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_uppercase()
}

/// Token lookup order:
///   1. --token / -t CLI arg
///   2. <BOTNAME>_TOKEN from environment or dotenv files (.env, .<botname>, .env.<botname>)
///   3. TELEGRAM_BOT_TOKEN
///   4. WC3BOT_TOKEN
///   5. BOT_TOKEN
///
/// Bot name: --name/-n arg, or first positional that is not a token.
fn resolve_token() -> Result<(String, String)> {
    let mut args = env::args().skip(1);
    let mut name: Option<String> = None;
    let mut token: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--token" | "-t" => token = args.next(),
            "--name" | "-n" => name = args.next(),
            _ => {
                if arg.contains(':') || arg.chars().all(|c| c.is_ascii_digit()) {
                    token = Some(arg);
                } else if name.is_none() {
                    name = Some(arg);
                }
            }
        }
    }

    // dotenv files
    let _ = dotenvy::dotenv();
    if let Some(n) = &name {
        let base = n.trim_start_matches('@');
        for f in [format!(".{base}"), format!(".env.{base}")] {
            if std::path::Path::new(&f).is_file() {
                let _ = dotenvy::from_filename(&f);
            }
        }
    }

    if let Some(t) = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        return Ok((t, "аргумент командной строки".into()));
    }

    let mut tried: Vec<String> = Vec::new();
    if let Some(n) = &name {
        let key = format!("{}_TOKEN", sanitize_name(n));
        tried.push(key.clone());
        if let Ok(v) = env::var(&key) {
            if !v.trim().is_empty() {
                return Ok((v.trim().to_string(), format!("env {key}")));
            }
        }
    }
    for key in ["TELEGRAM_BOT_TOKEN", "WC3BOT_TOKEN", "BOT_TOKEN"] {
        tried.push(key.to_string());
        if let Ok(v) = env::var(key) {
            if !v.trim().is_empty() {
                return Ok((v.trim().to_string(), format!("env {key}")));
            }
        }
    }
    bail!("токен не найден. Задай его через --token, или в переменной {} (можно в файле .env)", tried.join(" / "))
}

fn main() {
    let child = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .expect("не удалось создать поток");
    match child.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log::error!("{e:#}");
            std::process::exit(1);
        }
        Err(_) => std::process::exit(1),
    }
}

fn run() -> Result<()> {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(16 * 1024 * 1024)
        .build()
        .context("не удалось создать tokio runtime")?;
    rt.block_on(run_async())
}

async fn run_async() -> Result<()> {
    let (token, source) = resolve_token()?;
    log::info!("токен загружен из: {source}");

    let bot = teloxide::Bot::new(token);
    let me = bot
        .get_me()
        .await
        .context("не удалось подключиться к Telegram (проверь токен)")?;
    log::info!("бот @{} запущен", me.username());

    let database = std::sync::Arc::new(db::Db::open("wc3bot.db")?);
    let deleted: std::sync::Arc<std::sync::Mutex<HashSet<(i64, i32)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(HashSet::new()));

    {
        let bot = bot.clone();
        let database = database.clone();
        let deleted = deleted.clone();
        tokio::spawn(async move {
            if let Err(e) = poller(bot, database, deleted).await {
                log::error!("poller остановлен: {e:#}");
            }
        });
    }

    let state = handlers::AppState::new(database, me.id, deleted);
    let handler = teloxide::dptree::entry()
        .branch(Update::filter_message().endpoint(handlers::handle_message))
        .branch(Update::filter_callback_query().endpoint(handlers::handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(teloxide::dptree::deps![state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

struct TrackedGame {
    chat_id: ChatId,
    message_id: teloxide::types::MessageId,
    game: api::Game,
    lang: loc::Lang,
    sub_kind: String,
    sub_pattern: String,
}

async fn poller(
    bot: teloxide::Bot,
    database: std::sync::Arc<db::Db>,
    deleted: std::sync::Arc<std::sync::Mutex<HashSet<(i64, i32)>>>,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("wc3bot/0.1")
        .build()?;

    let interval = Duration::from_secs(
        env::var("POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10),
    );

    // Seed seen ids without notifying on startup.
    let mut seen: HashSet<i64> = api::fetch_gamelist(&client)
        .await
        .context("первичный опрос gamelist не удался")?
        .into_iter()
        .map(|g| g.id)
        .collect();
    log::info!("poller: инициализировано {} известных игр", seen.len());

    // game_id -> all tracked messages for that game
    let mut tracked: HashMap<i64, Vec<TrackedGame>> = HashMap::new();

    loop {
        tokio::time::sleep(interval).await;
        database.release_expired_map_snoozes();
        let games = match api::fetch_gamelist(&client).await {
            Ok(g) => g,
            Err(e) => {
                log::warn!("poller: ошибка запроса: {e:#}");
                continue;
            }
        };

        // --- detect new games → notify & start tracking ---
        for game in &games {
            if !seen.insert(game.id) {
                continue;
            }

            let active = database.all_active_subs();
            let mut quiet_cache: HashMap<i64, bool> = HashMap::new();
            for active in &active {
                if !quiet_cache.contains_key(&active.chat_id) {
                    quiet_cache.insert(active.chat_id, database.is_in_quiet_hours(active.chat_id));
                }
            }

            for active in active {
                if *quiet_cache.get(&active.chat_id).unwrap_or(&false) {
                    continue;
                }
                if database.should_notify(&active.sub, &game.map, &game.host, &game.name) {
                    let lang = database.lang(active.chat_id);
                    match bot
                        .send_message(ChatId(active.chat_id), game.notification_text(lang))
                        .reply_markup(pinger::notification_kb(
                            lang,
                            &active.sub.kind,
                            &active.sub.pattern,
                        ))
                        .await
                    {
                        Ok(m) => {
                            tracked.entry(game.id).or_default().push(TrackedGame {
                                chat_id: ChatId(active.chat_id),
                                message_id: m.id,
                                game: game.clone(),
                                lang,
                                sub_kind: active.sub.kind.clone(),
                                sub_pattern: active.sub.pattern.clone(),
                            });
                        }
                        Err(e) => {
                            log::warn!(
                                "poller: не удалось отправить уведомление в {}: {e}",
                                active.chat_id
                            );
                        }
                    }
                }
            }
        }

        // --- drop messages the user checked/deleted ---
        {
            let deleted = deleted.lock().unwrap();
            let mut empty: Vec<i64> = Vec::new();
            for (gid, entries) in tracked.iter_mut() {
                entries.retain(|e| !deleted.contains(&(e.chat_id.0, e.message_id.0)));
                if entries.is_empty() {
                    empty.push(*gid);
                }
            }
            for gid in empty {
                tracked.remove(&gid);
            }
        }

        // --- update tracked games still alive, finalize gone ones ---
        let mut to_remove: Vec<i64> = Vec::new();
        for (&game_id, entries) in &mut tracked {
            if let Some(current) = games.iter().find(|g| g.id == game_id) {
                // game still alive → update message
                for entry in entries.iter_mut() {
                    let text = pinger::pinger_msg(current, entry.lang);
if let Err(e) = bot
                    .edit_message_text(entry.chat_id, entry.message_id, text.clone())
                    .reply_markup(pinger::notification_kb(
                        entry.lang,
                        &entry.sub_kind,
                        &entry.sub_pattern,
                    ))
                    .await
                {
                    log::warn!("poller: failed to edit msg in {}: {e}", entry.chat_id);
                }
                    entry.game = current.clone();
                }
            } else {
                // game gone → final message
                let wait = pinger::game_age(entries[0].game.created);
                log::info!("poller: game {} gone, sending final message", game_id);
                for entry in entries.iter() {
                    let text = pinger::pinger_final_msg(&entry.game, wait, entry.lang);
let _ = bot
                    .edit_message_text(entry.chat_id, entry.message_id, text.clone())
                    .reply_markup(pinger::notification_kb(
                        entry.lang,
                        &entry.sub_kind,
                        &entry.sub_pattern,
                    ))
                    .await;
                }
                to_remove.push(game_id);
            }
        }
        for id in to_remove {
            tracked.remove(&id);
        }

        // Keep the set bounded.
        if seen.len() > 100_000 {
            seen.clear();
            if let Ok(games) = api::fetch_gamelist(&client).await {
                seen.extend(games.into_iter().map(|g| g.id));
            }
        }
    }
}

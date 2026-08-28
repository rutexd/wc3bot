use crate::db::{self, Db};
use crate::loc::{tr, T};
use futures::future::BoxFuture;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, UserId},
};

const MAX_PATTERN_LEN: usize = 64;

#[derive(Debug, Clone)]
pub enum Pending {
    AddMap,
    AddHost,
    AddName,
    AddAll,
    Rename(i64),
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Db>,
    pub pending: Arc<Mutex<HashMap<i64, Pending>>>,
    pub bot_id: UserId,
    pub deleted: Arc<Mutex<std::collections::HashSet<(i64, i32)>>>,
}

impl AppState {
    pub fn new(
        db: Arc<Db>,
        bot_id: UserId,
        deleted: Arc<Mutex<std::collections::HashSet<(i64, i32)>>>,
    ) -> Self {
        Self {
            db,
            pending: Arc::new(Mutex::new(HashMap::new())),
            bot_id,
            deleted,
        }
    }

    pub fn take_pending(&self, chat_id: i64) -> Option<Pending> {
        self.pending.lock().unwrap().remove(&chat_id)
    }

    pub fn set_pending(&self, chat_id: i64, p: Pending) {
        self.pending.lock().unwrap().insert(chat_id, p);
    }

    pub fn clear_pending(&self, chat_id: i64) {
        self.pending.lock().unwrap().remove(&chat_id);
    }
}

fn btn(text: &str, data: &str) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(text.to_string(), data.to_string())
}

fn main_menu_kb(state: &AppState, uid: i64, t: &'static T) -> InlineKeyboardMarkup {
    let notif_label = if state.db.notifications_enabled(uid) {
        t.btn_notif_on
    } else {
        t.btn_notif_off
    };
    InlineKeyboardMarkup::new(vec![
        vec![btn(t.btn_maps, "maps"), btn(t.btn_status, "status")],
        vec![btn(t.btn_add_map, "addmap"), btn(t.btn_add_host, "addhost")],
        vec![btn(t.btn_add_name, "addname")],
        vec![btn(t.btn_add_all, "addall")],
        vec![btn(notif_label, "notif"), btn(t.btn_lang, "lang")],
    ])
}

fn status_text(db: &Db, uid: i64, t: &'static T) -> String {
    let notif = if db.notifications_enabled(uid) {
        t.st_enabled
    } else {
        t.st_disabled
    };
    let subs = db.list_subs(uid);
    let active: Vec<&db::Sub> = subs.iter().filter(|s| s.enabled).collect();
    let mut text = format!(
        "{}\n\n{}: {}\n{}: {}\n{}: {}",
        t.st_hdr,
        t.st_notifications,
        notif,
        t.st_total,
        subs.len(),
        t.st_active_count,
        active.len()
    );
    if active.is_empty() {
        text.push_str("\n\n");
        text.push_str(t.st_no_active);
    } else {
        let kinds: [(&str, &str); 3] = [
            (db::KIND_MAP, t.kind_map),
            (db::KIND_HOST, t.kind_host),
            (db::KIND_NAME, t.kind_name),
        ];
        for (kind, label) in kinds {
            let items: Vec<&db::Sub> = active.iter().copied().filter(|s| s.kind == kind).collect();
            if items.is_empty() {
                continue;
            }
            text.push_str(&format!("\n\n{}:", label));
            for s in items {
                text.push_str(&format!("\n• {}", s.pattern));
            }
        }
    }

    // suppressed (per-map) section
    let mutes = db.list_map_mutes(uid);
    text.push_str(&format!("\n\n{}:", t.st_muted_hdr));
    if mutes.is_empty() {
        text.push_str(&format!("\n{}", t.st_muted_empty));
    } else {
        let now = crate::db::now_ts();
        for m in mutes {
            let dur = match m.until {
                None => t.st_forever.to_string(),
                Some(until) => {
                    let secs = (until - now).max(0);
                    let h = secs / 3600;
                    let min = (secs % 3600) / 60;
                    format!("{h}{} {min}{} {}", t.st_h, t.st_m, t.st_remaining)
                }
            };
            text.push_str(&format!("\n• {} — {}", m.map, dur));
        }
    }
    text
}

fn kind_label(s: &db::Sub, t: &'static T) -> &'static str {
    match s.kind.as_str() {
        db::KIND_HOST => t.kind_host,
        db::KIND_NAME => t.kind_name,
        _ => t.kind_map,
    }
}

fn maps_text(state: &AppState, uid: i64, t: &'static T) -> String {
    let subs = state.db.list_subs(uid);
    if subs.is_empty() {
        return t.maps_empty.into();
    }
    let mut text = t.maps_title.replace("{n}", &subs.len().to_string());
    text.push_str(t.maps_hint);
    for s in &subs {
        let icon = if s.enabled { "✅" } else { "❌" };
        text.push_str(&format!("\n{} {} {}", icon, kind_label(s, t), s.pattern));
    }
    text
}

fn maps_kb(subs: &[db::Sub], t: &'static T) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = subs
        .iter()
        .map(|s| {
            vec![btn(
                &format!("{} {}", kind_label(s, t), s.pattern),
                &format!("map:{}", s.id),
            )]
        })
        .collect();
    rows.push(vec![
        btn(t.btn_add_map, "addmap"),
        btn(t.btn_add_host, "addhost"),
    ]);
    rows.push(vec![
        btn(t.btn_add_name, "addname"),
        btn(t.btn_add_all, "addall"),
    ]);
    rows.push(vec![btn(t.btn_main_menu, "menu")]);
    InlineKeyboardMarkup::new(rows)
}

fn sub_view(s: &db::Sub, t: &'static T) -> (String, InlineKeyboardMarkup) {
    let status = if s.enabled { t.sub_enabled } else { t.sub_disabled };
    let desc = match s.kind.as_str() {
        db::KIND_HOST => t.sub_desc_host,
        db::KIND_NAME => t.sub_desc_name,
        _ => t.sub_desc_map,
    };
    let text = format!(
        "{}\n\n{}: {}\n{}: {}\n{}: {}\n\n{}",
        t.sub_id.replace("{id}", &s.id.to_string()),
        t.sub_name,
        s.pattern,
        t.sub_type,
        kind_label(s, t),
        t.sub_status,
        status,
        desc,
    );
    let toggle_label = if s.enabled { t.btn_disable } else { t.btn_enable };
    let kb = InlineKeyboardMarkup::new(vec![
        vec![btn(toggle_label, &format!("toggle:{}", s.id))],
        vec![
            btn(t.btn_rename, &format!("rename:{}", s.id)),
            btn(t.btn_delete, &format!("del:{}", s.id)),
        ],
        vec![btn(t.btn_to_list, "maps"), btn(t.btn_menu, "menu")],
    ]);
    (text, kb)
}

/// Клавиатура для режимов ввода текста (добавление/переименование).
fn cancel_kb(t: &'static T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![btn(t.btn_cancel, "cancel")]])
}

/// Клавиатура "Готово / Отмена" для режима массового добавления.
fn done_kb(t: &'static T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![btn(t.btn_done, "done")],
        vec![btn(t.btn_cancel, "cancel")],
    ])
}

async fn show(
    bot: Bot,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    text: String,
    kb: InlineKeyboardMarkup,
) -> ResponseResult<Option<MessageId>> {
    if let Some(mid) = message_id {
        if bot
            .edit_message_text(chat_id, mid, text.clone())
            .reply_markup(kb.clone())
            .await
            .is_ok()
        {
            return Ok(Some(mid));
        }
    }
    let m = bot.send_message(chat_id, text).reply_markup(kb).await?;
    Ok(Some(m.id))
}

async fn show_pinned(
    bot: Bot,
    chat_id: ChatId,
    message_id: Option<MessageId>,
    text: String,
    kb: InlineKeyboardMarkup,
    bot_id: UserId,
) -> ResponseResult<Option<MessageId>> {
    let mid = show(bot.clone(), chat_id, message_id, text, kb).await?;
    if let Some(id) = mid {
        let is_new = message_id != Some(id);
        let already_pinned_by_me = match bot.get_chat(chat_id).await {
            Ok(chat) => {
                if chat.is_private() {
                    true
                } else {
                    match chat.pinned_message {
                        Some(msg) => msg.from.as_ref().map_or(false, |u| u.id == bot_id),
                        None => false,
                    }
                }
            }
            Err(_) => true,
        };
        if is_new || !already_pinned_by_me {
            let _ = bot.unpin_chat_message(chat_id).await;
            let _ = bot
                .pin_chat_message(chat_id, id)
                .disable_notification(true)
                .await;
        }
    }
    Ok(mid)
}

/// Boxed, чтобы не раздувать комбинированный future dptree (переполнение стека в debug-сборке).
pub fn handle_message(
    bot: Bot,
    state: AppState,
    msg: Message,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        let Some(text) = msg.text() else { return Ok(()) };
        let chat_id = msg.chat.id;
        let uid = chat_id.0;
        state.db.ensure_user(uid);
        let lang = state.db.lang(uid);
        let t = tr(lang);
        let trimmed = text.trim();

        if trimmed.starts_with('/') {
            state.clear_pending(uid);
            match trimmed.split_whitespace().next().unwrap_or("") {
                "/start" | "/menu" | "/help" => {
                    println!("[start] uid={uid} chat={chat_id}");
                    show_pinned(
                        bot.clone(),
                        chat_id,
                        None,
                        t.welcome.into(),
                        main_menu_kb(&state, uid, t),
                        state.bot_id,
                    )
                    .await?;
                }
                "/maps" => {
                    let subs = state.db.list_subs(uid);
                    show(
                        bot.clone(),
                        chat_id,
                        None,
                        maps_text(&state, uid, t),
                        maps_kb(&subs, t),
                    )
                    .await?;
                }
                "/status" => {
                    show_pinned(
                        bot.clone(),
                        chat_id,
                        None,
                        status_text(&state.db, uid, t),
                        main_menu_kb(&state, uid, t),
                        state.bot_id,
                    )
                    .await?;
                }
                "/cancel" => {
                    state.clear_pending(uid);
                    show_pinned(
                        bot.clone(),
                        chat_id,
                        None,
                        t.msg_cancelled.into(),
                        main_menu_kb(&state, uid, t),
                        state.bot_id,
                    )
                    .await?;
                }
                "/done" => {
                    state.clear_pending(uid);
                    let subs = state.db.list_subs(uid);
                    show(
                        bot.clone(),
                        chat_id,
                        None,
                        maps_text(&state, uid, t),
                        maps_kb(&subs, t),
                    )
                    .await?;
                }
                "/stop" => {
                    state.clear_pending(uid);
                    state.db.delete_user(uid);
                    bot.send_message(chat_id, t.msg_stop).await?;
                }
                _ => {}
            }
            return Ok(());
        }

        match state.take_pending(uid) {
            Some(Pending::AddMap) => {
                add_sub_flow(bot, state, chat_id, db::KIND_MAP, trimmed, "🗺", t).await;
            }
            Some(Pending::AddHost) => {
                add_sub_flow(bot, state, chat_id, db::KIND_HOST, trimmed, "👤", t).await;
            }
            Some(Pending::AddName) => {
                add_sub_flow(bot, state, chat_id, db::KIND_NAME, trimmed, "📛", t).await;
            }
            Some(Pending::AddAll) => {
                add_all_flow(bot, state, chat_id, trimmed, t).await;
            }
            Some(Pending::Rename(id)) => {
                let new_name: String = trimmed.chars().take(MAX_PATTERN_LEN).collect();
                if new_name.trim().is_empty() {
                    bot.send_message(chat_id, t.msg_empty_name)
                        .reply_markup(cancel_kb(t))
                        .await?;
                } else if state.db.rename_sub(id, new_name.trim()).is_ok() {
                    state.clear_pending(uid);
                    if let Some(s) = state.db.get_sub(id) {
                        let (text, kb) = sub_view(&s, t);
                        show(bot, chat_id, None, text, kb).await?;
                    }
                } else {
                    bot.send_message(chat_id, t.msg_rename_fail)
                        .reply_markup(cancel_kb(t))
                        .await?;
                }
            }
            None => {
                bot.send_message(chat_id, t.msg_use_menu).await?;
            }
        }
        Ok(())
    })
}

async fn add_sub_flow(
    bot: Bot,
    state: AppState,
    chat_id: ChatId,
    kind: &'static str,
    input: &str,
    icon: &str,
    t: &'static T,
) {
    let name: String = input.chars().take(MAX_PATTERN_LEN).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        let _ = bot
            .send_message(chat_id, t.msg_empty_name)
            .reply_markup(done_kb(t))
            .await;
        return;
    }
    let uid = chat_id.0;
    let (text, kb) = match state.db.add_sub(uid, kind, &name) {
        Ok(true) => {
            println!("[add] uid={uid} kind={kind} name={name}");
            (
                format!(
                    "{}\n\n{}",
                    t.msg_added.replace("{icon}", icon).replace("{name}", &name),
                    t.msg_add_more
                ),
                done_kb(t),
            )
        }
        _ => (
            format!("{}\n\n{}", t.msg_duplicate, t.msg_add_more),
            done_kb(t),
        ),
    };
    state.set_pending(uid, if kind == db::KIND_HOST { Pending::AddHost } else if kind == db::KIND_NAME { Pending::AddName } else { Pending::AddMap });
    let _ = show(bot, chat_id, None, text, kb).await;
}

async fn add_all_flow(
    bot: Bot,
    state: AppState,
    chat_id: ChatId,
    input: &str,
    t: &'static T,
) {
    let name: String = input.chars().take(MAX_PATTERN_LEN).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        let _ = bot
            .send_message(chat_id, t.msg_empty_name)
            .reply_markup(done_kb(t))
            .await;
        return;
    }
    let uid = chat_id.0;
    let kinds = [(db::KIND_MAP, "🗺"), (db::KIND_HOST, "👤"), (db::KIND_NAME, "📛")];
    let mut added = Vec::new();
    for (kind, icon) in &kinds {
        if let Ok(true) = state.db.add_sub(uid, kind, &name) {
            println!("[add] uid={uid} kind={kind} name={name}");
            added.push(format!("{} {}", icon, kind_label_str(kind, t)));
        }
    }
    let text = if added.is_empty() {
        format!("{}\n\n{}", t.msg_duplicate, t.msg_add_more)
    } else {
        let list = added.join(", ");
        t.msg_add_all_done
            .replace("{name}", &name)
            .replace("{list}", &list)
            .replace("{more}", t.msg_add_more)
    };
    state.set_pending(uid, Pending::AddAll);
    let _ = show(bot, chat_id, None, text, done_kb(t)).await;
}

fn kind_label_str(kind: &str, t: &'static T) -> &'static str {
    match kind {
        db::KIND_HOST => t.kind_host,
        db::KIND_NAME => t.kind_name,
        _ => t.kind_map,
    }
}

/// Boxed, чтобы не раздувать комбинированный future dptree (переполнение стека в debug-сборке).
pub fn handle_callback(
    bot: Bot,
    state: AppState,
    cq: CallbackQuery,
) -> BoxFuture<'static, ResponseResult<()>> {
    Box::pin(async move {
        let _ = bot.answer_callback_query(cq.id.clone()).await;
        let Some(data) = cq.data.as_deref() else { return Ok(()) };
        let uid = cq.from.id.0 as i64;
        state.db.ensure_user(uid);
        let (chat_id, mid) = match cq.regular_message() {
            Some(m) => (m.chat.id, Some(m.id)),
            None => (ChatId(uid), None),
        };

        match data {
            "lang" => {
                let new_lang = state.db.lang(uid).toggled();
                state.db.set_lang(uid, new_lang.code());
                // fall through to menu redraw below with the new language
                let t = tr(new_lang);
                show_pinned(bot, chat_id, mid, t.welcome.into(), main_menu_kb(&state, uid, t), state.bot_id).await?;
            }
            _ => {
                let lang = state.db.lang(uid);
                let t = tr(lang);
                route_callback(bot, state, data, uid, chat_id, mid, t, cq.id).await?;
            }
        }
        Ok(())
    })
}

async fn route_callback(
    bot: Bot,
    state: AppState,
    data: &str,
    uid: i64,
    chat_id: ChatId,
    mid: Option<MessageId>,
    t: &'static T,
    cq_id: String,
) -> ResponseResult<()> {
    match data {
        "menu" => {
            show_pinned(bot, chat_id, mid, t.welcome.into(), main_menu_kb(&state, uid, t), state.bot_id).await?;
        }
        "maps" => {
            let subs = state.db.list_subs(uid);
            show(bot, chat_id, mid, maps_text(&state, uid, t), maps_kb(&subs, t)).await?;
        }
        "status" | "notif" => {
            if data == "notif" {
                let on = !state.db.notifications_enabled(uid);
                state.db.set_notifications(uid, on);
            }
            show_pinned(
                bot,
                chat_id,
                mid,
                status_text(&state.db, uid, t),
                main_menu_kb(&state, uid, t),
                state.bot_id,
            )
            .await?;
        }
        "addmap" => {
            state.set_pending(uid, Pending::AddMap);
            bot.send_message(chat_id, t.prompt_add_map)
                .reply_markup(cancel_kb(t))
                .await?;
        }
        "addhost" => {
            state.set_pending(uid, Pending::AddHost);
            bot.send_message(chat_id, t.prompt_add_host)
                .reply_markup(cancel_kb(t))
                .await?;
        }
        "addname" => {
            state.set_pending(uid, Pending::AddName);
            bot.send_message(chat_id, t.prompt_add_name)
                .reply_markup(cancel_kb(t))
                .await?;
        }
        "addall" => {
            state.set_pending(uid, Pending::AddAll);
            bot.send_message(chat_id, t.prompt_add_all)
                .reply_markup(cancel_kb(t))
                .await?;
        }
        "cancel" => {
            state.clear_pending(uid);
            show_pinned(
                bot,
                chat_id,
                mid,
                t.msg_cancelled.into(),
                main_menu_kb(&state, uid, t),
                state.bot_id,
            )
            .await?;
        }
        "done" => {
            state.clear_pending(uid);
            let subs = state.db.list_subs(uid);
            show(
                bot,
                chat_id,
                mid,
                maps_text(&state, uid, t),
                maps_kb(&subs, t),
            )
            .await?;
        }
        "check" => {
            if let Some(mid) = mid {
                let _ = bot.delete_message(chat_id, mid).await;
                state.deleted.lock().unwrap().insert((chat_id.0, mid.0));
            }
        }
        d => {
            let mut parts = d.splitn(2, ':');
            let op = parts.next().unwrap_or("");
            match op {
                "snooze" => {
                    if let Some(map) = parts.next() {
                        state.db.snooze_map(uid, map);
                        let _ = bot
                            .answer_callback_query(cq_id)
                            .text(t.msg_snoozed)
                            .show_alert(false)
                            .await;
                        if let Some(mid) = mid {
                            let _ = bot.delete_message(chat_id, mid).await;
                            state.deleted.lock().unwrap().insert((chat_id.0, mid.0));
                        }
                    }
                }
                "mute" => {
                    if let Some(map) = parts.next() {
                        state.db.mute_map(uid, map);
                        let _ = bot
                            .answer_callback_query(cq_id)
                            .text(t.msg_muted)
                            .show_alert(false)
                            .await;
                        if let Some(mid) = mid {
                            let _ = bot.delete_message(chat_id, mid).await;
                            state.deleted.lock().unwrap().insert((chat_id.0, mid.0));
                        }
                    }
                }
                _ => {
                    let Some(id) = parts.next().and_then(|v| v.parse::<i64>().ok()) else {
                        return Ok(());
                    };
                    match op {
                        "map" => {
                            if let Some(s) = owned_sub(&state, id, uid) {
                                let (text, kb) = sub_view(&s, t);
                                show(bot, chat_id, mid, text, kb).await?;
                            }
                        }
                        "toggle" => {
                            if let Some(s) = owned_sub(&state, id, uid) {
                                state.db.set_sub_enabled(id, !s.enabled);
                                let s = state.db.get_sub(id).unwrap();
                                let (text, kb) = sub_view(&s, t);
                                show(bot, chat_id, mid, text, kb).await?;
                            }
                        }
                        "rename" => {
                            if let Some(s) = owned_sub(&state, id, uid) {
                                state.set_pending(uid, Pending::Rename(id));
                                bot.send_message(chat_id, t.prompt_rename.replace("{name}", &s.pattern))
                                    .reply_markup(cancel_kb(t))
                                    .await?;
                            }
                        }
                        "del" => {
                            if let Some(s) = owned_sub(&state, id, uid) {
                                println!("[del] uid={uid} id={id} name={}", s.pattern);
                                state.db.delete_sub(id);
                                let subs = state.db.list_subs(uid);
                                show(
                                    bot,
                                    chat_id,
                                    mid,
                                    t.msg_deleted.replace("{name}", &s.pattern),
                                    maps_kb(&subs, t),
                                )
                                .await?;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

fn owned_sub(state: &AppState, id: i64, uid: i64) -> Option<db::Sub> {
    state.db.get_sub(id).filter(|s| s.chat_id == uid)
}

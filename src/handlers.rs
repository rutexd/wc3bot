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

fn sanitize_input(input: &str) -> String {
    let s: String = input.chars().take(MAX_PATTERN_LEN).collect();
    s.trim().to_string()
}

#[derive(Debug, Clone)]
pub enum Pending {
    AddMap,
    AddHost,
    AddName,
    AddAll,
    Rename(i64),
    AddHostFilter(i64),
    QhStart,
    QhEnd { start_min: i32 },
    QhTz { start_min: i32, end_min: i32 },
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

    /// Delete a tracked message and mark it so the poller skips it.
    pub fn mark_deleted(&self, chat_id: i64, msg_id: i32) {
        let _ = self.deleted.lock().unwrap().insert((chat_id, msg_id));
    }
}

fn btn(text: &str, data: &str) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(text.to_string(), data.to_string())
}

/// 2x2 grid of add buttons used in every menu.
fn add_buttons_kb(t: &'static T) -> Vec<Vec<InlineKeyboardButton>> {
    vec![
        vec![btn(t.btn_add_map, "addmap"), btn(t.btn_add_name, "addname")],
        vec![btn(t.btn_add_host, "addhost"), btn(t.btn_add_all, "addall")],
    ]
}

fn main_menu_kb(state: &AppState, uid: i64, t: &'static T) -> InlineKeyboardMarkup {
    let notif_label = if state.db.notifications_enabled(uid) {
        t.btn_notif_on
    } else {
        t.btn_notif_off
    };
    let mut rows = vec![
        vec![btn(t.btn_manage, "manage")],
    ];
    rows.extend(add_buttons_kb(t));
    rows.push(vec![btn(notif_label, "notif"), btn(t.btn_settings, "settings")]);
    rows.push(vec![btn(t.btn_lang, "lang")]);
    InlineKeyboardMarkup::new(rows)
}

fn status_body(db: &Db, uid: i64, t: &'static T) -> String {
    let lang = db.lang(uid);
    let notif = if db.notifications_enabled(uid) {
        t.st_enabled
    } else {
        t.st_disabled
    };
    let subs = db.list_subs(uid);
    let active_count = subs.iter().filter(|s| s.enabled).count();
    let mut text = format!(
        "{}\n\n{}: {}\n{}: {}\n{}: {}",
        t.st_hdr,
        t.st_notifications,
        notif,
        t.st_total,
        subs.len(),
        t.st_active_count,
        active_count
    );
    if active_count == 0 {
        text.push_str("\n\n");
        text.push_str(t.st_no_active);
    } else {
        for (kind, label) in [(db::KIND_MAP, t.kind_map), (db::KIND_HOST, t.kind_host), (db::KIND_NAME, t.kind_name)] {
            let items: Vec<_> = subs.iter().filter(|s| s.enabled && s.kind == kind).collect();
            if items.is_empty() {
                continue;
            }
            text.push_str(&format!("\n\n{}:", label));
            for s in items {
                text.push_str(&format!("\n• {}", s.pattern));
            }
        }
    }

    let mutes = db.list_map_mutes(uid);
    text.push_str(&format!("\n\n{}:", t.st_muted_hdr));
    if mutes.is_empty() {
        text.push_str(&format!("\n{}", t.st_muted_empty));
    } else {
        let now = crate::db::now_ts();
        for m in &mutes {
            let dur = match m.until {
                None => t.st_forever.to_string(),
                Some(until) => {
                    let secs = (until - now).max(0);
                    format!("{}{} {}{} {}", secs / 3600, t.st_h, (secs % 3600) / 60, t.st_m, t.st_remaining)
                }
            };
            text.push_str(&format!("\n• {} {} — {}", kind_label(&m.kind, t), m.pattern, dur));
        }
    }

    text.push_str(&format!("\n\n{}:", t.st_quiet_hdr));
    match db.get_quiet_hours(uid) {
        None => text.push_str(&format!("\n{}", t.quiet_off)),
        Some(qh) => {
            let start = lang.format_minutes(qh.start_min);
            let end = lang.format_minutes(qh.end_min);
            let tz = lang.format_tz_offset(qh.tz_offset_min);
            text.push_str(&format!("\n• {}–{} ({})", start, end, tz));
        }
    }

    text
}

/// Combined View & Manage screen: status body + interactive sub buttons.
fn manage_text(db: &Db, uid: i64, t: &'static T) -> String {
    let subs = db.list_subs(uid);
    let mut text = status_body(db, uid, t);
    text.push_str(&format!("\n\n{}", t.manage_title.replace("{n}", &subs.len().to_string())));
    if subs.is_empty() {
        text.push_str(&format!("\n{}", t.manage_empty));
    } else {
        text.push_str(&format!("\n{}\n", t.manage_subs_hdr));
        for s in &subs {
            let icon = if s.enabled { "✅" } else { "❌" };
            text.push_str(&format!("\n• {} {} {}", icon, kind_label(&s.kind, t), s.pattern));
        }
        text.push_str(t.manage_hint);
    }
    text
}

fn manage_kb(state: &AppState, uid: i64, t: &'static T) -> InlineKeyboardMarkup {
    let subs = state.db.list_subs(uid);
    let notif_label = if state.db.notifications_enabled(uid) {
        t.btn_notif_on
    } else {
        t.btn_notif_off
    };
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for s in &subs {
        rows.push(vec![btn(
            &format!("{} {}", kind_label(&s.kind, t), s.pattern),
            &format!("map:{}", s.id),
        )]);
    }
    rows.extend(add_buttons_kb(t));
    rows.push(vec![btn(notif_label, "notif"), btn(t.btn_settings, "settings")]);
    rows.push(vec![btn(t.btn_lang, "lang")]);
    rows.push(vec![btn(t.btn_menu, "menu")]);
    InlineKeyboardMarkup::new(rows)
}

fn kind_label(kind: &str, t: &'static T) -> &'static str {
    match kind {
        db::KIND_HOST => t.kind_host,
        db::KIND_NAME => t.kind_name,
        _ => t.kind_map,
    }
}

fn kind_to_pending(kind: &str) -> Pending {
    match kind {
        db::KIND_HOST => Pending::AddHost,
        db::KIND_NAME => Pending::AddName,
        _ => Pending::AddMap,
    }
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
        kind_label(&s.kind, t),
        t.sub_status,
        status,
        desc,
    );
    let toggle_label = if s.enabled { t.btn_disable } else { t.btn_enable };
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    rows.push(vec![btn(toggle_label, &format!("toggle:{}", s.id))]);
    let mut mid_row = vec![
        btn(t.btn_rename, &format!("rename:{}", s.id)),
        btn(t.btn_delete, &format!("del:{}", s.id)),
    ];
    if s.kind == db::KIND_MAP || s.kind == db::KIND_NAME {
        mid_row.push(btn(t.btn_hosts, &format!("hosts:{}", s.id)));
    }
    rows.push(mid_row);
    rows.push(vec![btn(t.btn_to_manage, "manage"), btn(t.btn_menu, "menu")]);
    (text, InlineKeyboardMarkup::new(rows))
}

/// Клавиатура для режимов ввода текста (добавление/переименование).
fn cancel_kb(t: &'static T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![btn(t.btn_cancel, "cancel")]])
}

fn hf_mode_info(mode: &str, t: &'static T) -> (&'static str, &'static str) {
    match mode {
        db::HF_WHITELIST => (t.hf_mode_wl, t.hf_desc_wl),
        db::HF_BLACKLIST => (t.hf_mode_bl, t.hf_desc_bl),
        _ => (t.hf_mode_off, ""),
    }
}

fn host_filter_screen(
    sub: &db::Sub,
    mode: &str,
    hosts: &[String],
    t: &'static T,
) -> (String, InlineKeyboardMarkup) {
    let title = t.hf_title.replace("{name}", &format!("{} {}", kind_label(&sub.kind, t), sub.pattern));
    let (mode_label, desc) = hf_mode_info(mode, t);

    let mut text = format!("{}\n\n{}: {}", title, t.hf_mode, mode_label);
    if !desc.is_empty() {
        text.push_str(&format!("\n{}", desc));
    }
    text.push_str(&format!("\n\n{}:", t.hf_hosts_hdr));
    if hosts.is_empty() {
        text.push_str(&format!("\n{}", t.hf_hosts_empty));
    } else {
        for h in hosts {
            text.push_str(&format!("\n• {}", h));
        }
    }

    let mut kb_rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for h in hosts {
        kb_rows.push(vec![
            btn(h, &format!("hback:{}", sub.id)),
            btn("🗑", &format!("hdel:{}:{}", sub.id, h)),
        ]);
    }
    kb_rows.push(vec![btn(t.btn_toggle_mode, &format!("hmode:{}", sub.id))]);
    kb_rows.push(vec![btn(t.btn_add_hf_host, &format!("hadd:{}", sub.id))]);
    kb_rows.push(vec![btn(t.btn_to_manage, &format!("hback:{}", sub.id))]);
    let kb = InlineKeyboardMarkup::new(kb_rows);
    (text, kb)
}

/// Клавиатура "Готово / Отмена" для режима массового добавления.
fn done_kb(t: &'static T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![btn(t.btn_done, "done")],
        vec![btn(t.btn_cancel, "cancel")],
    ])
}

fn settings_text(t: &'static T) -> String {
    t.btn_settings.into()
}

fn settings_kb(t: &'static T) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![btn(t.btn_quiet, "quiet")],
        vec![btn(t.btn_menu, "menu")],
    ])
}

fn quiet_screen_text(db: &Db, uid: i64, t: &'static T) -> String {
    let lang = db.lang(uid);
    match db.get_quiet_hours(uid) {
        None => t.quiet_disabled.into(),
        Some(qh) => {
            let start = lang.format_minutes(qh.start_min);
            let end = lang.format_minutes(qh.end_min);
            let tz = lang.format_tz_offset(qh.tz_offset_min);
            let status = if db.is_in_quiet_hours(uid) { t.quiet_on } else { t.quiet_off };
            format!(
                "{}\n\n{}: {}",
                t.quiet_title.replace("{start}", &start).replace("{end}", &end).replace("{tz}", &tz),
                t.btn_quiet,
                status
            )
        }
    }
}

fn quiet_kb(db: &Db, uid: i64, t: &'static T) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    rows.push(vec![btn(t.btn_quiet_setup, "qh_setup")]);
    if db.get_quiet_hours(uid).is_some() {
        rows.push(vec![btn(t.btn_quiet_disable, "qh_off")]);
    }
    rows.push(vec![btn(t.btn_quiet_back, "settings")]);
    InlineKeyboardMarkup::new(rows)
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
                "/manage" => {
                    show_pinned(
                        bot.clone(),
                        chat_id,
                        None,
                        manage_text(&state.db, uid, t),
                        manage_kb(&state, uid, t),
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
                    show_pinned(
                        bot.clone(),
                        chat_id,
                        None,
                        manage_text(&state.db, uid, t),
                        manage_kb(&state, uid, t),
                        state.bot_id,
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
            Some(p @ (Pending::AddMap | Pending::AddHost | Pending::AddName)) => {
                let (kind, icon) = match &p {
                    Pending::AddHost => (db::KIND_HOST, "👤"),
                    Pending::AddName => (db::KIND_NAME, "📛"),
                    _ => (db::KIND_MAP, "🗺"),
                };
                add_sub_flow(bot, state, chat_id, kind, trimmed, icon, t).await;
            }
            Some(Pending::AddAll) => {
                add_all_flow(bot, state, chat_id, trimmed, t).await;
            }
            Some(Pending::QhStart) => {
                match crate::quiet::parse_hhmm(trimmed) {
                    Some(start) => {
                        state.set_pending(uid, Pending::QhEnd { start_min: start });
                        bot.send_message(chat_id, t.prompt_qh_end)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                    None => {
                        state.set_pending(uid, Pending::QhStart);
                        bot.send_message(chat_id, t.msg_qh_bad_time)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                }
            }
            Some(Pending::QhEnd { start_min }) => {
                match crate::quiet::parse_hhmm(trimmed) {
                    Some(end) if end != start_min => {
                        state.set_pending(uid, Pending::QhTz { start_min, end_min: end });
                        bot.send_message(chat_id, t.prompt_qh_tz)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                    Some(_) => {
                        state.set_pending(uid, Pending::QhEnd { start_min });
                        bot.send_message(chat_id, t.msg_qh_invalid_range)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                    None => {
                        state.set_pending(uid, Pending::QhEnd { start_min });
                        bot.send_message(chat_id, t.msg_qh_bad_time)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                }
            }
            Some(Pending::QhTz { start_min, end_min }) => {
                match crate::quiet::parse_utc_offset(trimmed) {
                    Some(tz) => {
                        state.db.set_quiet_hours(uid, tz, start_min, end_min);
                        state.clear_pending(uid);
                        let lang = state.db.lang(uid);
                        let msg = t.msg_qh_saved
                            .replace("{start}", &lang.format_minutes(start_min))
                            .replace("{end}", &lang.format_minutes(end_min))
                            .replace("{tz}", &lang.format_tz_offset(tz));
                        show_pinned(
                            bot.clone(),
                            chat_id,
                            None,
                            quiet_screen_text(&state.db, uid, t),
                            quiet_kb(&state.db, uid, t),
                            state.bot_id,
                        )
                        .await?;
                        bot.send_message(chat_id, msg).await?;
                    }
                    None => {
                        state.set_pending(uid, Pending::QhTz { start_min, end_min });
                        bot.send_message(chat_id, t.msg_qh_bad_tz)
                            .reply_markup(cancel_kb(t))
                            .await?;
                    }
                }
            }
            Some(Pending::Rename(id)) => {
                let new_name = sanitize_input(trimmed);
                if new_name.is_empty() {
                    bot.send_message(chat_id, t.msg_empty_name)
                        .reply_markup(cancel_kb(t))
                        .await?;
                } else if state.db.rename_sub(id, &new_name).is_ok() {
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
            Some(Pending::AddHostFilter(sub_id)) => {
                let host = sanitize_input(trimmed);
                if host.is_empty() {
                    bot.send_message(chat_id, t.msg_empty_name)
                        .reply_markup(cancel_kb(t))
                        .await?;
                } else if let Some(s) = owned_sub(&state, sub_id, uid) {
                    state.db.add_sub_host(sub_id, &host);
                    bot.send_message(chat_id, t.msg_hf_host_added.replace("{host}", &host))
                        .await?;
                    let (mode, hosts) = state.db.get_host_filter(sub_id);
                    let (text, kb) = host_filter_screen(&s, &mode, &hosts, t);
                    show(bot, chat_id, None, text, kb).await?;
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
    let name = sanitize_input(input);
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
    state.set_pending(uid, kind_to_pending(kind));
    let _ = show(bot, chat_id, None, text, kb).await;
}

async fn add_all_flow(
    bot: Bot,
    state: AppState,
    chat_id: ChatId,
    input: &str,
    t: &'static T,
) {
    let name = sanitize_input(input);
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
            added.push(format!("{} {}", icon, kind_label(kind, t)));
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
        "manage" => {
            show_pinned(
                bot,
                chat_id,
                mid,
                manage_text(&state.db, uid, t),
                manage_kb(&state, uid, t),
                state.bot_id,
            )
            .await?;
        }
        "settings" => {
            show_pinned(bot, chat_id, mid, settings_text(t), settings_kb(t), state.bot_id).await?;
        }
        "quiet" => {
            show_pinned(bot, chat_id, mid, quiet_screen_text(&state.db, uid, t), quiet_kb(&state.db, uid, t), state.bot_id).await?;
        }
        "qh_setup" => {
            state.set_pending(uid, Pending::QhStart);
            bot.send_message(chat_id, t.prompt_qh_start).reply_markup(cancel_kb(t)).await?;
        }
        "qh_off" => {
            state.db.disable_quiet_hours(uid);
            let _ = bot.answer_callback_query(cq_id).text(t.msg_qh_disabled).await;
            show_pinned(bot, chat_id, mid, quiet_screen_text(&state.db, uid, t), quiet_kb(&state.db, uid, t), state.bot_id).await?;
        }
        "notif" => {
            let on = !state.db.notifications_enabled(uid);
            state.db.set_notifications(uid, on);
            show_pinned(
                bot,
                chat_id,
                mid,
                manage_text(&state.db, uid, t),
                manage_kb(&state, uid, t),
                state.bot_id,
            )
            .await?;
        }
        "addmap" | "addhost" | "addname" => {
            let (pending, prompt) = match data {
                "addhost" => (Pending::AddHost, t.prompt_add_host),
                "addname" => (Pending::AddName, t.prompt_add_name),
                _ => (Pending::AddMap, t.prompt_add_map),
            };
            state.set_pending(uid, pending);
            bot.send_message(chat_id, prompt)
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
            show_pinned(
                bot,
                chat_id,
                mid,
                manage_text(&state.db, uid, t),
                manage_kb(&state, uid, t),
                state.bot_id,
            )
            .await?;
        }
        "check" => {
            if let Some(mid) = mid {
                let _ = bot.delete_message(chat_id, mid).await;
                state.mark_deleted(chat_id.0, mid.0);
            }
        }
        d => {
            // snooze / mute: "snooze:{kind}:{pattern}" / "mute:{kind}:{pattern}"
            // kind is a fixed token (map|host|name); pattern is everything after it.
            if let Some((op, rest)) = d.split_once(':') {
                if op == "snooze" || op == "mute" {
                    if let Some((kind, pattern)) = rest.split_once(':') {
                        if matches!(kind, db::KIND_MAP | db::KIND_HOST | db::KIND_NAME) {
                            if op == "snooze" {
                                state.db.snooze_sub(uid, kind, pattern);
                            } else {
                                state.db.mute_sub(uid, kind, pattern);
                            }
                            let msg = if op == "snooze" { t.msg_snoozed } else { t.msg_muted };
                            let _ = bot
                                .answer_callback_query(cq_id)
                                .text(msg)
                                .show_alert(false)
                                .await;
                            if let Some(mid) = mid {
                                let _ = bot.delete_message(chat_id, mid).await;
                                state.mark_deleted(chat_id.0, mid.0);
                            }
                            return Ok(());
                        }
                    }
                }
            }

            // All other callbacks: op:sub_id[:extra]
            let Some(cb) = SubCallback::parse(d) else {
                return Ok(());
            };
            let Some((cb, sub)) = cb.lookup(&state, uid) else {
                return Ok(());
            };
            match cb.op.as_str() {
                "hosts" => {
                    let (mode, hosts) = state.db.get_host_filter(cb.id);
                    let (text, kb) = host_filter_screen(&sub, &mode, &hosts, t);
                    show(bot, chat_id, mid, text, kb).await?;
                }
                "hmode" => {
                    let (mode, _) = state.db.get_host_filter(cb.id);
                    let new_mode = match mode.as_str() {
                        db::HF_OFF => db::HF_WHITELIST,
                        db::HF_WHITELIST => db::HF_BLACKLIST,
                        _ => db::HF_OFF,
                    };
                    state.db.set_host_filter_mode(cb.id, new_mode);
                    let (mode, hosts) = state.db.get_host_filter(cb.id);
                    let (text, kb) = host_filter_screen(&sub, &mode, &hosts, t);
                    show(bot, chat_id, mid, text, kb).await?;
                }
                "hadd" => {
                    state.set_pending(uid, Pending::AddHostFilter(cb.id));
                    bot.send_message(chat_id, t.prompt_add_hf_host)
                        .reply_markup(cancel_kb(t))
                        .await?;
                }
                "hdel" => {
                    if let Some(host) = cb.extra.as_deref() {
                        state.db.remove_sub_host(cb.id, host);
                        let _ = bot
                            .answer_callback_query(cq_id)
                            .text(t.msg_hf_host_deleted.replace("{host}", host))
                            .show_alert(false)
                            .await;
                        let (mode, hosts) = state.db.get_host_filter(cb.id);
                        let (text, kb) = host_filter_screen(&sub, &mode, &hosts, t);
                        show(bot, chat_id, mid, text, kb).await?;
                    }
                }
                "map" | "hback" => {
                    let (text, kb) = sub_view(&sub, t);
                    show(bot, chat_id, mid, text, kb).await?;
                }
                "toggle" => {
                    state.db.set_sub_enabled(cb.id, !sub.enabled);
                    let sub = state.db.get_sub(cb.id).unwrap();
                    let (text, kb) = sub_view(&sub, t);
                    show(bot, chat_id, mid, text, kb).await?;
                }
                "rename" => {
                    state.set_pending(uid, Pending::Rename(cb.id));
                    bot.send_message(chat_id, t.prompt_rename.replace("{name}", &sub.pattern))
                        .reply_markup(cancel_kb(t))
                        .await?;
                }
                "del" => {
                    println!("[del] uid={uid} id={} name={}", cb.id, sub.pattern);
                    state.db.delete_sub(cb.id);
                    state.db.delete_map_mute(uid, &sub.kind, &sub.pattern);
                    show_pinned(
                        bot,
                        chat_id,
                        mid,
                        t.msg_deleted.replace("{name}", &sub.pattern),
                        manage_kb(&state, uid, t),
                        state.bot_id,
                    )
                    .await?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn owned_sub(state: &AppState, id: i64, uid: i64) -> Option<db::Sub> {
    state.db.get_sub(id).filter(|s| s.chat_id == uid)
}

/// Parsed callback data for sub-related operations: `op:sub_id[:extra]`.
struct SubCallback {
    op: String,
    id: i64,
    extra: Option<String>,
}

impl SubCallback {
    fn parse(data: &str) -> Option<Self> {
        let mut parts = data.splitn(3, ':');
        let op = parts.next()?.to_string();
        let id = parts.next()?.parse::<i64>().ok()?;
        let extra = parts.next().map(String::from);
        Some(Self { op, id, extra })
    }

    fn lookup(self, state: &AppState, uid: i64) -> Option<(Self, db::Sub)> {
        let sub = owned_sub(state, self.id, uid)?;
        Some((self, sub))
    }
}

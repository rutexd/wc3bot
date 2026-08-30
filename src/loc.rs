#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    pub fn parse(s: &str) -> Lang {
        if s.eq_ignore_ascii_case("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
        }
    }

    pub fn toggled(self) -> Lang {
        match self {
            Lang::En => Lang::Ru,
            Lang::Ru => Lang::En,
        }
    }
}

pub struct T {
    // Buttons
    pub btn_maps: &'static str,
    pub btn_status: &'static str,
    pub btn_add_map: &'static str,
    pub btn_add_host: &'static str,
    pub btn_add_name: &'static str,
    pub btn_add_all: &'static str,
    pub btn_notif_on: &'static str,
    pub btn_notif_off: &'static str,
    pub btn_lang: &'static str,
    pub btn_enable: &'static str,
    pub btn_disable: &'static str,
    pub btn_rename: &'static str,
    pub btn_delete: &'static str,
    pub btn_to_list: &'static str,
    pub btn_menu: &'static str,
    pub btn_main_menu: &'static str,
    pub btn_cancel: &'static str,
    pub btn_done: &'static str,
    // Welcome / status
    pub welcome: &'static str,
    pub st_hdr: &'static str,
    pub st_notifications: &'static str,
    pub st_enabled: &'static str,
    pub st_disabled: &'static str,
    pub st_total: &'static str,
    pub st_active_count: &'static str,
    pub st_no_active: &'static str,
    pub st_muted_hdr: &'static str,
    pub st_muted_empty: &'static str,
    pub st_forever: &'static str,
    pub st_remaining: &'static str,
    pub st_h: &'static str,
    pub st_m: &'static str,
    // Maps list
    pub maps_title: &'static str, // {n}
    pub maps_hint: &'static str,
    pub maps_empty: &'static str,
    // Subscription view
    pub sub_id: &'static str, // {id}
    pub sub_name: &'static str,
    pub sub_type: &'static str,
    pub sub_status: &'static str,
    pub sub_enabled: &'static str,
    pub sub_disabled: &'static str,
    pub sub_desc_map: &'static str,
    pub sub_desc_host: &'static str,
    pub sub_desc_name: &'static str,
    pub kind_map: &'static str,
    pub kind_host: &'static str,
    pub kind_name: &'static str,
    // Prompts
    pub prompt_add_map: &'static str,
    pub prompt_add_host: &'static str,
    pub prompt_add_name: &'static str,
    pub prompt_add_all: &'static str,
    pub prompt_rename: &'static str, // {name}
    // Messages
    pub msg_cancelled: &'static str,
    pub msg_empty_name: &'static str,
    pub msg_duplicate: &'static str,
    pub msg_rename_fail: &'static str,
    pub msg_added: &'static str,   // {icon} {name}
    pub msg_deleted: &'static str, // {name}
    pub msg_use_menu: &'static str,
    pub msg_stop: &'static str,
    pub msg_add_more: &'static str,
    // Pinger
    pub ping_map: &'static str,     // {map}
    pub ping_name: &'static str,    // {name} {server}
    pub ping_host: &'static str,    // {host}
    pub ping_slots: &'static str,   // {taken} {total}
    pub ping_created: &'static str, // {time}
    pub ping_started: &'static str, // {time}
    pub dash: &'static str,         // empty name placeholder
    pub msg_add_all_done: &'static str, // {name} {list}
    // Notification keyboard
    pub btn_snooze: &'static str,
    pub btn_mute: &'static str,
    pub btn_check: &'static str,
    pub msg_snoozed: &'static str,
    pub msg_muted: &'static str,
    // Host filter
    pub btn_hosts: &'static str,
    pub hf_title: &'static str,      // {name}
    pub hf_mode: &'static str,
    pub hf_mode_off: &'static str,
    pub hf_mode_wl: &'static str,
    pub hf_mode_bl: &'static str,
    pub hf_hosts_hdr: &'static str,
    pub hf_hosts_empty: &'static str,
    pub btn_toggle_mode: &'static str,
    pub btn_add_hf_host: &'static str,
    pub prompt_add_hf_host: &'static str,
    pub msg_hf_host_added: &'static str,   // {host}
    pub msg_hf_host_deleted: &'static str, // {host}
    pub hf_desc_wl: &'static str,
    pub hf_desc_bl: &'static str,
}

const EN: T = T {
    btn_maps: "🗺 My maps",
    btn_status: "📊 Status",
    btn_add_map: "➕ Map",
    btn_add_host: "➕ 👤 Host",
    btn_add_name: "➕ 📛 Name",
    btn_add_all: "➕ ✨ All types",
    btn_notif_on: "🔔 Notifications: ✅ on",
    btn_notif_off: "🔕 Notifications: ❌ off",
    btn_lang: "🌐 Language: English",
    btn_enable: "✅ Enable",
    btn_disable: "❌ Disable",
    btn_rename: "✏️ Rename",
    btn_delete: "🗑 Delete",
    btn_to_list: "⬅️ Back to list",
    btn_menu: "🏠 Menu",
    btn_main_menu: "🏠 Main menu",
    btn_cancel: "↩️ Cancel",
    btn_done: "✅ Done",
    welcome: "👋 Hi! I watch WarCraft 3 lobbies on wc3stats.\n\n\
              Add a subscription for a map or a host — I'll message you when a new lobby appears.\n\n\
              Names are matched case- and symbol-insensitively: \
              \u{00ab}CHS\u{00bb} matches both \u{00ab}CHS -\u{00bb} and \u{00ab}something*chs*\u{00bb}.",
    st_hdr: "📊 Status:",
    st_notifications: "Notifications",
    st_enabled: "✅ Enabled",
    st_disabled: "❌ Disabled",
    st_total: "Total maps",
    st_active_count: "Active maps",
    st_no_active: "No active maps",
    st_muted_hdr: "Suppressed maps",
    st_muted_empty: "No suppressed maps",
    st_forever: "forever",
    st_remaining: "left",
    st_h: "h",
    st_m: "m",
    maps_title: "🗺 My subscriptions ({n}):",
    maps_hint: "\n\nTap a subscription to configure it.",
    maps_empty: "🗺 You have no subscriptions yet.\n\nAdd a map or a host using the buttons below.",
    sub_id: "Subscription #{id}",
    sub_name: "Name",
    sub_type: "Type",
    sub_status: "Status",
    sub_enabled: "✅ Enabled",
    sub_disabled: "❌ Disabled",
    sub_desc_map: "Notifications are sent when a new lobby's map name contains this name (case- and symbol-insensitive).",
    sub_desc_host: "Notifications are sent when a new lobby's host name contains this name (case- and symbol-insensitive).",
    sub_desc_name: "Notifications are sent when a new lobby's game name contains this name (case- and symbol-insensitive).",
    kind_map: "🗺 Map",
    kind_host: "👤 Host",
    kind_name: "📛 Name",
    prompt_add_map: "🗺 Send me the map name to track.\n\nExamples: pudge, chs, legion td",
    prompt_add_host: "👤 Send me the host name to track.\n\nExamples: HellWolf#31976, hellwolf\n\n⚠️ Host subscriptions don't support whitelist/blacklist.",
    prompt_add_name: "📛 Send me the game name to track.\n\nExamples: dota, tavern, fun",
    prompt_add_all: "✨ Send me a name — I'll subscribe you to map, host, and game name at once.\n\n⚠️ Combined subscriptions don't support whitelist/blacklist.",
    prompt_rename: "✏️ Current name: \u{00ab}{name}\u{00bb}\n\nSend me the new name:",
    msg_cancelled: "↩️ Cancelled.",
    msg_empty_name: "⚠️ Name cannot be empty. Try again:",
    msg_duplicate: "⚠️ Such subscription already exists.",
    msg_rename_fail: "⚠️ Failed to rename. Try again:",
    msg_added: "{icon} Subscription \u{00ab}{name}\u{00bb} added!",
    msg_deleted: "🗑 Subscription \u{00ab}{name}\u{00bb} deleted.",
    msg_use_menu: "🤔 Use the menu buttons or /start.",
    msg_stop: "👋 All your data has been deleted. /start to start over.",
    msg_add_more: "Send another name or press Done to finish.",
    ping_map: "🗺️ Map: {map}",
    ping_name: "📛 Name: {name} ({server})",
    ping_host: "🏠 Host: {host}",
    ping_slots: "👥 Players: {taken}/{total}",
    ping_created: "⏱ Created: {time} ago",
    ping_started: "✅ Started: after {time}",
    dash: "—",
    msg_add_all_done: "✅ {name} — {list}\n\n{more}",
    btn_snooze: "😴 12h",
    btn_mute: "🔕 Off",
    btn_check: "✅",
    msg_snoozed: "🔕 Notifications off for 12 hours",
    msg_muted: "🔕 Notifications turned off",
    btn_hosts: "⚙️ Hosts",
    hf_title: "⚙️ Host filter: {name}",
    hf_mode: "Mode",
    hf_mode_off: "Off",
    hf_mode_wl: "Whitelist",
    hf_mode_bl: "Blacklist",
    hf_hosts_hdr: "Hosts",
    hf_hosts_empty: "No hosts added",
    btn_toggle_mode: "🔄 Toggle mode",
    btn_add_hf_host: "➕ Add host",
    prompt_add_hf_host: "👤 Send the host name to add to the filter.\n\nExamples: HellWolf#31976, hellwolf",
    msg_hf_host_added: "✅ Host \u{00ab}{host}\u{00bb} added.",
    msg_hf_host_deleted: "🗑 Host \u{00ab}{host}\u{00bb} removed.",
    hf_desc_wl: "Only notify when the host is in the list.",
    hf_desc_bl: "Skip games hosted by players in the list.",
};

const RU: T = T {
    btn_maps: "🗺 Мои карты",
    btn_status: "📊 Статус",
    btn_add_map: "➕ Карта",
    btn_add_host: "➕ 👤 Хост",
    btn_add_name: "➕ 📛 Название",
    btn_add_all: "➕ ✨ Все типы",
    btn_notif_on: "🔔 Уведомления: ✅ вкл",
    btn_notif_off: "🔕 Уведомления: ❌ выкл",
    btn_lang: "🌐 Язык: Русский",
    btn_enable: "✅ Включить",
    btn_disable: "❌ Выключить",
    btn_rename: "✏️ Переименовать",
    btn_delete: "🗑 Удалить",
    btn_to_list: "⬅️ К списку",
    btn_menu: "🏠 Меню",
    btn_main_menu: "🏠 Главное меню",
    btn_cancel: "↩️ Отмена",
    btn_done: "✅ Готово",
    welcome: "👋 Привет! Я слежу за игровыми лобби WarCraft 3 на wc3stats.\n\n\
              Добавь подписку на карту или хоста — и я пришлю уведомление, \
              когда появится новое лобби.\n\n\
              Названия сравниваются без учёта регистра и символов: \
              «CHS» совпадёт и с «CHS -», и с «something*chs*».",
    st_hdr: "📊 Статус:",
    st_notifications: "Уведомления",
    st_enabled: "✅ Включены",
    st_disabled: "❌ Выключены",
    st_total: "Всего карт",
    st_active_count: "Активных карт",
    st_no_active: "Активных карт нет",
    st_muted_hdr: "Отключённые карты",
    st_muted_empty: "Отключённых карт нет",
    st_forever: "навсегда",
    st_remaining: "осталось",
    st_h: "ч",
    st_m: "мин",
    maps_title: "🗺 Мои подписки ({n}):",
    maps_hint: "\n\nНажми на подписку, чтобы настроить её.",
    maps_empty: "🗺 У тебя пока нет подписок.\n\nДобавь карту или хоста через кнопки ниже.",
    sub_id: "Подписка #{id}",
    sub_name: "Название",
    sub_type: "Тип",
    sub_status: "Статус",
    sub_enabled: "✅ Включена",
    sub_disabled: "❌ Выключена",
    sub_desc_map: "Уведомления приходят, когда имя карты в новом лобби содержит это название (без учёта регистра и символов).",
    sub_desc_host: "Уведомления приходят, когда имя хоста в новом лобби содержит это название (без учёта регистра и символов).",
    sub_desc_name: "Уведомления приходят, когда название игры в новом лобби содержит это название (без учёта регистра и символов).",
    kind_map: "🗺 Карта",
    kind_host: "👤 Хост",
    kind_name: "📛 Название",
    prompt_add_map: "🗺 Отправь название карты для отслеживания.\n\nПримеры: pudge, chs, legion td",
    prompt_add_host: "👤 Отправь имя хоста для отслеживания.\n\nПримеры: HellWolf#31976, hellwolf\n\n⚠️ Подписки для хоста не поддерживают белый/чёрный список.",
    prompt_add_name: "📛 Отправь название игры для отслеживания.\n\nПримеры: dota, tavern, fun",
    prompt_add_all: "✨ Отправь название — я подпишу на карту, хоста и название игры сразу.\n\n⚠️ Общие подписки не поддерживают белый/чёрный список.",
    prompt_rename: "✏️ Текущее название: «{name}»\n\nОтправь новое название:",
    msg_cancelled: "↩️ Отменено.",
    msg_empty_name: "⚠️ Название не может быть пустым. Попробуй ещё раз:",
    msg_duplicate: "⚠️ Такая подписка уже существует.",
    msg_rename_fail: "⚠️ Не удалось переименовать. Попробуй ещё раз:",
    msg_added: "{icon} Подписка «{name}» добавлена!",
    msg_deleted: "🗑 Подписка «{name}» удалена.",
    msg_use_menu: "🤔 Используй кнопки меню или /start.",
    msg_stop: "👋 Все твои данные удалены. /start — начать заново.",
    msg_add_more: "Отправь ещё одно название или нажми Готово.",
    ping_map: "🗺️ Карта: {map}",
    ping_name: "📛 Название: {name} ({server})",
    ping_host: "🏠 Хост: {host}",
    ping_slots: "👥 Игроки: {taken}/{total}",
    ping_created: "⏱ Создано: {time} назад",
    ping_started: "✅ Началось: спустя {time}",
    dash: "—",
    msg_add_all_done: "✅ {name} — {list}\n\n{more}",
    btn_snooze: "😴 12ч",
    btn_mute: "🔕 Выкл",
    btn_check: "✅",
    msg_snoozed: "🔕 Уведомления выключены на 12 часов",
    msg_muted: "🔕 Уведомления выключены",
    btn_hosts: "⚙️ Хосты",
    hf_title: "⚙️ Фильтр хостов: {name}",
    hf_mode: "Режим",
    hf_mode_off: "Выкл",
    hf_mode_wl: "Белый список",
    hf_mode_bl: "Чёрный список",
    hf_hosts_hdr: "Хосты",
    hf_hosts_empty: "Хостов нет",
    btn_toggle_mode: "🔄 Сменить режим",
    btn_add_hf_host: "➕ Добавить хоста",
    prompt_add_hf_host: "👤 Отправь имя хоста для добавления в фильтр.\n\nПримеры: HellWolf#31976, hellwolf",
    msg_hf_host_added: "✅ Хост «{host}» добавлен.",
    msg_hf_host_deleted: "🗑 Хост «{host}» удалён.",
    hf_desc_wl: "Уведомлять только если хост в списке.",
    hf_desc_bl: "Пропускать игры от хостов из списка.",
};

pub fn tr(lang: Lang) -> &'static T {
    match lang {
        Lang::En => &EN,
        Lang::Ru => &RU,
    }
}

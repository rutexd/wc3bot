/// Парсинг «HH:MM» в минуты от полуночи. Возвращает `None` при ошибке.
pub fn parse_hhmm(s: &str) -> Option<i32> {
    let s = s.trim();
    let (h, m) = s.split_once(':')?;
    let h: i32 = h.parse().ok()?;
    let m: i32 = m.parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(h * 60 + m)
}

/// Парсинг смещения от UTC. Принимает формы:
/// - `UTC+3`, `UTC-5`, `UTC+5:30` (с префиксом UTC, любой регистр)
/// - `+3`, `-5`, `+5:30` (короткая форма)
/// - `5`, `5:30` (без знака = положительный)
/// - `0`, `UTC0`, `UTC` (= 0)
///
/// Пустая строка, только знаки (`+`, `-`), и любые невалидные сочетания
/// возвращают `None`.
pub fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let rest = s
        .strip_prefix("UTC")
        .or_else(|| s.strip_prefix("utc"))
        .unwrap_or(s);
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let (sign, num) = match rest.chars().next()? {
        '+' => (1i32, &rest[1..]),
        '-' => (-1i32, &rest[1..]),
        c if c.is_ascii_digit() => (1i32, rest),
        _ => return None,
    };
    if num.is_empty() {
        return None;
    }
    let (h_part, m_part) = match num.split_once(':') {
        Some((h, m)) => (h, m),
        None => (num, "0"),
    };
    if h_part.is_empty() || m_part.is_empty() {
        return None;
    }
    let h: i32 = h_part.parse().ok()?;
    let m: i32 = m_part.parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some(sign * (h * 60 + m))
}

/// Проверяет, попадает ли текущий момент времени в активное окно уведомлений
/// `[start_min, end_min)` (в минутах от полуночи в часовом поясе пользователя,
/// `tz_offset` в минутах).
///
/// - `start_min == end_min` — фича выключена, уведомления работают в любое время
///   (`true`).
/// - Иначе: `true`, когда `local_min` лежит в `[start, end)`.
/// - Поддерживает переход через полночь: например, 23:00–07:00 означает
///   «с 23:00 до 23:59 и с 00:00 до 07:00».
pub fn is_in_notification_window(now_ts: i64, tz_offset_min: i32, start_min: i32, end_min: i32) -> bool {
    if start_min == end_min {
        return true;
    }
    let local_min = local_minutes_of_day(now_ts, tz_offset_min);
    if start_min < end_min {
        local_min >= start_min && local_min < end_min
    } else {
        local_min >= start_min || local_min < end_min
    }
}

/// Переводит unix-таймстамп в «минуты от полуночи» в указанном часовом поясе.
pub fn local_minutes_of_day(unix_ts: i64, tz_offset_min: i32) -> i32 {
    const SECS_PER_DAY: i64 = 86_400;
    let shifted = unix_ts + (tz_offset_min as i64) * 60;
    let secs_in_day = shifted.rem_euclid(SECS_PER_DAY);
    ((secs_in_day / 3600) % 24 * 60 + (secs_in_day % 3600) / 60) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hhmm_basic() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_hhmm("9:30"), Some(9 * 60 + 30));
    }

    #[test]
    fn parse_hhmm_invalid() {
        assert!(parse_hhmm("").is_none());
        assert!(parse_hhmm("24:00").is_none());
        assert!(parse_hhmm("12:60").is_none());
        assert!(parse_hhmm("ab:cd").is_none());
        assert!(parse_hhmm("12").is_none());
    }

    #[test]
    fn parse_utc_offset_basic() {
        assert_eq!(parse_utc_offset("UTC0"), Some(0));
        assert_eq!(parse_utc_offset("UTC+3"), Some(180));
        assert_eq!(parse_utc_offset("UTC-5"), Some(-300));
        assert_eq!(parse_utc_offset("UTC+5:30"), Some(330));
        assert_eq!(parse_utc_offset("utc+3"), Some(180));
        // короткие формы без префикса UTC
        assert_eq!(parse_utc_offset("+3"), Some(180));
        assert_eq!(parse_utc_offset("-5"), Some(-300));
        assert_eq!(parse_utc_offset("+5:30"), Some(330));
        assert_eq!(parse_utc_offset("-5:30"), Some(-330));
        assert_eq!(parse_utc_offset("5"), Some(300));
        assert_eq!(parse_utc_offset("5:30"), Some(330));
        assert_eq!(parse_utc_offset("0"), Some(0));
        assert_eq!(parse_utc_offset("+0"), Some(0));
        assert_eq!(parse_utc_offset("-0"), Some(0));
        // пробелы вокруг
        assert_eq!(parse_utc_offset("  +3  "), Some(180));
        assert_eq!(parse_utc_offset("UTC +3"), Some(180));
        assert_eq!(parse_utc_offset("UTC  +3"), Some(180));
    }

    #[test]
    fn parse_utc_offset_invalid() {
        // пустые и пробельные
        assert!(parse_utc_offset("").is_none());
        assert!(parse_utc_offset("   ").is_none());
        // только префикс UTC без числа
        assert!(parse_utc_offset("UTC").is_none());
        assert!(parse_utc_offset("UTC  ").is_none());
        // только знак
        assert!(parse_utc_offset("+").is_none());
        assert!(parse_utc_offset("-").is_none());
        assert!(parse_utc_offset("UTC+").is_none());
        assert!(parse_utc_offset("UTC-").is_none());
        // пустые части вокруг `:`
        assert!(parse_utc_offset("UTC+:30").is_none());
        assert!(parse_utc_offset("UTC+3:").is_none());
        assert!(parse_utc_offset("+3:").is_none());
        // неверный префикс / мусор
        assert!(parse_utc_offset("GMT+3").is_none());
        assert!(parse_utc_offset("abc").is_none());
        // нечисловые компоненты
        assert!(parse_utc_offset("UTC+abc").is_none());
        assert!(parse_utc_offset("+abc").is_none());
        assert!(parse_utc_offset("UTC+3:abc").is_none());
        // выход за границы
        assert!(parse_utc_offset("UTC+24").is_none());
        assert!(parse_utc_offset("+24:00").is_none());
        assert!(parse_utc_offset("UTC+3:60").is_none());
    }

    /// 2024-01-15 12:00 UTC.
    const T_2024_01_15_12_00_UTC: i64 = 1_705_320_000;
    /// 2024-01-15 22:00 UTC.
    const T_2024_01_15_22_00_UTC: i64 = 1_705_356_000;
    // NB: для T_2024_01_15_22_00_UTC раньше я указывал 1_705_348_800 — это было
    // неправильное значение, соответствующее 19:46 UTC.

    #[test]
    fn notification_window_disabled_when_equal() {
        // start == end → фича выключена → уведомления работают в любое время.
        assert!(is_in_notification_window(T_2024_01_15_22_00_UTC, 0, 0, 0));
        assert!(is_in_notification_window(T_2024_01_15_22_00_UTC, 0, 60, 60));
    }

    #[test]
    fn notification_window_within_day() {
        // 09:00–18:00 UTC, сейчас 12:00 UTC → внутри окна.
        assert!(is_in_notification_window(T_2024_01_15_12_00_UTC, 0, 9 * 60, 18 * 60));
        // Сейчас 22:00 UTC → снаружи окна.
        assert!(!is_in_notification_window(T_2024_01_15_22_00_UTC, 0, 9 * 60, 18 * 60));
    }

    #[test]
    fn notification_window_cross_midnight() {
        // 23:00–07:00 UTC, сейчас 22:00 → снаружи.
        assert!(!is_in_notification_window(T_2024_01_15_22_00_UTC, 0, 23 * 60, 7 * 60));
        // А теперь 02:00 UTC → внутри.
        let t = T_2024_01_15_22_00_UTC + 4 * 3600;
        assert!(is_in_notification_window(t, 0, 23 * 60, 7 * 60));
        // А теперь 08:00 UTC → снаружи.
        let t = T_2024_01_15_22_00_UTC + 10 * 3600;
        assert!(!is_in_notification_window(t, 0, 23 * 60, 7 * 60));
        // Ровно 23:00 UTC → внутри (начало включается).
        assert!(is_in_notification_window(T_2024_01_15_22_00_UTC + 3600, 0, 23 * 60, 7 * 60));
        // Ровно 07:00 UTC → снаружи (конец не включается).
        assert!(!is_in_notification_window(T_2024_01_15_22_00_UTC + 9 * 3600, 0, 23 * 60, 7 * 60));
    }

    #[test]
    fn notification_window_with_timezone_offset() {
        // Окно 23:00–07:00 Europe/Moscow (UTC+3).
        // Сейчас 22:00 UTC = 01:00 MSK → внутри.
        assert!(is_in_notification_window(T_2024_01_15_22_00_UTC, 180, 23 * 60, 7 * 60));
        // А вот 17:00 UTC = 20:00 MSK → снаружи.
        let t = T_2024_01_15_22_00_UTC - 5 * 3600;
        assert!(!is_in_notification_window(t, 180, 23 * 60, 7 * 60));
    }

    #[test]
    fn notification_window_with_negative_offset() {
        // Окно 22:00–06:00 America/New_York (UTC-5).
        // Сейчас 03:00 UTC = 22:00 NY (предыдущего дня) → внутри.
        assert!(is_in_notification_window(T_2024_01_15_22_00_UTC + 5 * 3600, -300, 22 * 60, 6 * 60));
        // Сейчас 12:00 UTC = 07:00 NY → снаружи.
        assert!(!is_in_notification_window(T_2024_01_15_22_00_UTC + 14 * 3600, -300, 22 * 60, 6 * 60));
    }

    #[test]
    fn local_minutes_basic() {
        // 12:30 UTC
        let t = 12 * 3600 + 30 * 60;
        assert_eq!(local_minutes_of_day(t, 0), 12 * 60 + 30);
        // 23:00 UTC + UTC+3 = 02:00 следующего дня по местному
        assert_eq!(local_minutes_of_day(23 * 3600, 180), 2 * 60);
        // отрицательное смещение
        assert_eq!(local_minutes_of_day(3 * 3600, -300), 22 * 60);
    }

    #[test]
    fn local_minutes_extremes() {
        // граница полуночи
        assert_eq!(local_minutes_of_day(0, 0), 0);
        assert_eq!(local_minutes_of_day(0, 60), 60); // UTC+1: 00:00 UTC = 01:00 local
        assert_eq!(local_minutes_of_day(23 * 3600 + 59 * 60, 0), 23 * 60 + 59);
        // максимальное смещение +14 (Киритимати)
        assert_eq!(local_minutes_of_day(0, 14 * 60), 14 * 60);
        // максимальное смещение -12
        assert_eq!(local_minutes_of_day(0, -12 * 60), 12 * 60);
        // +12 UTC = -12 local
        assert_eq!(local_minutes_of_day(0, -12 * 60), 12 * 60);
        // +14 UTC vs -10 UTC на 12:00 UTC
        assert_eq!(local_minutes_of_day(12 * 3600, 14 * 60), (12 + 14) % 24 * 60);
        assert_eq!(local_minutes_of_day(12 * 3600, -10 * 60), (12 - 10 + 24) % 24 * 60);
    }

    #[test]
    fn local_minutes_rem_euclid_no_panic_on_negative_shift() {
        // важно: при экстремальных таймстампах rem_euclid не должен паниковать
        // например, начало эпохи + смещение -720 (UTC-12) → 12:00 предыдущего дня
        assert_eq!(local_minutes_of_day(0, -12 * 60), 12 * 60);
        // Очень большие таймстампы (далеко в будущем) с большим смещением
        let far_future = 365 * 24 * 3600 * 50i64; // ~50 лет
        let _ = local_minutes_of_day(far_future, 14 * 60);
        let _ = local_minutes_of_day(far_future, -12 * 60);
    }

#[test]
    fn is_in_notification_window_no_server_tz_dependency() {
        // Проверяем что результат детерминирован и не зависит от того,
        // где запущен код (UTC сервер или локальная TZ). Логика
        // is_in_notification_window получает now_ts явно, поэтому здесь просто
        // проверяем что разные «текущие» таймстампы дают ожидаемые результаты
        // для пользователя с UTC+3 и окна 23:00–07:00.

        // За час до окна (22:00 MSK = 19:00 UTC) → снаружи
        assert!(!is_in_notification_window(19 * 3600, 180, 23 * 60, 7 * 60));
        // В начале окна (23:00 MSK = 20:00 UTC) → внутри
        assert!(is_in_notification_window(20 * 3600, 180, 23 * 60, 7 * 60));
        // В конце окна (06:59 MSK = 03:59 UTC) → внутри
        assert!(is_in_notification_window(3 * 3600 + 59 * 60, 180, 23 * 60, 7 * 60));
        // Сразу после конца (07:00 MSK = 04:00 UTC) → снаружи
        assert!(!is_in_notification_window(4 * 3600, 180, 23 * 60, 7 * 60));
    }
}
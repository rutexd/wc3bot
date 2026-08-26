/// Нормализация: оставляем только буквы и цифры в нижнем регистре.
/// Поэтому «CHS -», «something*chs*» и «chs» дают «chs» / «somethingchs».
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Совпадение по подстроке: шаблон должен встречаться в нормализованном
/// тексте, т.е. вокруг него может быть любой другой текст.
pub fn matches(pattern: &str, text: &str) -> bool {
    let p = normalize(pattern);
    !p.is_empty() && normalize(text).contains(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chs_matches_anything_around_it() {
        assert!(matches("CHS -", "something*chs*"));
        assert!(matches("chs", "(10) CHS 5x5 - fast game"));
        assert!(!matches("", "anything"));
    }

    #[test]
    fn map_and_host_names() {
        assert!(matches("pudge", "(10) Pudge Wars v2.03d.w3x"));
        assert!(matches("legion td", "Legion_TD_11.4b_Team_OZE.w3x"));
        assert!(matches("hellwolf", "HellWolf#31976"));
        assert!(matches("HellWolf#31976", "HellWolf#31976"));
        assert!(!matches("dota", "Pudge Wars.w3x"));
    }
}

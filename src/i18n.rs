//! Russian or English, picked from the session locale (LC_ALL / LC_MESSAGES / LANG / LANGUAGE).

use std::sync::OnceLock;

/// True unless the locale is Russian, Ukrainian, Belarusian or Kazakh.
pub fn en() -> bool {
    static EN: OnceLock<bool> = OnceLock::new();
    *EN.get_or_init(|| {
        if let Ok(l) = std::env::var("WRAITH_LANG") { return !l.starts_with("ru"); }
        let lang = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"].iter()
            .filter_map(|k| std::env::var(k).ok()).find(|v| !v.trim().is_empty() && v != "C" && v != "POSIX")
            .unwrap_or_default().to_ascii_lowercase();
        !["ru", "uk", "be", "kk"].iter().any(|p| lang.starts_with(p))
    })
}
pub fn t(ru: &'static str, en_text: &'static str) -> &'static str { if en() { en_text } else { ru } }

#[macro_export]
macro_rules! tr { ($ru:expr, $en:expr) => { $crate::i18n::t($ru, $en) }; }
#[macro_export]
macro_rules! trf { ($ru:literal, $en:literal $(, $($a:tt)*)?) => { if $crate::i18n::en() { format!($en $(, $($a)*)?) } else { format!($ru $(, $($a)*)?) } }; }

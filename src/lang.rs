//! Localized error strings from `lang/en_us.json` and `lang/de_de.json`.
//!
//! Locale resolution: `COREIMAGE_LANG`, then `LANG` / `LC_ALL`
//! (prefix `de` selects German), otherwise English.

use std::collections::HashMap;
use std::sync::OnceLock;

const EN_US: &str = include_str!("../lang/en_us.json");
const DE_DE: &str = include_str!("../lang/de_de.json");

/// Supported language codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LangCode {
    EnUs,
    DeDe,
}

/// Detect the active language from the environment.
pub fn active_lang() -> LangCode {
    for key in ["COREIMAGE_LANG", "LANG", "LC_ALL"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.to_lowercase();
            if v.starts_with("de") {
                return LangCode::DeDe;
            }
            if v.starts_with("en") {
                return LangCode::EnUs;
            }
        }
    }
    LangCode::EnUs
}

fn table(lang: LangCode) -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static DE: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        LangCode::EnUs => EN.get_or_init(|| serde_json::from_str(EN_US).unwrap_or_default()),
        LangCode::DeDe => DE.get_or_init(|| serde_json::from_str(DE_DE).unwrap_or_default()),
    }
}

/// Translate `key` with English fallback to the key itself.
pub fn tr(key: &str) -> String {
    let lang = active_lang();
    if let Some(v) = table(lang).get(key) {
        return v.clone();
    }
    if lang != LangCode::EnUs {
        if let Some(v) = table(LangCode::EnUs).get(key) {
            return v.clone();
        }
    }
    key.to_string()
}

/// Translate `key` for an explicit language.
pub fn tr_in(lang: LangCode, key: &str) -> String {
    if let Some(v) = table(lang).get(key) {
        return v.clone();
    }
    table(LangCode::EnUs).get(key).cloned().unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tables_parse() {
        assert!(!table(LangCode::EnUs).is_empty());
        assert!(!table(LangCode::DeDe).is_empty());
    }

    #[test]
    fn fallback_returns_key() {
        assert_eq!(tr_in(LangCode::EnUs, "no_such_key_xyz"), "no_such_key_xyz");
    }
}

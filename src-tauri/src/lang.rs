//! 화면 언어. 한국어와 영어 두 가지라 문장마다 `l("한국어", "English")` 로 바로 고른다.
//! 설정값은 "system" | "ko" | "en". system 이면 OS 언어가 한국어일 때만 한국어.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

const UNSET: u8 = 0;
const KO: u8 = 1;
const EN: u8 = 2;

static CURRENT: AtomicU8 = AtomicU8::new(UNSET);
static OS_LOCALE: OnceLock<String> = OnceLock::new();

/// macOS 는 화면 언어와 LANG 이 다를 때가 많아서(en-US) 기본 언어 목록을 먼저 본다.
fn os_locale() -> &'static str {
    OS_LOCALE.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            if let Ok(out) = std::process::Command::new("defaults")
                .args(["read", "-g", "AppleLanguages"])
                .output()
            {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(first) = text
                    .split(|c: char| c == '"' || c == ',' || c.is_whitespace() || c == '(' || c == ')')
                    .find(|s| s.len() >= 2 && s.chars().take(2).all(|c| c.is_ascii_alphabetic()))
                {
                    return first.to_string();
                }
            }
        }
        sys_locale::get_locale().or_else(|| std::env::var("LANG").ok()).unwrap_or_default()
    })
}

pub fn system_lang(locale: Option<&str>) -> &'static str {
    let loc = locale.unwrap_or_else(|| os_locale());
    if loc.to_ascii_lowercase().starts_with("ko") { "ko" } else { "en" }
}

pub fn resolve_lang(setting: Option<&str>, locale: Option<&str>) -> &'static str {
    match setting {
        Some("ko") => "ko",
        Some("en") => "en",
        _ => system_lang(locale),
    }
}

pub fn set_lang(lang: &str) {
    CURRENT.store(if lang == "ko" { KO } else { EN }, Ordering::SeqCst);
}

pub fn get_lang() -> &'static str {
    match CURRENT.load(Ordering::SeqCst) {
        KO => "ko",
        EN => "en",
        _ => system_lang(None),
    }
}

/// 지금 언어에 맞는 문장
pub fn l(ko: &str, en: &str) -> String {
    if get_lang() == "ko" { ko.to_string() } else { en.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve() {
        assert_eq!(resolve_lang(Some("en"), Some("ko-KR")), "en");
        assert_eq!(resolve_lang(Some("ko"), Some("en-US")), "ko");
        assert_eq!(resolve_lang(Some("system"), Some("ko-KR")), "ko");
        assert_eq!(resolve_lang(Some("system"), Some("en-GB")), "en");
        assert_eq!(resolve_lang(None, Some("ja-JP")), "en");
    }
}

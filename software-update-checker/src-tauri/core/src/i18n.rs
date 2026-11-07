//! 表示言語の切り替え（日本語 / 英語）。
//!
//! 利用者に見えるメッセージ（警告・エラー・通信ログなど）は [`tr!`] で日本語と英語を並べて書き、
//! 実行時の表示言語で選ぶ。表示言語はフロントエンドの設定から [`set_language`] で受け取る。
//! 言語を足すときは [`Language`] に値を追加し、`tr!` の引数を増やす。

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Japanese = 0,
    English = 1,
}

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::Japanese as u8);

#[cfg(test)]
thread_local! {
    /// テストでは並行実行の影響を受けないよう、スレッドごとに言語を切り替える。
    static TEST_LANGUAGE: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

/// `ja` / `en`（`en-US` なども可）。それ以外は日本語。
pub fn set_language(code: &str) {
    let lang = if code.to_ascii_lowercase().starts_with("en") { Language::English } else { Language::Japanese };
    LANGUAGE.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Language {
    #[cfg(test)]
    if let Some(lang) = TEST_LANGUAGE.with(|l| l.get()) {
        return lang;
    }
    if LANGUAGE.load(Ordering::Relaxed) == Language::English as u8 {
        Language::English
    } else {
        Language::Japanese
    }
}

pub fn is_english() -> bool {
    current() == Language::English
}

#[cfg(test)]
pub fn with_language<T>(lang: Language, f: impl FnOnce() -> T) -> T {
    TEST_LANGUAGE.with(|l| l.set(Some(lang)));
    let result = f();
    TEST_LANGUAGE.with(|l| l.set(None));
    result
}

/// 表示言語に応じて日本語か英語の書式で文字列を作る。
/// `tr!("{name} を読めません", "Cannot read {name}")` のように、書式と引数は `format!` と同じ。
#[macro_export]
macro_rules! tr {
    ($ja:literal, $en:literal $(,)?) => {
        if $crate::i18n::is_english() { format!($en) } else { format!($ja) }
    };
    ($ja:literal, $en:literal, $($arg:tt)+) => {
        if $crate::i18n::is_english() { format!($en, $($arg)+) } else { format!($ja, $($arg)+) }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_language_with_captured_and_positional_args() {
        let name = "pom.xml";
        let ja = with_language(Language::Japanese, || tr!("{name} を読めません（{}）", "Cannot read {name} ({})", 1));
        let en = with_language(Language::English, || tr!("{name} を読めません（{}）", "Cannot read {name} ({})", 1));
        assert_eq!(ja, "pom.xml を読めません（1）");
        assert_eq!(en, "Cannot read pom.xml (1)");
    }
}

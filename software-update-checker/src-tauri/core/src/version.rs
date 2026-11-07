//! エコシステムをまたいで使う、ゆるいバージョン比較。
//!
//! Maven の ComparableVersion に近い規則で、数値トークンは数値として、
//! 修飾子（alpha / beta / rc / snapshot / sp など）は既知の順序で比較する。
//! npm（semver）や PyPI の一般的なバージョン表記もおおむね正しく並ぶ。

use crate::model::UpdateKind;
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Num(u64),
    Word(String),
}

fn tokenize(version: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_digit = false;
    let flush = |current: &mut String, is_digit: bool, tokens: &mut Vec<Token>| {
        if current.is_empty() {
            return;
        }
        if is_digit {
            tokens.push(Token::Num(current.parse().unwrap_or(u64::MAX)));
        } else {
            tokens.push(Token::Word(current.to_ascii_lowercase()));
        }
        current.clear();
    };
    for ch in version.trim().trim_start_matches(['v', 'V']).chars() {
        if ch == '.' || ch == '-' || ch == '_' || ch == '+' {
            flush(&mut current, current_is_digit, &mut tokens);
            continue;
        }
        let is_digit = ch.is_ascii_digit();
        if !current.is_empty() && is_digit != current_is_digit {
            flush(&mut current, current_is_digit, &mut tokens);
        }
        current_is_digit = is_digit;
        current.push(ch);
    }
    flush(&mut current, current_is_digit, &mut tokens);
    tokens
}

/// 修飾子の並び順。リリース版（修飾子なし）を 6 とする。
fn qualifier_rank(word: &str) -> Option<u8> {
    Some(match word {
        "alpha" | "a" | "dev" => 1,
        "beta" | "b" => 2,
        "milestone" | "m" | "ea" | "preview" | "pre" => 3,
        "rc" | "cr" => 4,
        "snapshot" => 5,
        "ga" | "final" | "release" => 6,
        "sp" => 7,
        _ => return None,
    })
}

const RELEASE_RANK: u8 = 6;

fn compare_word(a: &str, b: &str) -> Ordering {
    match (qualifier_rank(a), qualifier_rank(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        // 未知の修飾子は既知のものより後ろ（Maven と同じ扱い）
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

/// トークンが尽きた側と比べるときの、相手トークンの「重さ」。
fn compare_to_missing(token: &Token) -> Ordering {
    match token {
        Token::Num(0) => Ordering::Equal,
        Token::Num(_) => Ordering::Greater,
        Token::Word(w) => match qualifier_rank(w) {
            Some(r) => r.cmp(&RELEASE_RANK),
            None => Ordering::Greater,
        },
    }
}

pub fn compare(a: &str, b: &str) -> Ordering {
    let ta = tokenize(a);
    let tb = tokenize(b);
    let len = ta.len().max(tb.len());
    for i in 0..len {
        let ord = match (ta.get(i), tb.get(i)) {
            (Some(x), Some(y)) => match (x, y) {
                (Token::Num(m), Token::Num(n)) => m.cmp(n),
                (Token::Word(m), Token::Word(n)) => compare_word(m, n),
                // 数値は修飾子より新しい（1.0.1 > 1.0-rc1）
                (Token::Num(_), Token::Word(w)) => match qualifier_rank(w) {
                    Some(_) => Ordering::Greater,
                    None => Ordering::Less,
                },
                (Token::Word(w), Token::Num(_)) => match qualifier_rank(w) {
                    Some(_) => Ordering::Less,
                    None => Ordering::Greater,
                },
            },
            (Some(x), None) => compare_to_missing(x),
            (None, Some(y)) => compare_to_missing(y).reverse(),
            (None, None) => Ordering::Equal,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

/// プレリリース版か（alpha / beta / milestone / rc / snapshot / preview など）。
pub fn is_prerelease(version: &str) -> bool {
    tokenize(version).iter().any(|t| match t {
        Token::Word(w) => matches!(qualifier_rank(w), Some(r) if r < RELEASE_RANK) || w == "nightly" || w == "canary",
        Token::Num(_) => false,
    })
}

fn numeric_prefix(version: &str) -> Vec<u64> {
    tokenize(version)
        .into_iter()
        .map_while(|t| match t {
            Token::Num(n) => Some(n),
            Token::Word(_) => None,
        })
        .collect()
}

/// 先頭の数値（メジャーバージョン）。
pub fn major(version: &str) -> Option<u64> {
    numeric_prefix(version).first().copied()
}

/// 派生版を表す接尾辞（Guava の `-jre` / `-android`、byte-buddy の `-jdk5` など）。
/// alpha / rc / Final のような既知の修飾子は含めない。接尾辞が無ければ空文字。
pub fn variant(version: &str) -> String {
    tokenize(version)
        .into_iter()
        .filter_map(|t| match t {
            Token::Word(w) if qualifier_rank(&w).is_none() => Some(w),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// バージョン系列（`メジャー.マイナー`）。`1.3.16` → `1.3`、`8` → `8.0`。
pub fn series(version: &str) -> Option<String> {
    let n = numeric_prefix(version);
    let major = n.first()?;
    Some(format!("{major}.{}", n.get(1).copied().unwrap_or(0)))
}

/// `current` から `candidate` への更新の大きさ。
pub fn update_kind(current: &str, candidate: &str) -> UpdateKind {
    if compare(candidate, current) != Ordering::Greater {
        return UpdateKind::None;
    }
    let c = numeric_prefix(current);
    let n = numeric_prefix(candidate);
    if c.is_empty() || n.is_empty() {
        return UpdateKind::Unknown;
    }
    let at = |v: &Vec<u64>, i: usize| v.get(i).copied().unwrap_or(0);
    if at(&c, 0) != at(&n, 0) {
        UpdateKind::Major
    } else if at(&c, 1) != at(&n, 1) {
        UpdateKind::Minor
    } else {
        UpdateKind::Patch
    }
}

/// `YYYY-MM-DD...` 形式の先頭から、1970-01-01 からの日数を求める。
pub fn days_from_iso_date(s: &str) -> Option<i64> {
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Howard Hinnant の days_from_civil
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

pub fn today_days() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (secs / 86_400) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn numeric_ordering() {
        assert_eq!(compare("10.1.20", "10.1.9"), Greater);
        assert_eq!(compare("1.0", "1.0.0"), Equal);
        assert_eq!(compare("2.0.0", "10.0.0"), Less);
        assert_eq!(compare("v1.2.3", "1.2.3"), Equal);
    }

    #[test]
    fn qualifier_ordering() {
        assert_eq!(compare("10.0.0-M1", "10.0.0"), Less);
        assert_eq!(compare("1.0-alpha1", "1.0-beta1"), Less);
        assert_eq!(compare("1.0-rc1", "1.0"), Less);
        assert_eq!(compare("1.0-SNAPSHOT", "1.0"), Less);
        assert_eq!(compare("1.0.Final", "1.0"), Equal);
        assert_eq!(compare("1.0-sp1", "1.0"), Greater);
        assert_eq!(compare("1.0.1", "1.0-rc1"), Greater);
        assert_eq!(compare("10.0.0-M10", "10.0.0-M9"), Greater);
    }

    #[test]
    fn prerelease_detection() {
        assert!(is_prerelease("10.0.0-M1"));
        assert!(is_prerelease("2.0.0-rc.1"));
        assert!(is_prerelease("1.0-SNAPSHOT"));
        assert!(is_prerelease("5.0.0-beta"));
        assert!(!is_prerelease("31.1-jre"));
        assert!(!is_prerelease("5.6.Final"));
        assert!(!is_prerelease("2.17.0"));
    }

    #[test]
    fn update_kinds() {
        assert_eq!(update_kind("10.1.20", "10.1.60"), UpdateKind::Patch);
        assert_eq!(update_kind("10.1.20", "10.2.0"), UpdateKind::Minor);
        assert_eq!(update_kind("10.1.20", "11.0.0"), UpdateKind::Major);
        assert_eq!(update_kind("10.1.20", "10.1.20"), UpdateKind::None);
        assert_eq!(update_kind("10.1.20", "10.1.19"), UpdateKind::None);
    }

    #[test]
    fn variants() {
        assert_eq!(variant("1.18.14-jdk5"), "jdk");
        assert_eq!(variant("33.0-jre"), "jre");
        assert_eq!(variant("33.0-android"), "android");
        assert_eq!(variant("1.18.14"), "");
        assert_eq!(variant("3.29.0-GA"), "");
        assert_eq!(variant("2.0.0-rc1"), "");
    }

    #[test]
    fn series_names() {
        assert_eq!(series("1.3.16").as_deref(), Some("1.3"));
        assert_eq!(series("3.1.5.RELEASE").as_deref(), Some("3.1"));
        assert_eq!(series("8").as_deref(), Some("8.0"));
        assert_eq!(series("10.0.0-M1").as_deref(), Some("10.0"));
        assert_eq!(series("[4.13.2,)"), None);
    }

    #[test]
    fn date_to_days() {
        assert_eq!(days_from_iso_date("1970-01-01"), Some(0));
        assert_eq!(days_from_iso_date("2024-03-19T13:27:51Z"), Some(19_801));
        assert_eq!(days_from_iso_date("bad"), None);
    }
}

//! CVSS v3.x のベーススコア計算。
//! 仕様: https://www.first.org/cvss/v3.1/specification-document
//!
//! OSV の `severity` にはベクター文字列だけが入っているため、
//! 数値スコアはここで計算する（v4.0 は計算式が大きく異なるため未対応）。

use crate::model::Severity;
use std::collections::HashMap;

pub fn base_score_v3(vector: &str) -> Option<f64> {
    if !vector.starts_with("CVSS:3.") {
        return None;
    }
    let metrics: HashMap<&str, &str> = vector.split('/').skip(1).filter_map(|m| m.split_once(':')).collect();
    let scope_changed = match *metrics.get("S")? {
        "U" => false,
        "C" => true,
        _ => return None,
    };
    let av = match *metrics.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.2,
        _ => return None,
    };
    let ac = match *metrics.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let pr = match (*metrics.get("PR")?, scope_changed) {
        ("N", _) => 0.85,
        ("L", false) => 0.62,
        ("L", true) => 0.68,
        ("H", false) => 0.27,
        ("H", true) => 0.5,
        _ => return None,
    };
    let ui = match *metrics.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let cia = |key: &str| -> Option<f64> {
        Some(match *metrics.get(key)? {
            "H" => 0.56,
            "L" => 0.22,
            "N" => 0.0,
            _ => return None,
        })
    };
    let (c, i, a) = (cia("C")?, cia("I")?, cia("A")?);

    let iss = 1.0 - (1.0 - c) * (1.0 - i) * (1.0 - a);
    let impact = if scope_changed {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02f64).powi(15)
    } else {
        6.42 * iss
    };
    let exploitability = 8.22 * av * ac * pr * ui;
    if impact <= 0.0 {
        return Some(0.0);
    }
    let raw = if scope_changed {
        (1.08 * (impact + exploitability)).min(10.0)
    } else {
        (impact + exploitability).min(10.0)
    };
    Some(round_up(raw))
}

/// CVSS v3.1 の Roundup（浮動小数点誤差を避ける定義どおりの実装）。
fn round_up(value: f64) -> f64 {
    let int_input = (value * 100_000.0).round() as i64;
    if int_input % 10_000 == 0 {
        int_input as f64 / 100_000.0
    } else {
        ((int_input / 10_000) + 1) as f64 / 10.0
    }
}

pub fn severity_from_score(score: f64) -> Severity {
    match score {
        s if s >= 9.0 => Severity::Critical,
        s if s >= 7.0 => Severity::High,
        s if s >= 4.0 => Severity::Medium,
        s if s > 0.0 => Severity::Low,
        _ => Severity::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_scores() {
        // Log4Shell (CVE-2021-44228)
        assert_eq!(base_score_v3("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"), Some(10.0));
        assert_eq!(base_score_v3("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"), Some(9.8));
        assert_eq!(base_score_v3("CVSS:3.1/AV:N/AC:H/PR:N/UI:R/S:U/C:L/I:N/A:N"), Some(3.1));
        assert_eq!(base_score_v3("CVSS:3.0/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:H"), Some(7.5));
        assert_eq!(base_score_v3("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:N/I:N/A:N"), Some(0.0));
        assert_eq!(base_score_v3("CVSS:4.0/AV:N"), None);
    }

    #[test]
    fn severity_bands() {
        assert_eq!(severity_from_score(9.8), Severity::Critical);
        assert_eq!(severity_from_score(7.5), Severity::High);
        assert_eq!(severity_from_score(5.3), Severity::Medium);
        assert_eq!(severity_from_score(3.1), Severity::Low);
    }
}

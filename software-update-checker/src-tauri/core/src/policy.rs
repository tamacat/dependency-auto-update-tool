//! CI 向けの判定方針。チェック結果のうち、どれを「失敗」とするかを決める。
//!
//! 方針は `--fail-on malicious,kev,critical` のような規則の並びで指定する。
//! 規則のどれかに当たる指摘が 1 つでもあれば失敗（終了コード 1）。
//! 当たらない指摘も報告には含める（失敗にしないだけ）。

use crate::model::{CheckResult, Component, EolStatus, Severity, VulnInfo};
use std::collections::{HashMap, HashSet};

/// 失敗とする規則。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// 悪意あるパッケージ（OSV の MAL-）
    Malicious,
    /// CISA KEV に載っている脆弱性
    Kev,
    /// この深刻度以上の脆弱性（`Unknown` なら深刻度を問わずすべて）
    Severity(Severity),
    /// サポート終了
    Eol,
    /// 非推奨
    Deprecated,
}

impl Rule {
    /// `malicious` / `kev` / `critical` / `high` / `medium` / `low` / `any` / `eol` / `deprecated`
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "malicious" => Self::Malicious,
            "kev" => Self::Kev,
            "critical" => Self::Severity(Severity::Critical),
            "high" => Self::Severity(Severity::High),
            "medium" | "moderate" => Self::Severity(Severity::Medium),
            "low" => Self::Severity(Severity::Low),
            "any" => Self::Severity(Severity::Unknown),
            "eol" => Self::Eol,
            "deprecated" => Self::Deprecated,
            _ => return None,
        })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Malicious => "malicious",
            Self::Kev => "kev",
            Self::Severity(Severity::Critical) => "critical",
            Self::Severity(Severity::High) => "high",
            Self::Severity(Severity::Medium) => "medium",
            Self::Severity(Severity::Low) => "low",
            Self::Severity(Severity::Unknown) => "any",
            Self::Eol => "eol",
            Self::Deprecated => "deprecated",
        }
    }
}

/// 既定の方針: 悪意あるパッケージ・悪用確認済み・Critical があれば失敗。
pub const DEFAULT_FAIL_ON: &str = "malicious,kev,critical";

#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub fail_on: Vec<Rule>,
    /// 無視する脆弱性の ID（GHSA・CVE など。別名でも一致）
    pub ignore: HashSet<String>,
    /// 直接依存だけを判定する（推移的依存と分かっているものは除く）
    pub direct_only: bool,
}

impl Policy {
    /// `malicious,kev,critical` のような並びから作る。読めない規則があれば、その名前を返す。
    pub fn parse_rules(list: &str) -> Result<Vec<Rule>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "none")
            .map(|s| Rule::parse(s).ok_or_else(|| s.to_string()))
            .collect()
    }

    fn rules_for_vuln(&self, v: &VulnInfo) -> Vec<Rule> {
        self.fail_on
            .iter()
            .copied()
            .filter(|r| match r {
                Rule::Malicious => v.malicious,
                Rule::Kev => v.kev.is_some(),
                Rule::Severity(Severity::Unknown) => true,
                Rule::Severity(s) => v.severity >= *s,
                Rule::Eol | Rule::Deprecated => false,
            })
            .collect()
    }

    fn ignored(&self, v: &VulnInfo) -> bool {
        self.ignore.contains(&v.id) || v.aliases.iter().any(|a| self.ignore.contains(a))
    }
}

/// 指摘の種類。
#[derive(Clone, Debug)]
pub enum FindingKind {
    Vulnerability(Box<VulnInfo>),
    Eol { product: String, cycle: Option<String>, eol_from: Option<String> },
    Deprecated { reason: String },
}

/// 1 件の指摘。
#[derive(Clone, Debug)]
pub struct Finding {
    pub component_id: String,
    pub kind: FindingKind,
    /// 当たった規則。空なら失敗にはしない
    pub matched: Vec<Rule>,
}

impl Finding {
    pub fn failing(&self) -> bool {
        !self.matched.is_empty()
    }
}

/// チェック結果を方針で判定する。指摘は失敗するものから順に並べる。
pub fn evaluate(components: &[Component], results: &[CheckResult], policy: &Policy) -> Vec<Finding> {
    let by_id: HashMap<&str, &Component> = components.iter().map(|c| (c.id.as_str(), c)).collect();
    let mut findings = Vec::new();
    for r in results {
        let Some(c) = by_id.get(r.component_id.as_str()) else { continue };
        if policy.direct_only && c.direct == Some(false) {
            continue;
        }
        for v in r.vulnerabilities.iter().filter(|v| !policy.ignored(v)) {
            findings.push(Finding {
                component_id: c.id.clone(),
                matched: policy.rules_for_vuln(v),
                kind: FindingKind::Vulnerability(Box::new(v.clone())),
            });
        }
        if let Some(e) = r.eol.as_ref().filter(|e| e.status == EolStatus::Eol) {
            findings.push(Finding {
                component_id: c.id.clone(),
                matched: policy.fail_on.iter().copied().filter(|r| *r == Rule::Eol).collect(),
                kind: FindingKind::Eol { product: e.product_label.clone(), cycle: e.cycle.clone(), eol_from: e.eol_from.clone() },
            });
        }
        if let Some(reason) = r.version.as_ref().and_then(|v| v.deprecated.clone()) {
            findings.push(Finding {
                component_id: c.id.clone(),
                matched: policy.fail_on.iter().copied().filter(|r| *r == Rule::Deprecated).collect(),
                kind: FindingKind::Deprecated { reason },
            });
        }
    }
    // 失敗するもの → 深刻なもの の順
    let rank = |f: &Finding| match &f.kind {
        FindingKind::Vulnerability(v) => (v.malicious as u8 * 2 + v.kev.is_some() as u8, v.severity as u8 + 2),
        FindingKind::Eol { .. } => (0, 1),
        FindingKind::Deprecated { .. } => (0, 0),
    };
    findings.sort_by(|a, b| b.failing().cmp(&a.failing()).then_with(|| rank(b).cmp(&rank(a))));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Ecosystem, EolInfo, EolMatch, KevInfo};

    fn vuln(id: &str, severity: Severity) -> VulnInfo {
        VulnInfo {
            id: id.into(),
            aliases: vec![format!("CVE-{id}")],
            summary: None,
            severity,
            score: None,
            fixed_versions: vec![],
            published: None,
            url: String::new(),
            malicious: false,
            kev: None,
            epss: None,
        }
    }

    fn setup() -> (Vec<Component>, Vec<CheckResult>) {
        let mut a = Component::new("a", Ecosystem::Npm, "a");
        a.direct = Some(true);
        let mut b = Component::new("b", Ecosystem::Npm, "b");
        b.direct = Some(false);
        let mut kev = vuln("K", Severity::Medium);
        kev.kev = Some(KevInfo { cve: "CVE-K".into(), date_added: None, due_date: None, ransomware: false, required_action: None });
        let ra = CheckResult {
            component_id: "a".into(),
            vulnerabilities: vec![vuln("H", Severity::High), kev],
            eol: Some(EolInfo {
                product: "p".into(),
                product_label: "P".into(),
                cycle: Some("1".into()),
                status: EolStatus::Eol,
                eol_from: Some("2020-01-01".into()),
                latest_in_cycle: None,
                match_kind: EolMatch::Exact,
                link: String::new(),
                cycles: vec![],
            }),
            ..Default::default()
        };
        let rb = CheckResult { component_id: "b".into(), vulnerabilities: vec![vuln("C", Severity::Critical)], ..Default::default() };
        (vec![a, b], vec![ra, rb])
    }

    #[test]
    fn parses_rules() {
        assert_eq!(Policy::parse_rules(DEFAULT_FAIL_ON).unwrap(), vec![Rule::Malicious, Rule::Kev, Rule::Severity(Severity::Critical)]);
        assert_eq!(Policy::parse_rules("none").unwrap(), vec![]);
        assert_eq!(Policy::parse_rules("high, eol").unwrap(), vec![Rule::Severity(Severity::High), Rule::Eol]);
        assert_eq!(Policy::parse_rules("kev,severe"), Err("severe".into()));
        for r in ["malicious", "kev", "critical", "high", "medium", "low", "any", "eol", "deprecated"] {
            assert_eq!(Rule::parse(r).unwrap().name(), r);
        }
    }

    #[test]
    fn evaluates_default_policy() {
        let (c, r) = setup();
        let policy = Policy { fail_on: Policy::parse_rules(DEFAULT_FAIL_ON).unwrap(), ..Default::default() };
        let f = evaluate(&c, &r, &policy);
        // 失敗: b の Critical と a の KEV。失敗しない: a の High と EOL
        let failing: Vec<_> = f.iter().filter(|f| f.failing()).map(|f| f.component_id.as_str()).collect();
        assert_eq!(failing.len(), 2);
        assert!(f[0].failing() && f[1].failing() && !f[2].failing());
        assert_eq!(f.len(), 4);
        // KEV が Critical より先
        assert!(matches!(&f[0].kind, FindingKind::Vulnerability(v) if v.id == "K"));
    }

    #[test]
    fn applies_ignore_direct_only_and_eol() {
        let (c, r) = setup();
        let policy = Policy {
            fail_on: Policy::parse_rules("high,eol").unwrap(),
            ignore: ["CVE-H".to_string()].into_iter().collect(),
            direct_only: true,
        };
        let f = evaluate(&c, &r, &policy);
        // b（推移的依存）と、別名で無視した H は除かれる
        assert!(f.iter().all(|f| f.component_id == "a"));
        assert!(!f.iter().any(|f| matches!(&f.kind, FindingKind::Vulnerability(v) if v.id == "H")));
        assert!(f.iter().any(|f| matches!(f.kind, FindingKind::Eol { .. }) && f.matched == vec![Rule::Eol]));
        // KEV は Medium なので high では失敗しない
        assert!(f.iter().any(|f| matches!(&f.kind, FindingKind::Vulnerability(v) if v.id == "K") && !f.failing()));
    }
}

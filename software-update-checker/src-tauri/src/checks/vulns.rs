//! 既知の脆弱性のチェック（OSV.dev API）。
//! https://google.github.io/osv.dev/api/
//!
//! まず querybatch で各パッケージの脆弱性 ID だけを一括取得し、
//! 重複を除いた ID ごとに詳細を取得する。
//! 影響を受ける版の範囲から、すべてを直せる最小の更新先（[`FixAdvice`]）も求める。

use super::cvss;
use super::{canonical_purl, has_concrete_version, ttl, ProgressFn};
use crate::http::Http;
use crate::model::{CheckOptions, CheckProgress, CheckResult, Component, FixAdvice, Severity, VersionInfo, VulnInfo};
use crate::version;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const QUERY_BATCH_URL: &str = "https://api.osv.dev/v1/querybatch";
const VULN_URL: &str = "https://api.osv.dev/v1/vulns/";
const BATCH_SIZE: usize = 500;

#[derive(Deserialize)]
struct BatchResponse {
    #[serde(default)]
    results: Vec<BatchResult>,
}

#[derive(Deserialize, Default)]
struct BatchResult {
    #[serde(default)]
    vulns: Vec<VulnRef>,
}

#[derive(Deserialize)]
struct VulnRef {
    id: String,
}

#[derive(Deserialize)]
struct OsvVuln {
    id: String,
    summary: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    published: Option<String>,
    #[serde(default)]
    severity: Vec<OsvSeverity>,
    #[serde(default)]
    affected: Vec<OsvAffected>,
    database_specific: Option<Value>,
}

#[derive(Deserialize)]
struct OsvSeverity {
    #[serde(rename = "type")]
    kind: String,
    score: String,
}

#[derive(Deserialize)]
struct OsvAffected {
    package: Option<OsvPackage>,
    #[serde(default)]
    ranges: Vec<OsvRange>,
    /// 影響を受ける版の列挙
    #[serde(default)]
    versions: Vec<String>,
}

#[derive(Deserialize)]
struct OsvPackage {
    name: Option<String>,
    purl: Option<String>,
}

#[derive(Deserialize)]
struct OsvRange {
    /// `ECOSYSTEM` / `SEMVER` / `GIT`
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    events: Vec<Value>,
}

/// 悪意あるパッケージの報告（OpenSSF Malicious Packages）の ID か。
fn is_malicious_id(id: &str) -> bool {
    id.starts_with("MAL-")
}

/// 1 件の脆弱性について、このパッケージのどの版が影響を受けるか。
#[derive(Default, Debug)]
struct AffectedSpec {
    /// (introduced, 終端, 終端を含むか)。終端が `None` なら上限なし。
    intervals: Vec<(String, Option<String>, bool)>,
    /// 列挙された影響版
    versions: Vec<String>,
}

impl AffectedSpec {
    fn from_affected(entries: &[&OsvAffected]) -> Self {
        let mut spec = Self::default();
        for a in entries {
            spec.versions.extend(a.versions.iter().cloned());
            for r in a.ranges.iter().filter(|r| r.kind == "ECOSYSTEM" || r.kind == "SEMVER") {
                // (種類, 版)。introduced の "0" は最小として並べる
                let mut events: Vec<(&str, &str)> = r
                    .events
                    .iter()
                    .filter_map(|e| e.as_object())
                    .flat_map(|o| o.iter())
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.as_str(), v)))
                    .collect();
                events.sort_by(|a, b| match (a.1, b.1) {
                    ("0", "0") => Ordering::Equal,
                    ("0", _) => Ordering::Less,
                    (_, "0") => Ordering::Greater,
                    (x, y) => version::compare(x, y),
                });
                let mut start: Option<String> = None;
                for (kind, v) in events {
                    match kind {
                        "introduced" => start = start.or(Some(v.to_string())),
                        "fixed" | "last_affected" => {
                            if let Some(s) = start.take() {
                                spec.intervals.push((s, Some(v.to_string()), kind == "last_affected"));
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(s) = start {
                    spec.intervals.push((s, None, false));
                }
            }
        }
        spec
    }

    /// 範囲や列挙から判定できるか（GIT の範囲しか無いものは判定できない）。
    fn is_known(&self) -> bool {
        !self.intervals.is_empty() || !self.versions.is_empty()
    }

    fn affects(&self, v: &str) -> bool {
        self.versions.iter().any(|x| version::compare(x, v) == Ordering::Equal)
            || self.intervals.iter().any(|(start, end, inclusive)| {
                let after_start = start == "0" || version::compare(v, start) != Ordering::Less;
                let before_end = match end {
                    None => true,
                    Some(e) => match version::compare(v, e) {
                        Ordering::Less => true,
                        Ordering::Equal => *inclusive,
                        Ordering::Greater => false,
                    },
                };
                after_start && before_end
            })
    }
}

/// すべてを直せる最小の更新先を求める（ネットワークを使わない純粋関数）。
///
/// 候補は、各脆弱性の修正版と、deps.dev で分かった系列ごとの最新版など。
/// 現行より新しく、現行と同じ派生版（`-jre` など）のものだけを比べる。
fn fix_advice(
    current: &str,
    vulns: &[(&VulnInfo, &AffectedSpec)],
    version_info: Option<&VersionInfo>,
    include_prerelease: bool,
) -> FixAdvice {
    let mut candidates: Vec<String> = vulns.iter().flat_map(|(v, _)| v.fixed_versions.iter().cloned()).collect();
    if let Some(info) = version_info {
        candidates.extend(
            [&info.latest_in_minor, &info.latest_in_major, &info.latest].into_iter().flatten().cloned(),
        );
        candidates.extend(info.series.iter().map(|s| s.latest.clone()));
    }
    let allow_prerelease = include_prerelease || version::is_prerelease(current);
    candidates.retain(|c| {
        version::compare(c, current) == Ordering::Greater && (allow_prerelease || !version::is_prerelease(c))
    });
    let wanted = version::variant(current);
    if candidates.iter().any(|c| version::variant(c) == wanted) {
        candidates.retain(|c| version::variant(c) == wanted);
    }
    candidates.sort_by(|a, b| version::compare(a, b));
    candidates.dedup_by(|a, b| version::compare(a, b) == Ordering::Equal);

    let (fixable, unfixable): (Vec<_>, Vec<_>) = vulns
        .iter()
        .partition(|(_, spec)| spec.is_known() && candidates.iter().any(|c| !spec.affects(c)));
    if fixable.is_empty() {
        return FixAdvice { unfixed: unfixable.iter().map(|(v, _)| v.id.clone()).collect(), ..Default::default() };
    }
    let fixes_all = |c: &&String| fixable.iter().all(|(_, spec)| !spec.affects(c));
    let current_series = version::series(current);
    let current_major = version::major(current);
    FixAdvice {
        in_series: candidates
            .iter()
            .filter(|c| current_series.is_some() && version::series(c) == current_series)
            .find(fixes_all)
            .cloned(),
        in_major: candidates
            .iter()
            .filter(|c| current_major.is_some() && version::major(c) == current_major)
            .find(fixes_all)
            .cloned(),
        minimum: candidates.iter().find(fixes_all).cloned(),
        unfixed: unfixable.iter().map(|(v, _)| v.id.clone()).collect(),
    }
}

/// OSV の脆弱性 1 件を、特定パッケージ向けの [`VulnInfo`] と影響範囲に変換する。
fn to_vuln_info(v: &OsvVuln, package_name: &str, base_purl: &str) -> (VulnInfo, AffectedSpec) {
    let score = v
        .severity
        .iter()
        .filter(|s| s.kind == "CVSS_V3")
        .find_map(|s| cvss::base_score_v3(&s.score));

    let label = v
        .database_specific
        .as_ref()
        .and_then(|d| d.get("severity"))
        .and_then(Value::as_str)
        .map(|s| match s.to_ascii_uppercase().as_str() {
            "CRITICAL" => Severity::Critical,
            "HIGH" => Severity::High,
            "MODERATE" | "MEDIUM" => Severity::Medium,
            "LOW" => Severity::Low,
            _ => Severity::Unknown,
        });
    let malicious = is_malicious_id(&v.id) || v.aliases.iter().any(|a| is_malicious_id(a));
    let severity = match (label, score) {
        // 悪意あるパッケージは、入っているだけで最も深刻
        _ if malicious => Severity::Critical,
        (Some(l), _) if l != Severity::Unknown => l,
        (_, Some(s)) => cvss::severity_from_score(s),
        _ => Severity::Unknown,
    };

    // このパッケージに該当する affected エントリから修正版を集める
    let matches = |a: &&OsvAffected| {
        a.package.as_ref().is_some_and(|p| {
            p.purl.as_deref().map(|u| u.split('@').next() == Some(base_purl)).unwrap_or(false)
                || p.name.as_deref() == Some(package_name)
        })
    };
    let relevant: Vec<&OsvAffected> = v.affected.iter().filter(matches).collect();
    let source: Vec<&OsvAffected> = if relevant.is_empty() { v.affected.iter().collect() } else { relevant };
    let mut fixed_versions: Vec<String> = source
        .iter()
        .flat_map(|a| a.ranges.iter())
        .flat_map(|r| r.events.iter())
        .filter_map(|e| e.get("fixed").and_then(Value::as_str).map(str::to_string))
        .collect();
    fixed_versions.sort_by(|a, b| version::compare(a, b));
    fixed_versions.dedup();

    let mut aliases = v.aliases.clone();
    aliases.sort_by_key(|a| !a.starts_with("CVE-"));

    let info = VulnInfo {
        id: v.id.clone(),
        aliases,
        summary: v.summary.clone(),
        severity,
        score,
        fixed_versions,
        published: v.published.clone(),
        url: format!("https://osv.dev/vulnerability/{}", v.id),
        malicious,
        kev: None,
        epss: None,
    };
    (info, AffectedSpec::from_affected(&source))
}

/// 一覧の並び順: 悪意あるパッケージ → 悪用確認済み（KEV）→ 深刻度 → EPSS → ID。
pub(super) fn sort_vulns(list: &mut [VulnInfo]) {
    let epss = |v: &VulnInfo| v.epss.as_ref().map_or(0.0, |e| e.score);
    list.sort_by(|a, b| {
        b.malicious
            .cmp(&a.malicious)
            .then_with(|| b.kev.is_some().cmp(&a.kev.is_some()))
            .then_with(|| b.severity.cmp(&a.severity))
            .then_with(|| epss(b).total_cmp(&epss(a)))
            .then_with(|| a.id.cmp(&b.id))
    });
}

pub async fn check(
    http: &Http,
    targets: &[&Component],
    opts: &CheckOptions,
    results: &mut HashMap<String, CheckResult>,
    progress: ProgressFn,
) {
    // purl@version ごとにまとめる
    let mut by_purl: HashMap<String, Vec<&Component>> = HashMap::new();
    for c in targets.iter().filter(|c| has_concrete_version(c)) {
        if let Some(p) = canonical_purl(c, true) {
            by_purl.entry(p).or_default().push(c);
        }
    }
    let purls: Vec<String> = by_purl.keys().cloned().collect();

    // 1. querybatch で ID を取得
    let mut ids_by_purl: HashMap<String, Vec<String>> = HashMap::new();
    let batches: Vec<&[String]> = purls.chunks(BATCH_SIZE).collect();
    for (i, chunk) in batches.iter().enumerate() {
        progress(CheckProgress { phase: "vulnerabilities".into(), done: i, total: batches.len() });
        let body = json!({ "queries": chunk.iter().map(|p| json!({ "package": { "purl": p } })).collect::<Vec<_>>() });
        let summary = tr!("{} 件の purl を照会", "Query for {} purls", chunk.len());
        let parsed = match http.post_json(QUERY_BATCH_URL, &body, ttl(opts, 6), &summary).await {
            Ok(res) if res.is_success() => serde_json::from_str::<BatchResponse>(&res.body).map_err(|e| e.to_string()),
            Ok(res) => Err(format!("HTTP {}: {}", res.status, res.body.chars().take(200).collect::<String>())),
            Err(e) => Err(e),
        };
        match parsed {
            Ok(batch) => {
                for (purl, r) in chunk.iter().zip(batch.results) {
                    ids_by_purl.insert(purl.clone(), r.vulns.into_iter().map(|v| v.id).collect());
                }
            }
            Err(e) => {
                for purl in chunk.iter() {
                    for c in &by_purl[purl] {
                        if let Some(r) = results.get_mut(&c.id) {
                            r.errors.push(tr!("脆弱性情報の取得に失敗: {e}", "Failed to fetch vulnerability information: {e}"));
                        }
                    }
                }
            }
        }
    }

    // 2. 脆弱性の詳細を ID ごとに取得
    let unique_ids: BTreeSet<String> = ids_by_purl.values().flatten().cloned().collect();
    let total = unique_ids.len();
    let semaphore = Arc::new(Semaphore::new(http.network().max_concurrency));
    let mut tasks = JoinSet::new();
    for id in unique_ids {
        let (http, opts, semaphore) = (http.clone(), opts.clone(), semaphore.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            let res = http.get(&format!("{VULN_URL}{id}"), ttl(&opts, 24)).await;
            let parsed = match res {
                Ok(r) if r.is_success() => serde_json::from_str::<OsvVuln>(&r.body).ok(),
                _ => None,
            };
            (id, parsed)
        });
    }
    let mut details: HashMap<String, OsvVuln> = HashMap::new();
    let mut done = 0;
    while let Some(joined) = tasks.join_next().await {
        done += 1;
        progress(CheckProgress { phase: "vulnerabilityDetails".into(), done, total });
        if let Ok((id, Some(v))) = joined {
            details.insert(id, v);
        }
    }

    // 3. コンポーネントごとに組み立て
    for (purl, ids) in ids_by_purl {
        let base_purl = purl.rsplit_once('@').map(|(b, _)| b).unwrap_or(&purl).to_string();
        for c in &by_purl[&purl] {
            let Some(r) = results.get_mut(&c.id) else { continue };
            let package_name = c.package_name();
            let converted: Vec<(VulnInfo, AffectedSpec)> = ids
                .iter()
                .map(|id| match details.get(id) {
                    Some(v) => to_vuln_info(v, &package_name, &base_purl),
                    None => (
                        VulnInfo {
                            id: id.clone(),
                            aliases: Vec::new(),
                            summary: Some(tr!("（詳細を取得できませんでした）", "(Could not fetch the details)")),
                            severity: if is_malicious_id(id) { Severity::Critical } else { Severity::Unknown },
                            score: None,
                            fixed_versions: Vec::new(),
                            published: None,
                            url: format!("https://osv.dev/vulnerability/{id}"),
                            malicious: is_malicious_id(id),
                            kev: None,
                            epss: None,
                        },
                        AffectedSpec::default(),
                    ),
                })
                .collect();
            if let (false, Some(current)) = (converted.is_empty(), c.version.as_deref()) {
                let pairs: Vec<(&VulnInfo, &AffectedSpec)> = converted.iter().map(|(v, s)| (v, s)).collect();
                r.fix = Some(fix_advice(current, &pairs, r.version.as_ref(), opts.include_prerelease));
            }
            let mut infos: Vec<VulnInfo> = converted.into_iter().map(|(v, _)| v).collect();
            sort_vulns(&mut infos);
            r.vulnerabilities = infos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_osv_record() {
        let v: OsvVuln = serde_json::from_value(json!({
            "id": "GHSA-xxxx",
            "summary": "Session fixation",
            "aliases": ["BIT-1", "CVE-2025-55668"],
            "database_specific": { "severity": "MODERATE" },
            "severity": [ { "type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:H/PR:N/UI:R/S:U/C:L/I:N/A:N" } ],
            "affected": [
              { "package": { "name": "org.apache.tomcat:tomcat-catalina", "purl": "pkg:maven/org.apache.tomcat/tomcat-catalina" },
                "ranges": [ { "type": "ECOSYSTEM", "events": [ { "introduced": "0" }, { "fixed": "9.0.106" } ] } ] },
              { "package": { "name": "org.apache.tomcat.embed:tomcat-embed-core", "purl": "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core" },
                "ranges": [ { "type": "ECOSYSTEM", "events": [ { "introduced": "10.1.0-M1" }, { "fixed": "10.1.42" } ] },
                            { "type": "ECOSYSTEM", "events": [ { "introduced": "11.0.0-M1" }, { "fixed": "11.0.8" } ] } ] }
            ]
        }))
        .unwrap();
        let (info, spec) = to_vuln_info(
            &v,
            "org.apache.tomcat.embed:tomcat-embed-core",
            "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core",
        );
        assert_eq!(info.severity, Severity::Medium);
        assert_eq!(info.score, Some(3.1));
        assert_eq!(info.aliases[0], "CVE-2025-55668");
        assert_eq!(info.fixed_versions, vec!["10.1.42", "11.0.8"]);
        assert!(!info.malicious);
        // tomcat-catalina 側の範囲（〜9.0.106）は含めない
        assert!(!spec.affects("9.0.50"));
        assert!(spec.affects("10.1.41"));
        assert!(!spec.affects("10.1.42"));
        assert!(spec.affects("11.0.0-M1"));
        assert!(!spec.affects("11.0.8"));
    }

    fn vuln(id: &str, fixed: &[&str]) -> VulnInfo {
        VulnInfo {
            id: id.into(),
            aliases: Vec::new(),
            summary: None,
            severity: Severity::High,
            score: None,
            fixed_versions: fixed.iter().map(|s| s.to_string()).collect(),
            published: None,
            url: String::new(),
            malicious: false,
            kev: None,
            epss: None,
        }
    }

    fn spec(intervals: &[(&str, Option<&str>, bool)]) -> AffectedSpec {
        AffectedSpec {
            intervals: intervals.iter().map(|(s, e, i)| (s.to_string(), e.map(str::to_string), *i)).collect(),
            versions: Vec::new(),
        }
    }

    #[test]
    fn handles_last_affected_and_open_ranges() {
        let v: OsvAffected = serde_json::from_value(json!({
            "ranges": [ { "type": "ECOSYSTEM", "events": [ { "introduced": "2.0" }, { "last_affected": "2.3" } ] },
                        { "type": "GIT", "events": [ { "introduced": "abc" } ] },
                        { "type": "SEMVER", "events": [ { "introduced": "3.1.0" } ] } ],
            "versions": [ "1.5" ]
        }))
        .unwrap();
        let s = AffectedSpec::from_affected(&[&v]);
        assert!(s.affects("1.5"));
        assert!(!s.affects("1.6"));
        assert!(s.affects("2.3"));
        assert!(!s.affects("2.4"));
        assert!(s.affects("3.9.9"));
    }

    #[test]
    fn advises_smallest_version_fixing_everything() {
        // 10.1 系は 10.1.42 と 10.1.45 で別々に修正、11.0 系は 11.0.8 で両方修正
        let a = vuln("A", &["10.1.42", "11.0.8"]);
        let b = vuln("B", &["10.1.45", "11.0.8"]);
        let sa = spec(&[("10.1.0", Some("10.1.42"), false), ("11.0.0", Some("11.0.8"), false)]);
        let sb = spec(&[("10.1.0", Some("10.1.45"), false), ("11.0.0", Some("11.0.8"), false)]);
        let none = vuln("C", &[]);
        let snone = spec(&[("0", None, false)]);
        let advice = fix_advice("10.1.40", &[(&a, &sa), (&b, &sb), (&none, &snone)], None, false);
        assert_eq!(advice.in_series.as_deref(), Some("10.1.45"));
        assert_eq!(advice.minimum.as_deref(), Some("10.1.45"));
        assert_eq!(advice.unfixed, vec!["C"]);

        // 同じ系列に修正版が無い場合
        let advice = fix_advice("9.0.1", &[(&a, &sa)], None, false);
        assert_eq!(advice.in_series, None);
        assert_eq!(advice.in_major, None);
        assert_eq!(advice.minimum.as_deref(), Some("10.1.42"));

        // 同じメジャーの別系列で直る場合（npm の 4.17 → 4.20 など）
        let advice = fix_advice("10.0.5", &[(&a, &sa)], None, false);
        assert_eq!(advice.in_series, None);
        assert_eq!(advice.in_major.as_deref(), Some("10.1.42"));

        // 直せるものが無い場合
        let advice = fix_advice("1.0", &[(&none, &snone)], None, false);
        assert_eq!(advice, FixAdvice { unfixed: vec!["C".into()], ..Default::default() });
    }

    #[test]
    fn detects_malicious_packages() {
        let v: OsvVuln = serde_json::from_value(json!({
            "id": "MAL-2025-1234",
            "summary": "Malicious code in evil-pkg (npm)",
            "affected": [ { "package": { "name": "evil-pkg" }, "versions": ["1.0.0"] } ]
        }))
        .unwrap();
        let (info, _) = to_vuln_info(&v, "evil-pkg", "pkg:npm/evil-pkg");
        assert!(info.malicious);
        assert_eq!(info.severity, Severity::Critical);
    }

    #[test]
    fn sorts_malicious_then_kev_then_severity() {
        let mut list = vec![vuln("GHSA-b", &[]), vuln("GHSA-a", &[]), vuln("MAL-1", &[])];
        list[0].kev = Some(crate::model::KevInfo {
            cve: "CVE-1".into(),
            date_added: None,
            due_date: None,
            ransomware: false,
            required_action: None,
        });
        list[2].malicious = true;
        list[2].severity = Severity::Low;
        sort_vulns(&mut list);
        assert_eq!(list.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["MAL-1", "GHSA-b", "GHSA-a"]);
    }
}

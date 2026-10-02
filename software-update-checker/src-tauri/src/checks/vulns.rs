//! 既知の脆弱性のチェック（OSV.dev API）。
//! https://google.github.io/osv.dev/api/
//!
//! まず querybatch で各パッケージの脆弱性 ID だけを一括取得し、
//! 重複を除いた ID ごとに詳細を取得する。

use super::cvss;
use super::{canonical_purl, has_concrete_version, ttl, ProgressFn};
use crate::http::Http;
use crate::model::{CheckOptions, CheckProgress, CheckResult, Component, Severity, VulnInfo};
use crate::version;
use serde::Deserialize;
use serde_json::{json, Value};
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
}

#[derive(Deserialize)]
struct OsvPackage {
    name: Option<String>,
    purl: Option<String>,
}

#[derive(Deserialize)]
struct OsvRange {
    #[serde(default)]
    events: Vec<Value>,
}

/// OSV の脆弱性 1 件を、特定パッケージ向けの [`VulnInfo`] に変換する。
fn to_vuln_info(v: &OsvVuln, package_name: &str, base_purl: &str) -> VulnInfo {
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
    let severity = match (label, score) {
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
    let source = if relevant.is_empty() { v.affected.iter().collect() } else { relevant };
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

    VulnInfo {
        id: v.id.clone(),
        aliases,
        summary: v.summary.clone(),
        severity,
        score,
        fixed_versions,
        published: v.published.clone(),
        url: format!("https://osv.dev/vulnerability/{}", v.id),
    }
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
            let mut infos: Vec<VulnInfo> = ids
                .iter()
                .map(|id| match details.get(id) {
                    Some(v) => to_vuln_info(v, &package_name, &base_purl),
                    None => VulnInfo {
                        id: id.clone(),
                        aliases: Vec::new(),
                        summary: Some(tr!("（詳細を取得できませんでした）", "(Could not fetch the details)")),
                        severity: Severity::Unknown,
                        score: None,
                        fixed_versions: Vec::new(),
                        published: None,
                        url: format!("https://osv.dev/vulnerability/{id}"),
                    },
                })
                .collect();
            infos.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id)));
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
        let info = to_vuln_info(
            &v,
            "org.apache.tomcat.embed:tomcat-embed-core",
            "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core",
        );
        assert_eq!(info.severity, Severity::Medium);
        assert_eq!(info.score, Some(3.1));
        assert_eq!(info.aliases[0], "CVE-2025-55668");
        assert_eq!(info.fixed_versions, vec!["10.1.42", "11.0.8"]);
    }
}

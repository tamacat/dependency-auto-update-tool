//! サプライチェーンの情報（deps.dev API v3 のバージョン詳細とプロジェクト情報）。
//! https://docs.deps.dev/api/v3/
//!
//! - バージョン詳細（GetVersion）: ライセンス、来歴（SLSA provenance・attestation）、ソースリポジトリ
//! - プロジェクト情報（GetProject）: OpenSSF Scorecard
//!
//! `CheckOptions::supply_chain` が false のときは、ライセンスが入力に無い部品の
//! バージョン詳細だけを取ってライセンスを補完する（以前からの動作）。

use super::versions::url;
use super::{has_concrete_version, ttl, ProgressFn};
use crate::http::Http;
use crate::model::{
    CheckOptions, CheckProgress, CheckResult, Component, ProvenanceInfo, ScorecardCheck, ScorecardInfo,
    SupplyChainInfo,
};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct VersionDetail {
    #[serde(default)]
    licenses: Vec<String>,
    #[serde(default)]
    slsa_provenances: Vec<SlsaProvenance>,
    #[serde(default)]
    attestations: Vec<Attestation>,
    #[serde(default)]
    related_projects: Vec<RelatedProject>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlsaProvenance {
    source_repository: Option<String>,
    commit: Option<String>,
    url: Option<String>,
    #[serde(default)]
    verified: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attestation {
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
    #[serde(default)]
    verified: bool,
    source_repository: Option<String>,
    commit: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelatedProject {
    project_key: ProjectKey,
    relation_provenance: Option<String>,
    relation_type: Option<String>,
}

#[derive(Deserialize)]
struct ProjectKey {
    id: String,
}

#[derive(Deserialize)]
struct ProjectResponse {
    scorecard: Option<RawScorecard>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawScorecard {
    date: Option<String>,
    overall_score: Option<f64>,
    #[serde(default)]
    checks: Vec<RawCheck>,
}

#[derive(Deserialize)]
struct RawCheck {
    name: String,
    score: Option<i32>,
    reason: Option<String>,
}

/// バージョン詳細から、Scorecard 以外のサプライチェーン情報を組み立てる。
fn summarize(detail: &VersionDetail) -> SupplyChainInfo {
    // 来歴で裏付けられた対応（SLSA_ATTESTATION など）を、自己申告のメタデータより優先する
    let repos: Vec<&RelatedProject> =
        detail.related_projects.iter().filter(|p| p.relation_type.as_deref() == Some("SOURCE_REPO")).collect();
    let is_verified = |p: &&&RelatedProject| p.relation_provenance.as_deref().is_some_and(|s| s != "UNVERIFIED_METADATA");
    let (source_repo, source_repo_verified) = match repos.iter().find(is_verified) {
        Some(p) => (Some(p.project_key.id.clone()), true),
        None => (repos.first().map(|p| p.project_key.id.clone()), false),
    };

    // attestation と slsaProvenances は同じものを指すことが多いので、URL で重複を除く
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut provenance = Vec::new();
    for a in &detail.attestations {
        if a.url.as_ref().is_none_or(|u| seen.insert(u.clone())) {
            provenance.push(ProvenanceInfo {
                kind: a.kind.clone(),
                verified: a.verified,
                source_repository: a.source_repository.clone(),
                commit: a.commit.clone(),
            });
        }
    }
    for p in &detail.slsa_provenances {
        if p.url.as_ref().is_none_or(|u| seen.insert(u.clone())) {
            provenance.push(ProvenanceInfo {
                kind: None,
                verified: p.verified,
                source_repository: p.source_repository.clone(),
                commit: p.commit.clone(),
            });
        }
    }

    SupplyChainInfo { source_repo, source_repo_verified, scorecard: None, provenance }
}

fn to_scorecard(raw: RawScorecard) -> Option<ScorecardInfo> {
    Some(ScorecardInfo {
        overall: raw.overall_score?,
        date: raw.date,
        checks: raw
            .checks
            .into_iter()
            .map(|c| ScorecardCheck { name: c.name, score: c.score.filter(|s| *s >= 0), reason: c.reason })
            .collect(),
    })
}

async fn fetch_version(http: &Http, system: &str, name: &str, ver: &str, opts: &CheckOptions) -> Result<VersionDetail, String> {
    let res = http.get(&url(&["v3", "systems", system, "packages", name, "versions", ver]), ttl(opts, 24 * 7)).await?;
    match res.status {
        404 => Ok(VersionDetail::default()),
        _ if res.is_success() => serde_json::from_str(&res.body).map_err(|e| e.to_string()),
        s => Err(format!("deps.dev: HTTP {s}")),
    }
}

async fn fetch_scorecard(http: &Http, project: &str, opts: &CheckOptions) -> Result<Option<ScorecardInfo>, String> {
    let res = http.get(&url(&["v3", "projects", project]), ttl(opts, 24 * 7)).await?;
    match res.status {
        404 => Ok(None),
        _ if res.is_success() => serde_json::from_str::<ProjectResponse>(&res.body)
            .map(|p| p.scorecard.and_then(to_scorecard))
            .map_err(|e| e.to_string()),
        s => Err(format!("deps.dev: HTTP {s}")),
    }
}

pub async fn check(
    http: &Http,
    targets: &[&Component],
    opts: &CheckOptions,
    results: &mut HashMap<String, CheckResult>,
    progress: ProgressFn,
) {
    // (system, name, version) → コンポーネント ID
    let mut lookups: HashMap<(&'static str, String, String), Vec<String>> = HashMap::new();
    for c in targets {
        let (Some(system), true) = (c.ecosystem.deps_dev_system(), has_concrete_version(c)) else { continue };
        let Some(r) = results.get(&c.id) else { continue };
        // レジストリに無い版（社内ライブラリなど）は照会しない
        if r.version.as_ref().is_none_or(|v| v.current_not_found) {
            continue;
        }
        if !opts.supply_chain && !c.licenses.is_empty() {
            continue;
        }
        let ver = c.version.clone().unwrap_or_default();
        lookups.entry((system, c.package_name(), ver)).or_default().push(c.id.clone());
    }
    let needs_license: BTreeSet<&str> =
        targets.iter().filter(|c| c.licenses.is_empty()).map(|c| c.id.as_str()).collect();

    let semaphore = Arc::new(Semaphore::new(http.network().max_concurrency));
    let total = lookups.len();
    let mut tasks = JoinSet::new();
    for ((system, name, ver), ids) in lookups {
        let (http, opts, semaphore) = (http.clone(), opts.clone(), semaphore.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            (ids, fetch_version(&http, system, &name, &ver, &opts).await)
        });
    }
    let mut projects: BTreeSet<String> = BTreeSet::new();
    let mut done = 0;
    while let Some(joined) = tasks.join_next().await {
        done += 1;
        progress(CheckProgress { phase: "versionDetails".into(), done, total });
        let Ok((ids, res)) = joined else { continue };
        for id in ids {
            let Some(r) = results.get_mut(&id) else { continue };
            match &res {
                Ok(detail) => {
                    if needs_license.contains(id.as_str()) {
                        r.licenses = detail.licenses.clone();
                    }
                    if opts.supply_chain {
                        let info = summarize(detail);
                        projects.extend(info.source_repo.clone());
                        r.supply_chain = Some(info);
                    }
                }
                Err(e) if opts.supply_chain => r.errors.push(tr!(
                    "サプライチェーン情報の取得に失敗: {e}",
                    "Failed to fetch supply-chain information: {e}"
                )),
                Err(_) => {}
            }
        }
    }
    if !opts.supply_chain || projects.is_empty() {
        return;
    }

    // プロジェクト（ソースリポジトリ）ごとに Scorecard を取る
    let total = projects.len();
    let mut tasks = JoinSet::new();
    for project in projects {
        let (http, opts, semaphore) = (http.clone(), opts.clone(), semaphore.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            let res = fetch_scorecard(&http, &project, &opts).await;
            (project, res)
        });
    }
    let mut scorecards: HashMap<String, ScorecardInfo> = HashMap::new();
    let mut done = 0;
    while let Some(joined) = tasks.join_next().await {
        done += 1;
        progress(CheckProgress { phase: "scorecard".into(), done, total });
        match joined {
            Ok((project, Ok(Some(card)))) => {
                scorecards.insert(project, card);
            }
            Ok((project, Err(e))) => http.log().info(tr!(
                "Scorecard を取得できません（{project}）: {e}",
                "Cannot fetch the Scorecard ({project}): {e}"
            )),
            _ => {}
        }
    }
    for r in results.values_mut() {
        if let Some(info) = r.supply_chain.as_mut() {
            info.scorecard = info.source_repo.as_ref().and_then(|p| scorecards.get(p)).cloned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_verified_source_repo_and_dedupes_provenance() {
        let detail: VersionDetail = serde_json::from_str(
            r#"{"licenses":["Apache-2.0"],
                "slsaProvenances":[{"sourceRepository":"https://github.com/sigstore/sigstore-js","commit":"06cd","url":"https://registry.npmjs.org/-/npm/v1/attestations/sigstore@3.1.0","verified":true}],
                "attestations":[{"type":"https://slsa.dev/provenance/v1","url":"https://registry.npmjs.org/-/npm/v1/attestations/sigstore@3.1.0","verified":true,"sourceRepository":"https://github.com/sigstore/sigstore-js","commit":"06cd"}],
                "relatedProjects":[
                  {"projectKey":{"id":"github.com/fork/sigstore-js"},"relationProvenance":"UNVERIFIED_METADATA","relationType":"SOURCE_REPO"},
                  {"projectKey":{"id":"github.com/sigstore/sigstore-js"},"relationProvenance":"SLSA_ATTESTATION","relationType":"SOURCE_REPO"},
                  {"projectKey":{"id":"github.com/other/issues"},"relationProvenance":"UNVERIFIED_METADATA","relationType":"ISSUE_TRACKER"}]}"#,
        )
        .unwrap();
        let info = summarize(&detail);
        assert_eq!(info.source_repo.as_deref(), Some("github.com/sigstore/sigstore-js"));
        assert!(info.source_repo_verified);
        assert_eq!(info.provenance.len(), 1);
        assert_eq!(info.provenance[0].kind.as_deref(), Some("https://slsa.dev/provenance/v1"));
        assert!(info.provenance[0].verified);
    }

    #[test]
    fn unverified_repo_only() {
        let detail: VersionDetail = serde_json::from_str(
            r#"{"relatedProjects":[{"projectKey":{"id":"github.com/expressjs/express"},"relationProvenance":"UNVERIFIED_METADATA","relationType":"SOURCE_REPO"}]}"#,
        )
        .unwrap();
        let info = summarize(&detail);
        assert_eq!(info.source_repo.as_deref(), Some("github.com/expressjs/express"));
        assert!(!info.source_repo_verified);
        assert!(info.provenance.is_empty());
    }

    #[test]
    fn converts_scorecard() {
        let p: ProjectResponse = serde_json::from_str(
            r#"{"scorecard":{"date":"2026-08-24T00:00:00Z","overallScore":7.4,"checks":[
                {"name":"Maintained","score":10,"reason":"active"},
                {"name":"Packaging","score":-1,"reason":"packaging workflow not detected"}]}}"#,
        )
        .unwrap();
        let card = to_scorecard(p.scorecard.unwrap()).unwrap();
        assert_eq!(card.overall, 7.4);
        assert_eq!(card.checks[0].score, Some(10));
        assert_eq!(card.checks[1].score, None);
    }
}

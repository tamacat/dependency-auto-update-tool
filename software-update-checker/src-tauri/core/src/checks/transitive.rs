//! 推移的依存の脆弱性を、どの直接依存をどの版に上げれば解消できるかを調べる。
//!
//! 1. 取り込んだ依存関係のグラフから、直接依存 → … → 対象 の経路を探す（通信しない）。
//! 2. 経路の起点の直接依存ごとに、deps.dev が解決した依存関係のグラフ（GetDependencies）で、
//!    新しい版がどの版の対象を引き込むかを見る。引き込まなくなるか、引き込む版が OSV で
//!    今の脆弱性に当たらなければ「解消」とする。
//! 3. 通信を抑えるため、まず系列ごとの最新版を古い順に調べ、解消する最初の系列の中で
//!    二分探索して最小の版を求める（同じ系列の中では、新しい版ほど直っている前提）。
//!
//! deps.dev の解決結果は、プロジェクトの実際の解決（Maven の dependencyManagement、
//! npm の重複排除など）と違うことがあるため、結果は「推定」として示す。
//! 詳細画面のボタンから個別に呼ぶ（通信が多いため、チェック実行では行わない）。

use super::versions::{fetch_package, url};
use super::{canonical_purl, has_concrete_version, ttl};
use crate::http::Http;
use crate::model::{CheckOptions, Component, RouteFix, TransitiveFix};
use crate::sources;
use crate::version;
use serde::Deserialize;
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// 表示する経路の最大数
const MAX_PATHS: usize = 10;
/// 調べる直接依存の最大数
const MAX_ROUTES: usize = 5;
/// 1 つの直接依存について調べる版の最大数（通信の上限）
const MAX_PROBES: usize = 20;
const OSV_QUERY_BATCH: &str = "https://api.osv.dev/v1/querybatch";

#[derive(Deserialize)]
struct DependencyGraph {
    #[serde(default)]
    nodes: Vec<GraphNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphNode {
    version_key: GraphKey,
}

#[derive(Deserialize)]
struct GraphKey {
    name: String,
    version: String,
}

#[derive(Deserialize)]
struct OsvBatch {
    #[serde(default)]
    results: Vec<OsvResult>,
}

#[derive(Deserialize, Default)]
struct OsvResult {
    #[serde(default)]
    vulns: Vec<OsvId>,
}

#[derive(Deserialize)]
struct OsvId {
    id: String,
}

/// 直接依存から対象までの経路（部品 ID の列。先頭が直接依存、末尾が対象）。
///
/// 直接依存の情報が無い入力では、親を持たない部品を起点とみなす。
pub(crate) fn find_paths(components: &[Component], target: &str) -> Vec<Vec<String>> {
    let mut parents: HashMap<&str, Vec<&str>> = HashMap::new();
    for c in components {
        for d in &c.dependencies {
            parents.entry(d.as_str()).or_default().push(c.id.as_str());
        }
    }
    let by_id: HashMap<&str, &Component> = components.iter().map(|c| (c.id.as_str(), c)).collect();
    let has_direct_info = components.iter().any(|c| c.direct.is_some());
    let is_start = |id: &str| match by_id.get(id) {
        Some(c) if has_direct_info => c.direct == Some(true),
        _ => !parents.contains_key(id),
    };

    // 対象から親へ向かって幅優先で辿る（短い経路から見つかる）
    let mut paths = Vec::new();
    let mut queue: VecDeque<Vec<&str>> = VecDeque::from([vec![target]]);
    while let Some(path) = queue.pop_front() {
        if paths.len() >= MAX_PATHS {
            break;
        }
        let head = path[path.len() - 1];
        if head != target && is_start(head) {
            paths.push(path.iter().rev().map(|s| s.to_string()).collect());
            continue;
        }
        for p in parents.get(head).into_iter().flatten() {
            // 循環を避ける
            if !path.contains(p) {
                let mut next = path.clone();
                next.push(p);
                queue.push_back(next);
            }
        }
    }
    paths
}

/// 依存を固定する方法の案内（エコシステムごと）。
fn pin_hint(target: &Component, fix: Option<&str>) -> Option<String> {
    let name = target.package_name();
    let version = fix.unwrap_or("…");
    Some(match target.ecosystem.purl_type()? {
        "maven" => tr!(
            "直接依存を上げずに、pom.xml の dependencyManagement で {name} を {version} 以上に固定する方法もあります。",
            "Instead of upgrading a direct dependency, you can pin {name} to {version} or later in the dependencyManagement section of pom.xml."
        ),
        "npm" => tr!(
            "直接依存を上げずに、package.json の overrides で {name} を {version} 以上に固定する方法もあります。",
            "Instead of upgrading a direct dependency, you can pin {name} to {version} or later with \"overrides\" in package.json."
        ),
        "pypi" => tr!(
            "直接依存を上げずに、constraints ファイル（pip install -c）や依存の指定で {name}>={version} を指定する方法もあります。",
            "Instead of upgrading a direct dependency, you can require {name}>={version} with a constraints file (pip install -c) or in your requirements."
        ),
        _ => return None,
    })
}

/// 調べる順番: 系列ごとの最新版（古い系列から）。
fn series_order(candidates: &[String]) -> Vec<(String, Vec<String>)> {
    let mut by_series: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for v in candidates {
        by_series.entry(version::series(v).unwrap_or_else(|| v.clone())).or_default().push(v.clone());
    }
    let mut list: Vec<(String, Vec<String>)> = by_series.into_iter().collect();
    list.sort_by(|a, b| version::compare(&a.0, &b.0));
    for (_, versions) in &mut list {
        versions.sort_by(|a, b| version::compare(a, b));
    }
    list
}

/// 「直接依存をその版にすると解消するか」を答えるもの（`None` は調べられなかった）。
trait Prober {
    async fn probe(&mut self, version: &str) -> Option<bool>;
}

/// 系列の最新版で解消する最初の系列を探し、その系列の中で最小の版を二分探索する。
/// 戻り値は（解消する最小の版、調べた回数）。
async fn search<P: Prober>(candidates: &[String], prober: &mut P) -> (Option<String>, usize) {
    let mut probes = 0;
    for (_, versions) in series_order(candidates) {
        if probes >= MAX_PROBES {
            break;
        }
        probes += 1;
        if prober.probe(&versions[versions.len() - 1]).await != Some(true) {
            continue;
        }
        // versions[hi] は解消する。versions[..lo] は解消しないと分かっている
        let (mut lo, mut hi) = (0, versions.len() - 1);
        while lo < hi && probes < MAX_PROBES {
            let mid = (lo + hi) / 2;
            probes += 1;
            if prober.probe(&versions[mid]).await == Some(true) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        return (Some(versions[hi].clone()), probes);
    }
    (None, probes)
}

/// deps.dev と OSV に問い合わせて答える [`Prober`]。
struct LiveProber<'a> {
    http: &'a Http,
    opts: &'a CheckOptions,
    system: &'static str,
    direct_name: String,
    target: &'a Component,
    target_name: String,
    ids: &'a HashSet<String>,
    /// 直接依存の版 → 引き込む対象の版（引き込まなければ None）
    found: HashMap<String, Option<String>>,
    last_error: Option<String>,
}

impl Prober for LiveProber<'_> {
    async fn probe(&mut self, version: &str) -> Option<bool> {
        let resolved =
            match resolved_version(self.http, self.opts, self.system, &self.direct_name, version, &self.target_name).await {
                Ok(r) => r,
                Err(e) => {
                    self.last_error = Some(e);
                    return None;
                }
            };
        self.found.insert(version.to_string(), resolved.clone());
        // 引き込まなくなれば解消
        let Some(tv) = resolved else { return Some(true) };
        let mut probe = self.target.clone();
        probe.version = Some(tv);
        let purl = canonical_purl(&probe, true)?;
        match still_vulnerable(self.http, self.opts, &purl, self.ids).await {
            Ok(vulnerable) => Some(!vulnerable),
            Err(e) => {
                self.last_error = Some(e);
                None
            }
        }
    }
}

/// 直接依存のある版が引き込む対象の版。引き込まなければ `Ok(None)`。
async fn resolved_version(http: &Http, opts: &CheckOptions, system: &str, direct: &str, ver: &str, target: &str) -> Result<Option<String>, String> {
    let u = format!("{}:dependencies", url(&["v3", "systems", system, "packages", direct, "versions", ver]));
    let res = http.get(&u, ttl(opts, 24 * 7)).await?;
    if !res.is_success() {
        return Err(format!("deps.dev: HTTP {}", res.status));
    }
    let graph: DependencyGraph = serde_json::from_str(&res.body).map_err(|e| e.to_string())?;
    Ok(graph.nodes.into_iter().find(|n| n.version_key.name == target).map(|n| n.version_key.version))
}

/// 対象のある版に、今の脆弱性のどれかが残っているか。
async fn still_vulnerable(http: &Http, opts: &CheckOptions, purl: &str, ids: &HashSet<String>) -> Result<bool, String> {
    let body = json!({ "queries": [ { "package": { "purl": purl } } ] });
    let res = http.post_json(OSV_QUERY_BATCH, &body, ttl(opts, 6), &tr!("1 件の purl を照会", "Query for 1 purl")).await?;
    if !res.is_success() {
        return Err(format!("OSV: HTTP {}", res.status));
    }
    let batch: OsvBatch = serde_json::from_str(&res.body).map_err(|e| e.to_string())?;
    Ok(batch.results.into_iter().next().unwrap_or_default().vulns.iter().any(|v| ids.contains(&v.id)))
}

/// 推移的依存 `target` の脆弱性（`vuln_ids`）を、どの直接依存をどの版に上げれば解消できるか。
pub async fn find_fix(
    http: &Http,
    target: &Component,
    components: &[Component],
    vuln_ids: &[String],
    fix_hint: Option<&str>,
    opts: &CheckOptions,
) -> TransitiveFix {
    let paths = find_paths(components, &target.id);
    let mut result = TransitiveFix { paths: paths.clone(), routes: Vec::new(), pin_hint: pin_hint(target, fix_hint) };
    if !opts.source_enabled(sources::DEPS_DEV) || !opts.source_enabled(sources::OSV) {
        return result;
    }
    let by_id: HashMap<&str, &Component> = components.iter().map(|c| (c.id.as_str(), c)).collect();
    let target_name = target.package_name();
    let ids: HashSet<String> = vuln_ids.iter().cloned().collect();

    // 経路の起点（直接依存）ごとに 1 回だけ調べる
    let mut seen = HashSet::new();
    let starts: Vec<&Component> = paths
        .iter()
        .filter_map(|p| p.first())
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .take(MAX_ROUTES)
        .collect();

    for direct in starts {
        let mut route = RouteFix {
            direct_id: direct.id.clone(),
            current: direct.version.clone(),
            fix_version: None,
            target_version: None,
            probes: 0,
            error: None,
        };
        let (Some(system), true) = (direct.ecosystem.deps_dev_system(), has_concrete_version(direct)) else {
            route.error = Some(tr!("この部品は調べられません（版が不明か、未対応のエコシステム）", "Cannot look this component up (unknown version or unsupported ecosystem)"));
            result.routes.push(route);
            continue;
        };
        let direct_name = direct.package_name();
        let current = direct.version.clone().unwrap_or_default();

        // 現行より新しい版（プレリリースを除き、同じ派生版だけ）
        let candidates: Vec<String> = match fetch_package(http, system, &direct_name, opts).await {
            Ok(Some(pkg)) => {
                let wanted = version::variant(&current);
                let mut list: Vec<String> = pkg
                    .versions
                    .into_iter()
                    .map(|v| v.version_key.version)
                    .filter(|v| version::compare(v, &current) == Ordering::Greater)
                    .filter(|v| opts.include_prerelease || !version::is_prerelease(v))
                    .collect();
                if list.iter().any(|v| version::variant(v) == wanted) {
                    list.retain(|v| version::variant(v) == wanted);
                }
                list
            }
            Ok(None) => Vec::new(),
            Err(e) => {
                route.error = Some(e);
                result.routes.push(route);
                continue;
            }
        };

        let mut prober = LiveProber {
            http,
            opts,
            system,
            direct_name,
            target,
            target_name: target_name.clone(),
            ids: &ids,
            found: HashMap::new(),
            last_error: None,
        };
        let (fix, probes) = search(&candidates, &mut prober).await;
        route.probes = probes;
        route.target_version = fix.as_ref().and_then(|v| prober.found.get(v).cloned().flatten());
        route.fix_version = fix;
        if route.fix_version.is_none() {
            route.error = prober.last_error;
        }
        result.routes.push(route);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ecosystem;

    fn comp(id: &str, direct: Option<bool>, deps: &[&str]) -> Component {
        let mut c = Component::new(id, Ecosystem::Maven, id);
        c.direct = direct;
        c.dependencies = deps.iter().map(|s| s.to_string()).collect();
        c
    }

    #[test]
    fn finds_paths_from_direct_dependencies() {
        let list = vec![
            comp("web", Some(true), &["tomcat", "spring"]),
            comp("spring", Some(false), &["tomcat"]),
            comp("other", Some(true), &["spring"]),
            comp("tomcat", Some(false), &[]),
        ];
        let paths = find_paths(&list, "tomcat");
        assert_eq!(paths[0], vec!["web", "tomcat"]);
        assert!(paths.contains(&vec!["web".to_string(), "spring".into(), "tomcat".into()]));
        assert!(paths.contains(&vec!["other".to_string(), "spring".into(), "tomcat".into()]));
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn falls_back_to_roots_and_survives_cycles() {
        let list = vec![comp("a", None, &["b"]), comp("b", None, &["c"]), comp("c", None, &["b"])];
        assert_eq!(find_paths(&list, "c"), vec![vec!["a".to_string(), "b".into(), "c".into()]]);
    }

    #[test]
    fn groups_candidates_by_series() {
        let order = series_order(&["3.2.1".into(), "3.1.5".into(), "3.2.0".into(), "3.10.0".into()]);
        assert_eq!(order.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), vec!["3.1", "3.2", "3.10"]);
        assert_eq!(order[1].1, vec!["3.2.0", "3.2.1"]);
    }

    #[test]
    fn searches_smallest_fixing_version() {
        let candidates: Vec<String> = ["1.4.0", "1.4.1", "1.5.0", "1.5.1", "1.5.2", "1.5.3", "1.6.0"].map(String::from).to_vec();
        struct FixedFrom(&'static str);
        impl Prober for FixedFrom {
            async fn probe(&mut self, v: &str) -> Option<bool> {
                Some(version::compare(v, self.0) != Ordering::Less)
            }
        }
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        // 1.5.2 から直っている
        let (fix, probes) = rt.block_on(search(&candidates, &mut FixedFrom("1.5.2")));
        assert_eq!(fix.as_deref(), Some("1.5.2"));
        assert!(probes <= 5, "{probes}");
        // どこでも直らない
        let (fix, _) = rt.block_on(search(&candidates, &mut FixedFrom("9.9")));
        assert_eq!(fix, None);
    }

    /// 実際の deps.dev・OSV に問い合わせる確認用テスト。通常は実行しない。
    /// `SUC_VULNS=GHSA-...,GHSA-... cargo test live_transitive -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_transitive() {
        let ids: Vec<String> = std::env::var("SUC_VULNS").unwrap_or_default().split(',').map(str::to_string).collect();
        let mut web = Component::new("web", Ecosystem::Maven, "spring-boot-starter-web");
        web.group = Some("org.springframework.boot".into());
        web.version = Some("3.2.0".into());
        web.direct = Some(true);
        web.dependencies = vec!["tomcat".into()];
        let mut tomcat = Component::new("tomcat", Ecosystem::Maven, "tomcat-embed-core");
        tomcat.group = Some("org.apache.tomcat.embed".into());
        tomcat.version = Some("10.1.16".into());
        tomcat.direct = Some(false);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let http = Http::new(None, std::sync::Arc::new(crate::activity::ActivityLog::disabled()));
        let fix = rt.block_on(find_fix(&http, &tomcat, &[web, tomcat.clone()], &ids, Some("10.1.x"), &CheckOptions::default()));
        println!("{fix:#?}");
    }

    #[test]
    fn hints_how_to_pin() {
        let mut c = Component::new("x", Ecosystem::Npm, "qs");
        assert!(pin_hint(&c, Some("6.10.3")).unwrap().contains("6.10.3"));
        c.ecosystem = Ecosystem::Other;
        assert!(pin_hint(&c, None).is_none());
    }
}

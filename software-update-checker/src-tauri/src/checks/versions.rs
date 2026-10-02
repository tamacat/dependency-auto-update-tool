//! 最新バージョンのチェック（deps.dev API v3）。
//! https://docs.deps.dev/api/v3/

use super::{has_concrete_version, ttl, ProgressFn};
use crate::http::Http;
use crate::model::{CheckOptions, CheckProgress, CheckResult, Component, SeriesInfo, UpdateKind, VersionInfo};
use crate::version;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const BASE: &str = "https://api.deps.dev";
/// 詳細画面に出す系列の最大数
const MAX_SERIES: usize = 12;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PackageResponse {
    #[serde(default)]
    versions: Vec<VersionEntry>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct VersionEntry {
    version_key: VersionKey,
    published_at: Option<String>,
    #[serde(default)]
    is_deprecated: bool,
    #[serde(default)]
    deprecated_reason: String,
}

#[derive(Deserialize, Clone)]
struct VersionKey {
    version: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    #[serde(default)]
    licenses: Vec<String>,
}

fn url(segments: &[&str]) -> String {
    let mut u = reqwest::Url::parse(BASE).expect("固定 URL");
    u.path_segments_mut().expect("http URL").pop_if_empty().extend(segments);
    u.to_string()
}

/// パッケージのバージョン一覧。存在しなければ `Ok(None)`。
async fn fetch_package(http: &Http, system: &str, name: &str, opts: &CheckOptions) -> Result<Option<PackageResponse>, String> {
    let res = http.get(&url(&["v3", "systems", system, "packages", name]), ttl(opts, 24)).await?;
    match res.status {
        404 => Ok(None),
        _ if res.is_success() => serde_json::from_str(&res.body).map(Some).map_err(|e| tr!("deps.dev の応答を解釈できません: {e}", "Cannot parse the deps.dev response: {e}")),
        s => Err(format!("deps.dev: HTTP {s}")),
    }
}

async fn fetch_licenses(http: &Http, system: &str, name: &str, ver: &str, opts: &CheckOptions) -> Vec<String> {
    let Ok(res) = http.get(&url(&["v3", "systems", system, "packages", name, "versions", ver]), ttl(opts, 24 * 7)).await else {
        return Vec::new();
    };
    if !res.is_success() {
        return Vec::new();
    }
    serde_json::from_str::<VersionResponse>(&res.body).map(|r| r.licenses).unwrap_or_default()
}

fn max_version<'a>(iter: impl Iterator<Item = &'a VersionEntry>) -> Option<&'a VersionEntry> {
    iter.max_by(|a, b| version::compare(&a.version_key.version, &b.version_key.version))
}

/// バージョン一覧から [`VersionInfo`] を組み立てる（ネットワークを使わない純粋関数）。
fn summarize(pkg: &PackageResponse, current: Option<&str>, opts: &CheckOptions, today: i64) -> VersionInfo {
    // Maven Central には "20041127.091804" のような日付形式の古い誤登録があり、
    // そのまま比較すると最新版に見えてしまう。現行版が日付形式でなければ除外する。
    let date_like = |v: &str| version::major(v).is_some_and(|m| m >= 10_000);
    let current_is_date_like = current.is_some_and(date_like);
    let mut candidates: Vec<&VersionEntry> = pkg
        .versions
        .iter()
        .filter(|v| opts.include_prerelease || !version::is_prerelease(&v.version_key.version))
        .filter(|v| current_is_date_like || !date_like(&v.version_key.version))
        .collect();
    // "-jre" / "-android" / "-jdk5" のような派生版は、現行と同じ種類のものだけを比べる
    if let Some(cur) = current {
        let wanted = version::variant(cur);
        if candidates.iter().any(|v| version::variant(&v.version_key.version) == wanted) {
            candidates.retain(|v| version::variant(&v.version_key.version) == wanted);
        }
    }
    let latest = max_version(candidates.iter().copied());

    let current_entry = current.and_then(|cur| {
        pkg.versions
            .iter()
            .find(|v| v.version_key.version == cur)
            .or_else(|| pkg.versions.iter().find(|v| version::compare(&v.version_key.version, cur) == Ordering::Equal))
    });
    let latest_in_major = current.and_then(version::major).and_then(|m| {
        max_version(candidates.iter().copied().filter(|v| version::major(&v.version_key.version) == Some(m)))
    });
    let current_series = current.and_then(version::series);
    let latest_in_minor = current_series.as_ref().and_then(|s| {
        max_version(candidates.iter().copied().filter(|v| version::series(&v.version_key.version).as_ref() == Some(s)))
    });

    // 系列ごとの最新版（現行の系列以降、新しい順）
    let mut by_series: HashMap<String, &VersionEntry> = HashMap::new();
    for v in &candidates {
        let Some(s) = version::series(&v.version_key.version) else { continue };
        let newer = by_series
            .get(&s)
            .is_none_or(|cur| version::compare(&v.version_key.version, &cur.version_key.version) == Ordering::Greater);
        if newer {
            by_series.insert(s, v);
        }
    }
    let mut series: Vec<SeriesInfo> = by_series
        .into_iter()
        .filter(|(s, _)| {
            current_series.as_ref().is_none_or(|cur| version::compare(s, cur) != Ordering::Less)
        })
        .map(|(s, v)| SeriesInfo {
            is_current: current_series.as_ref() == Some(&s),
            series: s,
            latest: v.version_key.version.clone(),
            published_at: v.published_at.clone(),
        })
        .collect();
    series.sort_by(|a, b| version::compare(&b.series, &a.series));
    series.truncate(MAX_SERIES);

    let update_kind = match (current, &latest) {
        (Some(cur), Some(l)) => version::update_kind(cur, &l.version_key.version),
        _ => UpdateKind::Unknown,
    };

    let last_release_at = pkg.versions.iter().filter_map(|v| v.published_at.clone()).max();
    let stale = last_release_at
        .as_deref()
        .and_then(version::days_from_iso_date)
        .is_some_and(|d| today - d > i64::from(opts.stale_years) * 365);

    VersionInfo {
        latest: latest.as_ref().map(|v| v.version_key.version.clone()),
        latest_published_at: latest.as_ref().and_then(|v| v.published_at.clone()),
        latest_in_major: latest_in_major.map(|v| v.version_key.version.clone()),
        latest_in_minor: latest_in_minor.map(|v| v.version_key.version.clone()),
        series,
        update_kind,
        current_published_at: current_entry.and_then(|v| v.published_at.clone()),
        last_release_at,
        stale,
        deprecated: current_entry.filter(|v| v.is_deprecated).map(|v| {
            if v.deprecated_reason.is_empty() {
                tr!("非推奨（deprecated）", "Deprecated")
            } else {
                v.deprecated_reason.clone()
            }
        }),
        current_not_found: current.is_some() && current_entry.is_none(),
    }
}

pub async fn check(
    http: &Http,
    targets: &[&Component],
    opts: &CheckOptions,
    results: &mut HashMap<String, CheckResult>,
    progress: ProgressFn,
) {
    // 同じパッケージを何度も照会しないよう、(system, name) でまとめる
    let mut packages: HashMap<(&'static str, String), Vec<&Component>> = HashMap::new();
    for c in targets {
        if let Some(system) = c.ecosystem.deps_dev_system() {
            packages.entry((system, c.package_name())).or_default().push(c);
        }
    }

    let total = packages.len();
    let semaphore = Arc::new(Semaphore::new(http.network().max_concurrency));
    let mut tasks = JoinSet::new();
    for ((system, name), _) in packages.iter() {
        let (http, opts, semaphore) = (http.clone(), opts.clone(), semaphore.clone());
        let (system, name) = (*system, name.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            let res = fetch_package(&http, system, &name, &opts).await;
            (system, name, res)
        });
    }

    let today = version::today_days();
    let mut done = 0;
    let mut license_lookups: Vec<(String, &'static str, String, String)> = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        done += 1;
        progress(CheckProgress { phase: "versions".into(), done, total });
        let Ok((system, name, res)) = joined else { continue };
        let Some(components) = packages.get(&(system, name.clone())) else { continue };
        for c in components {
            let Some(result) = results.get_mut(&c.id) else { continue };
            match &res {
                Ok(Some(pkg)) => {
                    let current = has_concrete_version(c).then(|| c.version.as_deref()).flatten();
                    let info = summarize(pkg, current, opts, today);
                    if c.licenses.is_empty() && !info.current_not_found {
                        if let Some(v) = current {
                            license_lookups.push((c.id.clone(), system, name.clone(), v.to_string()));
                        }
                    }
                    result.version = Some(info);
                }
                Ok(None) => result.errors.push(tr!(
                    "deps.dev にパッケージが見つかりません（非公開ライブラリの可能性）",
                    "Package not found on deps.dev (possibly a private library)"
                )),
                Err(e) => result.errors.push(tr!("最新版の取得に失敗: {e}", "Failed to fetch the latest version: {e}")),
            }
        }
    }

    // ライセンスが入力に無いものだけ補完する
    let total = license_lookups.len();
    let mut tasks = JoinSet::new();
    for (id, system, name, ver) in license_lookups {
        let (http, opts, semaphore) = (http.clone(), opts.clone(), semaphore.clone());
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await;
            (id, fetch_licenses(&http, system, &name, &ver, &opts).await)
        });
    }
    let mut done = 0;
    while let Some(joined) = tasks.join_next().await {
        done += 1;
        progress(CheckProgress { phase: "licenses".into(), done, total });
        if let Ok((id, licenses)) = joined {
            if let Some(r) = results.get_mut(&id) {
                r.licenses = licenses;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(entries: &[(&str, &str)]) -> PackageResponse {
        PackageResponse {
            versions: entries
                .iter()
                .map(|(v, d)| VersionEntry {
                    version_key: VersionKey { version: v.to_string() },
                    published_at: Some(d.to_string()),
                    is_deprecated: false,
                    deprecated_reason: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn summarizes_latest_and_latest_in_major() {
        let p = pkg(&[
            ("10.1.20", "2024-03-19T00:00:00Z"),
            ("10.1.60", "2026-09-09T00:00:00Z"),
            ("11.0.26", "2026-09-09T00:00:00Z"),
            ("12.0.0-M1", "2026-09-20T00:00:00Z"),
        ]);
        let today = version::days_from_iso_date("2026-10-03").unwrap();
        let info = summarize(&p, Some("10.1.20"), &CheckOptions::default(), today);
        assert_eq!(info.latest.as_deref(), Some("11.0.26"));
        assert_eq!(info.latest_in_major.as_deref(), Some("10.1.60"));
        assert_eq!(info.latest_in_minor.as_deref(), Some("10.1.60"));
        assert_eq!(
            info.series.iter().map(|s| (s.series.as_str(), s.latest.as_str(), s.is_current)).collect::<Vec<_>>(),
            vec![("11.0", "11.0.26", false), ("10.1", "10.1.60", true)]
        );
        assert_eq!(info.update_kind, UpdateKind::Major);
        assert_eq!(info.current_published_at.as_deref(), Some("2024-03-19T00:00:00Z"));
        assert_eq!(info.last_release_at.as_deref(), Some("2026-09-20T00:00:00Z"));
        assert!(!info.stale);
        assert!(!info.current_not_found);

        let opts = CheckOptions { include_prerelease: true, ..Default::default() };
        let info = summarize(&p, Some("10.1.20"), &opts, today);
        assert_eq!(info.latest.as_deref(), Some("12.0.0-M1"));
    }

    #[test]
    fn latest_in_minor_differs_from_latest_in_major() {
        // logback のように 1.x のまま系列ごとに必要な JDK が変わるケース
        let p = pkg(&[
            ("1.3.16", "2025-10-29T00:00:00Z"),
            ("1.3.17", "2026-01-10T00:00:00Z"),
            ("1.4.14", "2023-12-01T00:00:00Z"),
            ("1.5.34", "2026-09-30T00:00:00Z"),
        ]);
        let today = version::days_from_iso_date("2026-10-03").unwrap();
        let info = summarize(&p, Some("1.3.16"), &CheckOptions::default(), today);
        assert_eq!(info.latest_in_minor.as_deref(), Some("1.3.17"));
        assert_eq!(info.latest_in_major.as_deref(), Some("1.5.34"));
        assert_eq!(info.series.len(), 3);
        assert!(info.series[2].is_current);
    }

    #[test]
    fn compares_only_same_variant() {
        let today = version::days_from_iso_date("2026-10-03").unwrap();
        let p = pkg(&[
            ("1.10.14", "2020-01-01T00:00:00Z"),
            ("1.18.14", "2026-09-01T00:00:00Z"),
            ("1.18.14-jdk5", "2026-09-01T00:00:00Z"),
        ]);
        let info = summarize(&p, Some("1.10.14"), &CheckOptions::default(), today);
        assert_eq!(info.latest.as_deref(), Some("1.18.14"));

        let g = pkg(&[("31.1-jre", "2022-01-01T00:00:00Z"), ("33.5-jre", "2026-01-01T00:00:00Z"), ("33.5-android", "2026-01-01T00:00:00Z")]);
        let info = summarize(&g, Some("31.1-android"), &CheckOptions::default(), today);
        assert_eq!(info.latest.as_deref(), Some("33.5-android"));
    }

    #[test]
    fn ignores_date_like_bogus_versions() {
        let p = pkg(&[
            ("1.22.1", "2026-01-01T00:00:00Z"),
            ("20041127.091804", "2005-01-01T00:00:00Z"),
        ]);
        let today = version::days_from_iso_date("2026-10-03").unwrap();
        let info = summarize(&p, Some("1.22.1"), &CheckOptions::default(), today);
        assert_eq!(info.latest.as_deref(), Some("1.22.1"));
        assert_eq!(info.update_kind, UpdateKind::None);
    }

    #[test]
    fn detects_stale_and_missing_current() {
        let p = pkg(&[("1.0", "2015-01-01T00:00:00Z")]);
        let today = version::days_from_iso_date("2026-10-03").unwrap();
        let info = summarize(&p, Some("0.9-internal"), &CheckOptions::default(), today);
        assert!(info.stale);
        assert!(info.current_not_found);
    }

    #[test]
    fn builds_encoded_urls() {
        assert_eq!(
            url(&["v3", "systems", "NPM", "packages", "@angular/core"]),
            "https://api.deps.dev/v3/systems/NPM/packages/@angular%2Fcore"
        );
    }
}

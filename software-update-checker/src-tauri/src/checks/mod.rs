//! 取り込んだコンポーネントに対するチェック処理。
//!
//! チェックの種類ごとにモジュールを分けている。
//! - [`versions`] — 最新バージョン・非推奨・最終リリース日（deps.dev）
//! - [`vulns`] — 既知の脆弱性（OSV.dev）
//! - [`eol`] — サポート終了と系列ごとの要件（endoflife.date）
//! - [`java`] — jar から必要な Java を判定（Maven Central、詳細画面から個別に呼ぶ）
//!
//! 新しいチェック（ライセンス判定、社内リポジトリの照会など）を足すときは、
//! モジュールを追加して [`run_checks`] から呼ぶ。

mod cvss;
mod eol;
pub mod java;
mod versions;
mod vulns;

use crate::http::Http;
use crate::model::{CheckOptions, CheckProgress, CheckResult, Component};
use crate::purl::Purl;
use crate::sources;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub type ProgressFn = Arc<dyn Fn(CheckProgress) + Send + Sync>;


/// チェック対象として扱えるコンポーネント（エコシステムが分かるもの）。
pub(crate) fn checkable(c: &Component) -> bool {
    c.ecosystem.purl_type().is_some() && !c.name.is_empty()
}

/// バージョンが確定しているか（`${...}` の未解決や `[1.0,)` のような範囲指定でないか）。
pub(crate) fn has_concrete_version(c: &Component) -> bool {
    c.version
        .as_deref()
        .is_some_and(|v| !v.is_empty() && !v.contains("${") && !v.starts_with(['[', '(']))
}

/// qualifiers を落とした purl。`with_version` が true ならバージョン付き。
pub(crate) fn canonical_purl(c: &Component, with_version: bool) -> Option<String> {
    let purl_type = c.ecosystem.purl_type()?;
    let p = Purl::build(purl_type, c.group.as_deref(), &c.name, c.version.as_deref());
    Some(p.to_string_canonical(with_version))
}

pub(crate) fn ttl(opts: &CheckOptions, hours: u64) -> Option<Duration> {
    opts.use_cache.then(|| Duration::from_secs(hours * 3600))
}

pub async fn run_checks(
    http: &Http,
    components: &[Component],
    opts: &CheckOptions,
    progress: ProgressFn,
) -> Vec<CheckResult> {
    let targets: Vec<&Component> = components.iter().filter(|c| checkable(c)).collect();
    let mut results: HashMap<String, CheckResult> = targets
        .iter()
        .map(|c| {
            (c.id.clone(), CheckResult { component_id: c.id.clone(), ..Default::default() })
        })
        .collect();

    // 無効にしたデータソースのチェックは丸ごと飛ばす（Http 側でも通信は遮断される）
    let enabled: Vec<&str> = sources::SOURCES.iter().map(|s| s.id).filter(|id| opts.source_enabled(id)).collect();
    http.log().info(tr!(
        "チェック開始: {} 件（対象 {} 件）。有効なデータソース: {}",
        "Check started: {} components ({} checkable). Enabled data sources: {}",
        components.len(),
        targets.len(),
        enabled.join(", ")
    ));
    let started = std::time::Instant::now();

    if opts.source_enabled(sources::DEPS_DEV) {
        versions::check(http, &targets, opts, &mut results, progress.clone()).await;
    }
    if opts.source_enabled(sources::OSV) {
        vulns::check(http, &targets, opts, &mut results, progress.clone()).await;
    }
    if opts.source_enabled(sources::ENDOFLIFE) {
        eol::check(http, &targets, opts, &mut results, progress.clone()).await;
    }
    http.log().info(tr!("チェック完了: {:.1} 秒", "Check finished in {:.1} s", started.elapsed().as_secs_f64()));

    progress(CheckProgress { phase: "done".into(), done: 1, total: 1 });
    // 入力順を保って返す
    targets.iter().filter_map(|c| results.remove(&c.id)).collect()
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// 実際の外部 API に問い合わせる確認用テスト。通常は実行しない。
    /// `SUC_LIVE_INPUT=<pom.xml や SBOM のパス> cargo test live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_check() {
        let path = std::env::var("SUC_LIVE_INPUT").expect("SUC_LIVE_INPUT を指定してください");
        let imported = crate::importers::import_file(std::path::Path::new(&path)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let http = Http::new(None, Arc::new(crate::activity::ActivityLog::disabled()));
        let results = runtime.block_on(run_checks(
            &http,
            &imported.components,
            &CheckOptions::default(),
            Arc::new(|_| {}),
        ));
        assert!(!results.is_empty());
        // 画面確認用の模擬データ（src/lib/fixtures）を作るときに使う
        if let Ok(out) = std::env::var("SUC_LIVE_DUMP") {
            // 系列ごとの最新版について Java 要件も取得しておく（模擬バックエンドが返す）
            let mut java = serde_json::Map::new();
            for (c, r) in imported.components.iter().zip(results.iter()) {
                let (Some(group), Some(info)) = (c.group.as_deref(), r.version.as_ref()) else { continue };
                if c.ecosystem != crate::model::Ecosystem::Maven {
                    continue;
                }
                let mut versions: Vec<String> = info.series.iter().map(|s| s.latest.clone()).collect();
                versions.extend(c.version.clone());
                let store = crate::knowledge::KnowledgeStore::open(None);
                let reqs: Vec<_> = versions
                    .iter()
                    .map(|v| runtime.block_on(java::resolve(&http, &store, &[], group, &c.name, v, true)))
                    .collect();
                java.insert(c.id.clone(), serde_json::to_value(reqs).unwrap());
            }
            let json = serde_json::json!({ "import": imported, "results": results, "java": java });
            std::fs::write(out, serde_json::to_string_pretty(&json).unwrap()).unwrap();
        }
        for (c, r) in imported.components.iter().zip(results.iter()) {
            let v = r.version.as_ref();
            println!(
                "{:<60} {:<12} latest={:<14} {:?} vulns={:<3} eol={} errors={:?}",
                c.package_name(),
                c.version.as_deref().unwrap_or("-"),
                v.and_then(|v| v.latest.as_deref()).unwrap_or("-"),
                v.map(|v| v.update_kind),
                r.vulnerabilities.len(),
                r.eol.as_ref().map(|e| format!("{}:{:?}:{:?}", e.product, e.cycle, e.status)).unwrap_or("-".into()),
                r.errors,
            );
        }
    }
}

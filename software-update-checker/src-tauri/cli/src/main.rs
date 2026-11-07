//! Software Update Checker のコマンドライン版（CI 向け）。
//!
//! pom.xml・SBOM・lock ファイルを読み込んでチェックし、方針（`--fail-on`）に当たる指摘があれば
//! 終了コード 1 で終わる。結果はテキスト・JSON・SARIF（GitHub の Code scanning 用）で出力できる。
//!
//! 終了コード: 0 = 問題なし、1 = 方針に当たる指摘あり、2 = 引数や入力の誤り

mod args;
mod report;

use args::{Args, Format};
use software_update_checker_core::activity::{ActivityLog, LogEntry, Sink};
use software_update_checker_core::checks::{self, ProgressFn};
use software_update_checker_core::http::Http;
use software_update_checker_core::model::{CheckOptions, CheckProgress};
use software_update_checker_core::{i18n, importers, policy};
use std::process::ExitCode;
use std::sync::Arc;

fn main() -> ExitCode {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => return ExitCode::SUCCESS, // --help / --version
        Err(e) => {
            eprintln!("error: {e}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    i18n::set_language(&args.lang);
    match run(&args) {
        Ok(failed) if failed && !args.no_fail => ExitCode::from(1),
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// チェックして報告を書く。方針に当たる指摘があれば `Ok(true)`。
fn run(args: &Args) -> Result<bool, String> {
    let imported = importers::import_file(&args.input)?;
    if args.verbose {
        eprintln!("{} ({}, {} components)", imported.source_path, imported.format, imported.components.len());
        for w in &imported.warnings {
            eprintln!("warning: {w}");
        }
    }

    let options = CheckOptions {
        disabled_sources: args.disabled_sources.clone(),
        include_prerelease: args.include_prerelease,
        supply_chain: args.supply_chain,
        use_cache: args.cache.is_some(),
        ..Default::default()
    };
    let sink: Option<Sink> = if args.verbose {
        Some(Box::new(|e: &LogEntry| {
            let what = e.url.as_deref().or(e.message.as_deref()).unwrap_or_default();
            eprintln!("[{}] {} {}", e.outcome, e.status.map(|s| s.to_string()).unwrap_or_default(), what);
        }))
    } else {
        None
    };
    let http = Http::new(args.cache.clone(), Arc::new(ActivityLog::new(None, sink)));
    http.configure(&options);

    let verbose = args.verbose;
    let progress: ProgressFn = Arc::new(move |p: CheckProgress| {
        if verbose && p.total > 0 {
            eprintln!("{} {}/{}", p.phase, p.done, p.total);
        }
    });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    let results = runtime.block_on(checks::run_checks(&http, &imported.components, &options, progress));
    if args.cache.is_some() {
        if let Err(e) = runtime.block_on(http.save()) {
            eprintln!("warning: cannot save the cache: {e}");
        }
    }

    let findings = policy::evaluate(&imported.components, &results, &args.policy);
    let failed = findings.iter().any(|f| f.failing());
    let ctx = report::Context { input: &args.input_display, imported: &imported, results: &results, findings: &findings, policy: &args.policy };
    let body = match args.format {
        Format::Text => report::text(&ctx),
        Format::Json => report::json(&ctx),
        Format::Sarif => report::sarif(&ctx, std::fs::read_to_string(&args.input).ok().as_deref()),
    };
    match &args.output {
        Some(path) => std::fs::write(path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?,
        None => print!("{body}"),
    }
    // JSON・SARIF を標準出力やファイルに出すときも、結果の要約は標準エラーに出す
    if args.format != Format::Text || args.output.is_some() {
        eprintln!("{}", report::summary_line(&ctx));
    }
    Ok(failed)
}

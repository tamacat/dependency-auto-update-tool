//! ローカルの Maven で CycloneDX SBOM を生成する。
//!
//! pom.xml の直接読込では親 POM・BOM・推移的依存を解決できないため、
//! 正確な依存一覧が必要なときは Maven 自身に解決させる。
//! 対象プロジェクトの pom.xml は変更せず、プラグインをコマンドラインから
//! 直接呼び出す（出力は対象プロジェクトの target/ 配下）。

use crate::activity::{ActivityLog, LogEntry};
use crate::sources;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const CYCLONEDX_PLUGIN: &str = "org.cyclonedx:cyclonedx-maven-plugin:2.9.3";
const OUTPUT_NAME: &str = "software-update-checker-bom";

/// SBOM を生成し、生成されたファイルのパスを返す。時間がかかるので
/// 呼び出し側でブロッキング用スレッドに逃がすこと。
pub fn generate_sbom(pom_path: &Path, log: &ActivityLog) -> Result<PathBuf, String> {
    let started = Instant::now();
    let result = run_maven(pom_path);
    // Maven 自身の通信（リポジトリからの依存解決）はこのツールの外で行われるため、
    // 実行したこととその結果だけを記録する
    log.record(LogEntry {
        kind: "process".into(),
        source: Some(sources::MAVEN_LOCAL.into()),
        method: Some("mvn".into()),
        url: Some(format!("{CYCLONEDX_PLUGIN}:makeAggregateBom -f {}", pom_path.display())),
        duration_ms: Some(started.elapsed().as_millis() as u64),
        outcome: if result.is_ok() { "network" } else { "error" }.into(),
        message: Some(match &result {
            Ok(p) => tr!("SBOM を生成: {}", "SBOM generated: {}", p.display()),
            Err(e) => e.lines().next().unwrap_or_default().to_string(),
        }),
        ..Default::default()
    });
    result
}

fn run_maven(pom_path: &Path) -> Result<PathBuf, String> {
    let project_dir = pom_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| tr!("{} の親ディレクトリが分かりません", "Cannot determine the directory of {}", pom_path.display()))?;

    let mvn = if cfg!(windows) { "mvn.cmd" } else { "mvn" };
    let output = Command::new(mvn)
        .current_dir(project_dir)
        .args([
            "-B",
            "-q",
            "-f",
            &pom_path.to_string_lossy(),
            &format!("{CYCLONEDX_PLUGIN}:makeAggregateBom"),
            "-DoutputFormat=json",
            &format!("-DoutputName={OUTPUT_NAME}"),
            "-DincludeTestScope=true",
        ])
        .output()
        .map_err(|e| {
            tr!(
                "Maven（{mvn}）を実行できません。PATH に Maven があるか確認してください: {e}",
                "Cannot run Maven ({mvn}). Check that Maven is on the PATH: {e}"
            )
        })?;

    if !output.status.success() {
        let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
        log.push_str(&String::from_utf8_lossy(&output.stderr));
        let tail: Vec<&str> = log.lines().rev().take(30).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(tr!("Maven での SBOM 生成に失敗しました:\n{}", "Maven failed to generate the SBOM:\n{}", tail.join("\n")));
    }

    let bom = project_dir.join("target").join(format!("{OUTPUT_NAME}.json"));
    if bom.exists() {
        Ok(bom)
    } else {
        Err(tr!("Maven は成功しましたが {} が見つかりません", "Maven succeeded but {} was not found", bom.display()))
    }
}

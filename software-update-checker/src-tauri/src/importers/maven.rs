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

/// `pom.xml` 以外の名前の pom（Maven リポジトリに公開された `{artifact}-{version}.pom` など）は
/// プロジェクトのフォルダではないので、一時フォルダに pom.xml としてコピーして実行する。
/// そうしないと、公開用リポジトリの中に target/ が作られてしまう。
fn working_pom(pom_path: &Path) -> Result<PathBuf, String> {
    if pom_path.file_name().and_then(|n| n.to_str()) == Some("pom.xml") {
        return Ok(pom_path.to_path_buf());
    }
    let dir = private_temp_dir(&std::env::temp_dir())
        .map_err(|e| tr!("一時フォルダを作れません: {e}", "Cannot create a temporary directory: {e}"))?;
    let copy = dir.join("pom.xml");
    std::fs::copy(pom_path, &copy).map_err(|e| tr!("{} をコピーできません: {e}", "Cannot copy {}: {e}", pom_path.display()))?;
    Ok(copy)
}

/// Maven を実行する一時フォルダを作る。
///
/// Maven は実行フォルダの `.mvn/`（拡張機能や設定）を読むため、他のユーザーが同名のフォルダを
/// 先に作って中身を仕込めると、そのコードが実行されうる（Linux / macOS の共有 /tmp）。
/// そのため名前は推測できないものにし、既に存在するフォルダは決して使わず（新規作成のみ）、
/// Unix では本人だけが読み書きできる権限（700）で作る。
fn private_temp_dir(base: &Path) -> std::io::Result<PathBuf> {
    use std::hash::{BuildHasher, Hasher};
    let mut last_error = None;
    for attempt in 0..8u32 {
        // RandomState は OS の乱数で初期化されるので、出力は外から推測できない
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        h.write_i64(crate::activity::now_millis());
        h.write_u32(attempt);
        let dir = base.join(format!("software-update-checker-mvn-{:016x}", h.finish()));
        // 権限の指定は Unix だけなので、それ以外では mut が使われない
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_error = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_error.unwrap_or_else(|| std::io::Error::other("temporary directory")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_dirs_are_new_unpredictable_and_private() {
        let base = std::env::temp_dir().join(format!("suc-mvn-test-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let a = private_temp_dir(&base).unwrap();
        let b = private_temp_dir(&base).unwrap();
        assert_ne!(a, b, "毎回別のフォルダを作る");
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("software-update-checker-mvn-") && name.len() == "software-update-checker-mvn-".len() + 16);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn pom_xml_is_used_in_place_and_other_poms_are_copied() {
        let base = std::env::temp_dir().join(format!("suc-mvn-copy-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let project = base.join("pom.xml");
        std::fs::write(&project, "<project/>").unwrap();
        assert_eq!(working_pom(&project).unwrap(), project);

        let published = base.join("app-1.0.pom");
        std::fs::write(&published, "<project/>").unwrap();
        let copy = working_pom(&published).unwrap();
        assert_eq!(copy.file_name().unwrap(), "pom.xml");
        assert_ne!(copy.parent().unwrap(), base.as_path(), "元のフォルダでは実行しない");
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "<project/>");
        std::fs::remove_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }
}

fn run_maven(original_pom: &Path) -> Result<PathBuf, String> {
    let pom_path = &working_pom(original_pom)?;
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

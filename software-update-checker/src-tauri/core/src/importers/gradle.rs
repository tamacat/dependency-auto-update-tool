//! Gradle の依存のロック（dependency locking）の gradle.lockfile の取り込み。
//! 仕様: https://docs.gradle.org/current/userguide/dependency_locking.html
//!
//! 1 行が `group:artifact:version=構成1,構成2`。推移的依存も含むが、つながりと直接依存の区別は無い。
//! テスト用の構成（testCompileClasspath など）だけで使われるものは範囲を `test` にする。
//! build.gradle は書き方の自由度が高く正確に読めないため、扱わない
//! （CycloneDX Gradle プラグインで作った SBOM なら推移的依存とつながりも読める）。

use super::Importer;
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use std::path::Path;

pub struct GradleLockImporter;

impl Importer for GradleLockImporter {
    fn format_id(&self) -> &'static str {
        "gradle-lockfile"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with("gradle.lockfile"))
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let components = parse(content);
        Ok(ImportResult {
            source_path: path.display().to_string(),
            format: self.format_id().into(),
            project_name: None,
            components,
            warnings: vec![tr!(
                "gradle.lockfile には依存関係のつながりと直接依存の区別が無いため、ツリーと直接依存の判定はできません（CycloneDX Gradle プラグインの SBOM なら読めます）。",
                "gradle.lockfile has no dependency graph and does not mark direct dependencies (an SBOM from the CycloneDX Gradle plugin has both)."
            )],
            includes_transitive: true,
            related_files: Vec::new(),
        })
    }
}

fn parse(content: &str) -> Vec<Component> {
    let mut components: Vec<Component> = Vec::new();
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with("empty=") {
            continue;
        }
        let (coordinate, configurations) = line.split_once('=').unwrap_or((line, ""));
        let parts: Vec<&str> = coordinate.split(':').collect();
        let [group, artifact, version] = parts.as_slice() else { continue };
        let id = format!("maven:{group}:{artifact}@{version}");
        if components.iter().any(|c| c.id == id) {
            continue;
        }
        let mut c = Component::new(id, Ecosystem::Maven, *artifact);
        c.group = Some(group.to_string());
        c.version = Some(version.to_string());
        c.purl = Some(Purl::build("maven", Some(group), artifact, Some(version)).to_string_canonical(true));
        let configs: Vec<&str> = configurations.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
        if !configs.is_empty() && configs.iter().all(|c| c.to_ascii_lowercase().contains("test")) {
            c.scope = Some("test".into());
        }
        components.push(c);
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gradle_lockfile() {
        let lock = "# This is a Gradle generated file for dependency locking.\n# Manual edits can break the build and are not advised.\n# This file is expected to be part of source control.\n\
com.google.guava:guava:33.0.0-jre=compileClasspath,runtimeClasspath,testCompileClasspath,testRuntimeClasspath\n\
junit:junit:4.13.2=testCompileClasspath,testRuntimeClasspath\n\
org.hamcrest:hamcrest-core:1.3=testRuntimeClasspath\n\
empty=annotationProcessor\n";
        let list = parse(lock);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].package_name(), "com.google.guava:guava");
        assert_eq!(list[0].purl.as_deref(), Some("pkg:maven/com.google.guava/guava@33.0.0-jre"));
        assert_eq!(list[0].scope, None);
        assert_eq!(list[1].scope.as_deref(), Some("test"));
        assert!(list.iter().all(|c| c.direct.is_none()));
    }
}

//! Rust の Cargo.lock の取り込み（version 1〜4）。
//! 仕様: https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html
//!
//! - `source` の無い記録はワークスペースの部品（自分たちのコード）で、その依存を直接依存とする。
//! - 依存先は `name`、同じ名前の版が複数あるときは `name version`（`(source)` が付くこともある）。
//! - crates.io 以外（git・社内レジストリ）から入れたものは照会しない。
//! - Cargo.lock には開発用（dev-dependencies）の区別が無いので、範囲は付けない。

use super::{apply_roots, Importer};
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
const CRATES_IO_SPARSE: &str = "sparse+https://index.crates.io/";

pub struct CargoLockImporter;

#[derive(Deserialize)]
struct CargoLock {
    #[serde(default)]
    package: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    version: String,
    source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

impl Importer for CargoLockImporter {
    fn format_id(&self) -> &'static str {
        "cargo-lock"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some("Cargo.lock")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let (project_name, components, warnings) = parse(content)?;
        Ok(ImportResult {
            source_path: path.display().to_string(),
            format: self.format_id().into(),
            project_name,
            components,
            warnings,
            includes_transitive: true,
            related_files: Vec::new(),
        })
    }
}

fn component_id(name: &str, version: &str) -> String {
    format!("cargo:{name}@{version}")
}

/// （プロジェクト名、部品、警告）
type Parsed = (Option<String>, Vec<Component>, Vec<String>);

fn parse(content: &str) -> Result<Parsed, String> {
    let lock: CargoLock = toml::from_str(content).map_err(|e| tr!("Cargo.lock が不正です: {e}", "Invalid Cargo.lock: {e}"))?;
    let mut warnings = Vec::new();

    // 名前 → 版の一覧（依存先が名前だけで書かれているときに使う）
    let mut versions: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in &lock.package {
        versions.entry(p.name.as_str()).or_default().push(p.version.as_str());
    }
    let resolve = |dep: &str| -> Option<String> {
        let mut parts = dep.split_whitespace();
        let name = parts.next()?;
        let version = match parts.next() {
            Some(v) => v,
            None => *versions.get(name)?.first()?,
        };
        Some(component_id(name, version))
    };

    let is_own = |p: &Package| p.source.is_none();
    let project_name = lock.package.iter().find(|p| is_own(p)).map(|p| format!("{}@{}", p.name, p.version));
    let mut components = Vec::new();
    let mut prod = BTreeSet::new();
    let mut not_crates_io = 0;
    for p in &lock.package {
        let deps: BTreeSet<String> = p.dependencies.iter().filter_map(|d| resolve(d)).collect();
        if is_own(p) {
            prod.extend(deps);
            continue;
        }
        let from_crates_io = matches!(p.source.as_deref(), Some(CRATES_IO) | Some(CRATES_IO_SPARSE));
        let mut c = Component::new(component_id(&p.name, &p.version), Ecosystem::Cargo, &p.name);
        if from_crates_io {
            c.version = Some(p.version.clone());
            c.purl = Some(Purl::build("cargo", None, &p.name, Some(&p.version)).to_string_canonical(true));
        } else {
            not_crates_io += 1;
            c.notes.push(tr!(
                "crates.io 以外（{}）から入れた版 {} のため、照会しません。",
                "Version {} comes from outside crates.io ({}), so it is not looked up.",
                p.source.as_deref().unwrap_or("-"),
                p.version
            ));
        }
        c.dependencies = deps.into_iter().collect();
        components.push(c);
    }
    // ワークスペースの部品同士の依存は直接依存に含めない
    let own: BTreeSet<String> = lock.package.iter().filter(|p| is_own(p)).map(|p| component_id(&p.name, &p.version)).collect();
    prod.retain(|id| !own.contains(id));
    for c in &mut components {
        c.dependencies.retain(|d| !own.contains(d));
    }

    if prod.is_empty() {
        warnings.push(tr!(
            "Cargo.lock にプロジェクト自身の記録が無いため、直接依存か推移的依存かを判定できません。",
            "Cargo.lock has no entry for the project itself, so direct and transitive dependencies cannot be told apart."
        ));
    } else {
        apply_roots(&mut components, &prod, &BTreeSet::new());
    }
    warnings.push(tr!(
        "Cargo.lock には開発用（dev-dependencies）の区別が無いため、テスト・開発用の依存も同じように表示します。",
        "Cargo.lock does not mark dev-dependencies, so test and development dependencies are shown like the others."
    ));
    if not_crates_io > 0 {
        warnings.push(tr!(
            "{not_crates_io} 件は crates.io 以外（git・社内レジストリなど）から入れたもので、チェック対象外です。",
            "{not_crates_io} crates come from outside crates.io (git, private registries, etc.) and are not checked."
        ));
    }
    Ok((project_name, components, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cargo_lock() {
        let lock = r#"
version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = [
 "serde",
 "tokio 1.38.0",
 "util",
]

[[package]]
name = "util"
version = "0.1.0"
dependencies = ["rand"]

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[package]]
name = "tokio"
version = "1.38.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["bytes"]

[[package]]
name = "tokio"
version = "0.2.25"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "bytes"
version = "1.6.0"
source = "sparse+https://index.crates.io/"

[[package]]
name = "rand"
version = "0.9.0"
source = "git+https://github.com/rust-random/rand?rev=abc#abc"
"#;
        let (name, list, warnings) = parse(lock).unwrap();
        assert_eq!(name.as_deref(), Some("app@0.1.0"));
        // app と util（ワークスペース）は部品にしない
        assert_eq!(list.len(), 5);
        let find = |id: &str| list.iter().find(|c| c.id == id).unwrap();
        let tokio = find("cargo:tokio@1.38.0");
        assert_eq!(tokio.direct, Some(true));
        assert_eq!(tokio.dependencies, vec!["cargo:bytes@1.6.0"]);
        assert_eq!(tokio.purl.as_deref(), Some("pkg:cargo/tokio@1.38.0"));
        assert_eq!(find("cargo:tokio@0.2.25").direct, Some(false));
        assert_eq!(find("cargo:bytes@1.6.0").version.as_deref(), Some("1.6.0"));
        // util の依存（rand）も直接依存。ただし crates.io 以外なので照会しない
        let rand = find("cargo:rand@0.9.0");
        assert_eq!(rand.direct, Some(true));
        assert!(rand.version.is_none() && rand.purl.is_none());
        assert_eq!(warnings.len(), 2);
    }
}

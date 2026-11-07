//! pnpm の pnpm-lock.yaml の取り込み（lockfileVersion 5.x・6.x・9.x）。
//! 仕様: https://github.com/pnpm/spec/tree/master/lockfile
//!
//! - 直接依存は `importers`（ワークスペースの各プロジェクト。単一プロジェクトの 5.x・6.x は最上位）の
//!   `dependencies`・`optionalDependencies`・`devDependencies`。
//! - 依存関係は 9.x では `snapshots`、それより前は `packages` の `dependencies`。
//! - 同じ名前・版で peer 依存の組み合わせだけが違う記録（`react-dom@18.2.0(react@18.2.0)`）は 1 つにまとめる。
//! - 実行時の依存から辿れないものは開発用（`dev`）。

use super::npm::npm_component;
use super::yaml::{self, Node};
use super::{apply_roots, Importer};
use crate::model::{Component, ImportResult};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub struct PnpmLockImporter;

impl Importer for PnpmLockImporter {
    fn format_id(&self) -> &'static str {
        "pnpm-lock"
    }

    fn can_import(&self, path: &Path, content: &str) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some("pnpm-lock.yaml") || content.trim_start().starts_with("lockfileVersion:")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let (components, warnings) = parse(content)?;
        Ok(ImportResult {
            source_path: path.display().to_string(),
            format: self.format_id().into(),
            project_name: None,
            components,
            warnings,
            includes_transitive: true,
            related_files: Vec::new(),
        })
    }
}

/// peer 依存の組み合わせ（`(react@18.2.0)`、5.x の `_react@18.2.0`）を落とした版。
fn strip_peers(version: &str) -> &str {
    let v = version.split('(').next().unwrap_or(version);
    // 5.x は `18.2.0_react@18.2.0`。版そのものに `_` は使われない
    v.split('_').next().unwrap_or(v)
}

/// 記録のキー（`express@4.17.1`、`/express@4.17.1`、5.x の `/express/4.17.1`）から名前と版。
fn parse_key(key: &str, v5: bool) -> Option<(String, String)> {
    let key = key.trim_start_matches('/');
    let key = strip_peers(key);
    let split = if v5 { key.rfind('/') } else { key.rfind('@').filter(|&i| i > 0) }?;
    let (name, version) = (&key[..split], &key[split + 1..]);
    (!name.is_empty() && !version.is_empty()).then(|| (name.to_string(), version.to_string()))
}

/// 依存先の参照（`dep: 1.2.3(peer@1)`、別名の `/other@1.0.0`、`link:../x`）から名前と版。
fn resolve_ref(name: &str, reference: &str, v5: bool) -> Option<(String, String)> {
    if reference.starts_with("link:") || reference.starts_with("file:") || reference.starts_with("workspace:") {
        return None;
    }
    if reference.starts_with('/') {
        return parse_key(reference, v5);
    }
    // 9.x の別名は `other@1.0.0`
    if !reference.starts_with(|c: char| c.is_ascii_digit()) {
        if let Some(found) = parse_key(reference, false) {
            return Some(found);
        }
    }
    Some((name.to_string(), strip_peers(reference).to_string()))
}

/// 依存の並び（`name: version` か、6.x 以降の importers の `name: { specifier, version }`）。
fn dependency_refs(node: Option<&Node>) -> Vec<(String, String)> {
    node.map(|n| {
        n.entries()
            .iter()
            .filter_map(|(name, v)| {
                let reference = v.as_str().or_else(|| v.get("version").and_then(Node::as_str))?;
                Some((name.clone(), reference.to_string()))
            })
            .collect()
    })
    .unwrap_or_default()
}

/// pnpm 10 以降は、pnpm 自身の記録（`packageManagerDependencies` など）を別の文書として先頭に書く。
/// プロジェクトの依存を記録した文書を選ぶ。
fn project_document(content: &str) -> Node {
    let docs = yaml::parse_documents(content);
    let tool_only = |doc: &Node| {
        doc.get("importers").is_some_and(|imps| {
            imps.entries().iter().all(|(_, imp)| {
                imp.entries().iter().all(|(k, _)| matches!(k.as_str(), "packageManagerDependencies" | "configDependencies"))
            })
        })
    };
    let count = docs.len();
    docs.into_iter()
        .enumerate()
        .find(|(i, d)| d.get("lockfileVersion").is_some() && (!tool_only(d) || i + 1 == count))
        .map(|(_, d)| d)
        .unwrap_or(Node::Map(Vec::new()))
}

fn parse(content: &str) -> Result<(Vec<Component>, Vec<String>), String> {
    let doc = project_document(content);
    let version = doc
        .get("lockfileVersion")
        .and_then(Node::as_str)
        .ok_or_else(|| tr!("pnpm-lock.yaml に lockfileVersion がありません", "pnpm-lock.yaml has no lockfileVersion"))?;
    let major: u32 = version.split('.').next().and_then(|m| m.parse().ok()).unwrap_or(0);
    let v5 = major < 6;
    let mut warnings = Vec::new();

    // 部品と依存関係（9.x は snapshots に依存関係、packages に部品の情報）
    let mut components: BTreeMap<String, Component> = BTreeMap::new();
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let id_of = |name: &str, version: &str| npm_component(name, Some(version)).id;
    let sections: Vec<&Node> = ["packages", "snapshots"].iter().filter_map(|s| doc.get(s)).collect();
    for section in sections {
        for (key, entry) in section.entries() {
            let Some((name, ver)) = parse_key(key, v5) else { continue };
            let id = id_of(&name, &ver);
            components.entry(id.clone()).or_insert_with(|| {
                let mut c = npm_component(&name, Some(&ver));
                if c.version.is_none() {
                    c.notes.push(tr!(
                        "レジストリ以外から入れたもの（{ver}）のため、チェック対象外です。",
                        "Installed from outside the registry ({ver}), so it is not checked."
                    ));
                }
                c
            });
            let deps = dependency_refs(entry.get("dependencies"))
                .into_iter()
                .chain(dependency_refs(entry.get("optionalDependencies")));
            for (dep, reference) in deps {
                if let Some((n, v)) = resolve_ref(&dep, &reference, v5) {
                    edges.entry(id.clone()).or_default().insert(id_of(&n, &v));
                }
            }
        }
    }

    // 直接依存（ワークスペースの各プロジェクト、単一プロジェクトなら最上位）
    let importers: Vec<&Node> = match doc.get("importers") {
        Some(i) => i.entries().iter().map(|(_, n)| n).collect(),
        None => vec![&doc],
    };
    let (mut prod, mut dev) = (BTreeSet::new(), BTreeSet::new());
    for imp in importers {
        for (section, is_dev) in [("dependencies", false), ("optionalDependencies", false), ("devDependencies", true)] {
            for (dep, reference) in dependency_refs(imp.get(section)) {
                if let Some((n, v)) = resolve_ref(&dep, &reference, v5) {
                    let target = if is_dev { &mut dev } else { &mut prod };
                    target.insert(id_of(&n, &v));
                }
            }
        }
    }

    let mut list: Vec<Component> = components.into_values().collect();
    for c in &mut list {
        c.dependencies = edges.remove(&c.id).map(|s| s.into_iter().filter(|d| d != &c.id).collect()).unwrap_or_default();
    }
    if prod.is_empty() && dev.is_empty() {
        warnings.push(tr!(
            "pnpm-lock.yaml にプロジェクトの依存の記録が無いため、直接依存か推移的依存かを判定できません。",
            "pnpm-lock.yaml has no dependencies for the project itself, so direct and transitive dependencies cannot be told apart."
        ));
    } else {
        apply_roots(&mut list, &prod, &dev);
    }
    Ok((list, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(list: &'a [Component], id: &str) -> &'a Component {
        list.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} がありません"))
    }

    #[test]
    fn reads_lockfile_v9() {
        let lock = "lockfileVersion: '9.0'\n\nsettings:\n  autoInstallPeers: true\n\n\
importers:\n\n  .:\n    dependencies:\n      express:\n        specifier: ^4.17.1\n        version: 4.17.1\n      react-dom:\n        specifier: ^18.2.0\n        version: 18.2.0(react@18.2.0)\n    devDependencies:\n      '@types/node':\n        specifier: ^20.0.0\n        version: 20.0.0\n      local:\n        specifier: workspace:*\n        version: link:packages/local\n\n\
packages:\n\n  '@types/node@20.0.0':\n    resolution: {integrity: sha512-a}\n\n  express@4.17.1:\n    resolution: {integrity: sha512-b}\n    engines: {node: '>= 0.10.0'}\n\n  ms@2.0.0:\n    resolution: {integrity: sha512-c}\n\n  react-dom@18.2.0:\n    resolution: {integrity: sha512-d}\n    peerDependencies:\n      react: ^18.2.0\n\n  react@18.2.0:\n    resolution: {integrity: sha512-e}\n\n\
snapshots:\n\n  '@types/node@20.0.0': {}\n\n  express@4.17.1:\n    dependencies:\n      ms: 2.0.0\n\n  ms@2.0.0: {}\n\n  react-dom@18.2.0(react@18.2.0):\n    dependencies:\n      react: 18.2.0\n\n  react@18.2.0: {}\n";
        let (list, warnings) = parse(lock).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(list.len(), 5);
        let express = find(&list, "npm:express@4.17.1");
        assert_eq!(express.direct, Some(true));
        assert_eq!(express.scope, None);
        assert_eq!(express.dependencies, vec!["npm:ms@2.0.0"]);
        assert_eq!(find(&list, "npm:react-dom@18.2.0").dependencies, vec!["npm:react@18.2.0"]);
        assert_eq!(find(&list, "npm:ms@2.0.0").direct, Some(false));
        let types = find(&list, "npm:@types/node@20.0.0");
        assert_eq!((types.direct, types.scope.as_deref()), (Some(true), Some("dev")));
        assert_eq!(types.group.as_deref(), Some("@types"));
        assert_eq!(types.purl.as_deref(), Some("pkg:npm/%40types/node@20.0.0"));
    }

    #[test]
    fn reads_lockfile_v6_and_v5() {
        let v6 = "lockfileVersion: '6.0'\n\ndependencies:\n  debug:\n    specifier: ^4.3.4\n    version: 4.3.4\n\ndevDependencies:\n  jest:\n    specifier: ^29.0.0\n    version: 29.7.0\n\n\
packages:\n\n  /debug@4.3.4:\n    resolution: {integrity: sha512-a}\n    dependencies:\n      ms: 2.1.2\n    dev: false\n\n  /ms@2.1.2:\n    resolution: {integrity: sha512-b}\n    dev: false\n\n  /jest@29.7.0:\n    resolution: {integrity: sha512-c}\n    dependencies:\n      ms: 2.1.2\n    dev: true\n";
        let (list, _) = parse(v6).unwrap();
        assert_eq!(find(&list, "npm:debug@4.3.4").dependencies, vec!["npm:ms@2.1.2"]);
        assert_eq!(find(&list, "npm:jest@29.7.0").scope.as_deref(), Some("dev"));
        // jest からも使われるが、実行時の依存（debug）からも辿れるので開発用ではない
        assert_eq!(find(&list, "npm:ms@2.1.2").scope, None);

        let v5 = "lockfileVersion: 5.4\n\nspecifiers:\n  '@scope/a': ^1.0.0\n\ndependencies:\n  '@scope/a': 1.0.0_react@18.2.0\n\n\
packages:\n\n  /@scope/a/1.0.0_react@18.2.0:\n    resolution: {integrity: sha512-a}\n    dependencies:\n      b: 2.0.0\n    dev: false\n\n  /b/2.0.0:\n    resolution: {integrity: sha512-b}\n    dev: false\n";
        let (list, _) = parse(v5).unwrap();
        let a = find(&list, "npm:@scope/a@1.0.0");
        assert_eq!((a.direct, a.dependencies.clone()), (Some(true), vec!["npm:b@2.0.0".to_string()]));
    }

    #[test]
    fn skips_the_package_manager_document() {
        let lock = "---
lockfileVersion: '9.0'

importers:

  .:
    configDependencies: {}
    packageManagerDependencies:
      pnpm:
        specifier: 10.0.0
        version: 10.0.0

packages:

  pnpm@10.0.0:
    resolution: {integrity: sha512-a}

snapshots:

  pnpm@10.0.0: {}

---
lockfileVersion: '9.0'

importers:

  .:
    dependencies:
      ms:
        specifier: ^2.1.3
        version: 2.1.3

packages:

  ms@2.1.3:
    resolution: {integrity: sha512-b}

snapshots:

  ms@2.1.3: {}
";
        let (list, warnings) = parse(lock).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id.as_str(), list[0].direct), ("npm:ms@2.1.3", Some(true)));
    }

    #[test]
    fn resolves_aliases_and_links() {
        assert_eq!(resolve_ref("x", "/real@1.0.0", false), Some(("real".into(), "1.0.0".into())));
        assert_eq!(resolve_ref("x", "real@1.0.0(peer@2)", false), Some(("real".into(), "1.0.0".into())));
        assert_eq!(resolve_ref("x", "1.0.0(peer@2)", false), Some(("x".into(), "1.0.0".into())));
        assert_eq!(resolve_ref("x", "link:../x", false), None);
        assert_eq!(parse_key("/@scope/a/1.0.0_react@18.2.0", true), Some(("@scope/a".into(), "1.0.0".into())));
    }
}

//! Yarn の yarn.lock の取り込み（Yarn 1 の独自形式と、Yarn 2 以降の YAML 形式）。
//!
//! どちらも「依存の指定（`name@range`）の並び → 解決した版」の記録なので、
//! 依存先は `name@range` をそのまま引けば決まる。
//! - Yarn 2 以降はワークスペース（`@workspace:`）の記録の依存を直接依存とする。
//! - Yarn 1 には直接依存の記録が無いので、同じ場所の package.json から判定する。
//! - 開発用かどうかは package.json の devDependencies から辿って判定する。

use super::npm::{npm_component, package_json_roots};
use super::yaml::{self, Node};
use super::{apply_roots, Importer};
use crate::model::{Component, ImportResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub struct YarnLockImporter;

impl Importer for YarnLockImporter {
    fn format_id(&self) -> &'static str {
        "yarn-lock"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some("yarn.lock")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let package_json = path.parent().and_then(|d| std::fs::read_to_string(d.join("package.json")).ok());
        let (components, warnings) = parse(content, package_json.as_deref())?;
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

/// 1 つの解決結果。
#[derive(Default, Debug)]
struct Entry {
    /// この記録に当たる依存の指定（`name@range`）
    descriptors: Vec<String>,
    name: String,
    version: Option<String>,
    /// 依存先（名前、範囲）
    deps: Vec<(String, String)>,
    /// ワークスペース（自分たちのプロジェクト）か
    workspace: bool,
}

/// `name@range` から名前（`@scope/name` の `@` は区切りではない）。
fn descriptor_name(descriptor: &str) -> &str {
    match descriptor[1..].find('@') {
        Some(i) => &descriptor[..i + 1],
        None => descriptor,
    }
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// Yarn 1 の形式。
fn parse_v1(content: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut in_deps = false;
    for line in content.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let text = line.trim();
        match indent {
            0 => {
                entries.extend(current.take());
                let header = text.trim_end_matches(':');
                let descriptors: Vec<String> = header.split(", ").map(|d| unquote(d).to_string()).collect();
                let name = descriptors.first().map(|d| descriptor_name(d).to_string()).unwrap_or_default();
                current = Some(Entry { descriptors, name, ..Default::default() });
                in_deps = false;
            }
            2 => {
                in_deps = matches!(text, "dependencies:" | "optionalDependencies:");
                if let (Some(e), Some(v)) = (current.as_mut(), text.strip_prefix("version ")) {
                    e.version = Some(unquote(v).to_string());
                }
            }
            _ if in_deps => {
                // `"@babel/highlight" "^7.12.13"` / `lodash "^4.17.21"`
                let (name, range) = match text.strip_prefix('"') {
                    Some(rest) => match rest.split_once('"') {
                        Some((n, r)) => (n.to_string(), unquote(r).to_string()),
                        None => continue,
                    },
                    None => match text.split_once(' ') {
                        Some((n, r)) => (n.to_string(), unquote(r).to_string()),
                        None => continue,
                    },
                };
                if let Some(e) = current.as_mut() {
                    e.deps.push((name, range));
                }
            }
            _ => {}
        }
    }
    entries.extend(current);
    entries
}

/// Yarn 2 以降（YAML）の形式。
fn parse_berry(content: &str) -> Vec<Entry> {
    let doc = yaml::parse(content);
    doc.entries()
        .iter()
        .filter(|(key, _)| key != "__metadata")
        .map(|(key, node)| {
            let descriptors: Vec<String> = key.split(", ").map(|d| d.trim().to_string()).collect();
            let resolution = node.get("resolution").and_then(Node::as_str).unwrap_or_default();
            let name = descriptors.first().map(|d| descriptor_name(d).to_string()).unwrap_or_default();
            // レジストリの版は `name@npm:1.2.3`。それ以外（workspace:、patch:、git など）は照会しない
            let from_registry = resolution.contains("@npm:") && !resolution.contains("patch:");
            let version = node.get("version").and_then(Node::as_str).filter(|_| from_registry).map(str::to_string);
            let deps = ["dependencies", "optionalDependencies"]
                .iter()
                .filter_map(|s| node.get(s))
                .flat_map(|n| n.entries().iter().filter_map(|(dep, range)| Some((dep.clone(), range.as_str()?.to_string()))))
                .collect();
            Entry { descriptors, name, version, deps, workspace: resolution.contains("@workspace:") }
        })
        .collect()
}

fn parse(content: &str, package_json: Option<&str>) -> Result<(Vec<Component>, Vec<String>), String> {
    let berry = content.contains("__metadata:");
    let entries = if berry { parse_berry(content) } else { parse_v1(content) };
    if entries.is_empty() {
        return Err(tr!("yarn.lock に記録がありません", "yarn.lock has no entries"));
    }
    let mut warnings = Vec::new();

    // 依存の指定 → 部品 ID
    let mut by_descriptor: HashMap<&str, String> = HashMap::new();
    let mut components: BTreeMap<String, Component> = BTreeMap::new();
    for e in entries.iter().filter(|e| !e.workspace) {
        let mut c = npm_component(&e.name, e.version.as_deref());
        if c.version.is_none() {
            // 版が無い（レジストリ以外）ものは、指定ごとに別の部品にする
            c.id = format!("npm:{}", e.descriptors.first().cloned().unwrap_or_default());
            c.notes.push(tr!(
                "レジストリ以外から入れたもののため、チェック対象外です。",
                "Installed from outside the registry, so it is not checked."
            ));
        }
        for d in &e.descriptors {
            by_descriptor.insert(d.as_str(), c.id.clone());
        }
        components.entry(c.id.clone()).or_insert(c);
    }
    let lookup = |name: &str, range: &str| -> Option<String> {
        by_descriptor
            .get(format!("{name}@{range}").as_str())
            // Yarn 2 以降は範囲に `npm:` が付く（依存先の書き方では省かれることもある）
            .or_else(|| by_descriptor.get(format!("{name}@npm:{range}").as_str()))
            .cloned()
    };

    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let (mut prod, mut dev) = (BTreeSet::new(), BTreeSet::new());
    for e in &entries {
        let targets: BTreeSet<String> = e.deps.iter().filter_map(|(n, r)| lookup(n, r)).collect();
        if e.workspace {
            prod.extend(targets);
        } else if let Some(id) = e.descriptors.first().and_then(|d| by_descriptor.get(d.as_str())) {
            edges.entry(id.clone()).or_default().extend(targets);
        }
    }

    // package.json があれば、直接依存（Yarn 1）と開発用（両方）をそこから決める
    let roots = package_json.and_then(package_json_roots);
    if let Some((pj_prod, pj_dev)) = &roots {
        // 範囲が分からないので、その名前のすべての版を直接依存とみなす
        let by_name = |names: &[String]| -> BTreeSet<String> {
            let wanted: BTreeSet<&str> = names.iter().map(String::as_str).collect();
            entries
                .iter()
                .filter(|e| !e.workspace && wanted.contains(e.name.as_str()))
                .filter_map(|e| e.descriptors.first().and_then(|d| by_descriptor.get(d.as_str())).cloned())
                .collect()
        };
        let dev_ids = by_name(pj_dev);
        if berry {
            // ワークスペースの依存には開発用も含まれるので、package.json の開発用を除く
            prod.retain(|id| !dev_ids.contains(id) || by_name(pj_prod).contains(id));
        } else {
            prod = by_name(pj_prod);
        }
        dev = dev_ids;
    }

    let mut list: Vec<Component> = components.into_values().collect();
    for c in &mut list {
        c.dependencies = edges.remove(&c.id).map(|s| s.into_iter().filter(|d| d != &c.id).collect()).unwrap_or_default();
    }
    if prod.is_empty() && dev.is_empty() {
        warnings.push(tr!(
            "直接依存の記録が無く、同じ場所に package.json も無いため、直接依存か推移的依存かを判定できません。",
            "There is no record of direct dependencies and no package.json next to it, so direct and transitive dependencies cannot be told apart."
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

    const V1: &str = "# THIS IS AN AUTOGENERATED FILE. DO NOT EDIT THIS FILE DIRECTLY.\n# yarn lockfile v1\n\n\n\
\"@babel/code-frame@^7.0.0\":\n  version \"7.12.13\"\n  resolved \"https://registry.yarnpkg.com/@babel/code-frame/-/code-frame-7.12.13.tgz\"\n  integrity sha512-a\n  dependencies:\n    \"@babel/highlight\" \"^7.12.13\"\n\n\
\"@babel/highlight@^7.12.13\":\n  version \"7.13.10\"\n\n\
lodash@^4.17.20, lodash@^4.17.21:\n  version \"4.17.21\"\n\n\
jest@^29.0.0:\n  version \"29.7.0\"\n  dependencies:\n    lodash \"^4.17.21\"\n";

    #[test]
    fn reads_yarn_v1_with_package_json() {
        let pj = r#"{ "dependencies": { "@babel/code-frame": "^7.0.0", "lodash": "^4.17.20" }, "devDependencies": { "jest": "^29.0.0" } }"#;
        let (list, warnings) = parse(V1, Some(pj)).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(list.len(), 4);
        let cf = find(&list, "npm:@babel/code-frame@7.12.13");
        assert_eq!(cf.direct, Some(true));
        assert_eq!(cf.dependencies, vec!["npm:@babel/highlight@7.13.10"]);
        assert_eq!(find(&list, "npm:@babel/highlight@7.13.10").direct, Some(false));
        assert_eq!(find(&list, "npm:jest@29.7.0").scope.as_deref(), Some("dev"));
        assert_eq!(find(&list, "npm:jest@29.7.0").dependencies, vec!["npm:lodash@4.17.21"]);
        assert_eq!(find(&list, "npm:lodash@4.17.21").scope, None);

        let (list, warnings) = parse(V1, None).unwrap();
        assert!(list.iter().all(|c| c.direct.is_none()));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn reads_yarn_berry() {
        let lock = "# This file is generated by running \"yarn install\" inside your project.\n\n__metadata:\n  version: 8\n  cacheKey: 10c0\n\n\
\"app@workspace:.\":\n  version: 0.0.0-use.local\n  resolution: \"app@workspace:.\"\n  dependencies:\n    debug: \"npm:^4.3.4\"\n    typescript: \"npm:^5.0.0\"\n  languageName: unknown\n  linkType: soft\n\n\
\"debug@npm:^4.3.4\":\n  version: 4.3.4\n  resolution: \"debug@npm:4.3.4\"\n  dependencies:\n    ms: \"npm:2.1.2\"\n  peerDependenciesMeta:\n    supports-color:\n      optional: true\n  checksum: 10c0/abc\n  languageName: node\n  linkType: hard\n\n\
\"ms@npm:2.1.2\":\n  version: 2.1.2\n  resolution: \"ms@npm:2.1.2\"\n  languageName: node\n  linkType: hard\n\n\
\"typescript@npm:^5.0.0, typescript@patch:typescript@npm%3A^5.0.0#optional!builtin<compat/typescript>\":\n  version: 5.4.5\n  resolution: \"typescript@patch:typescript@npm%3A5.4.5#optional!builtin<compat/typescript>::version=5.4.5&hash=5adc0c\"\n  languageName: node\n  linkType: hard\n";
        let pj = r#"{ "dependencies": { "debug": "^4.3.4" }, "devDependencies": { "typescript": "^5.0.0" } }"#;
        let (list, warnings) = parse(lock, Some(pj)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let debug = find(&list, "npm:debug@4.3.4");
        assert_eq!((debug.direct, debug.scope.clone()), (Some(true), None));
        assert_eq!(debug.dependencies, vec!["npm:ms@2.1.2"]);
        let ts = list.iter().find(|c| c.name == "typescript").unwrap();
        assert!(ts.version.is_none(), "patch: は照会しない");
        assert_eq!((ts.direct, ts.scope.as_deref()), (Some(true), Some("dev")));
        assert!(list.iter().all(|c| c.name != "app"));

        // package.json が無くても、ワークスペースの記録から直接依存は分かる
        let (list, warnings) = parse(lock, None).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(find(&list, "npm:debug@4.3.4").direct, Some(true));
    }

    #[test]
    fn splits_descriptor_names() {
        assert_eq!(descriptor_name("@babel/core@^7.0.0"), "@babel/core");
        assert_eq!(descriptor_name("lodash@npm:^4.17.21"), "lodash");
    }
}

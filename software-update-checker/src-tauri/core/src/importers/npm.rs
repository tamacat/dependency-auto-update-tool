//! npm の package-lock.json / npm-shrinkwrap.json の取り込み（lockfileVersion 1〜3）。
//! 仕様: https://docs.npmjs.com/cli/configuring-npm/package-lock-json
//!
//! - v2・v3 は `packages`（`node_modules/...` のパスごとの記録）を読む。ルート（キー `""`）と
//!   ワークスペース（`node_modules` を含まないキー）の依存を直接依存とする。
//! - v1 は入れ子の `dependencies` を同じ形のパスに直して読む。v1 には直接依存の記録が無いので、
//!   同じ場所の package.json があればそこから判定する。
//! - 依存先は Node.js と同じく、自分の `node_modules` から親へ向かって探す。
//! - 同じパッケージの同じ版が複数の場所にあっても、1 つの部品にまとめる。

use super::Importer;
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub struct PackageLockImporter;

const FILE_NAMES: [&str; 2] = ["package-lock.json", "npm-shrinkwrap.json"];

impl Importer for PackageLockImporter {
    fn format_id(&self) -> &'static str {
        "npm-package-lock"
    }

    fn can_import(&self, path: &Path, content: &str) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let looks_like_lock = content.trim_start().starts_with('{') && content.contains("\"lockfileVersion\"");
        looks_like_lock && (FILE_NAMES.contains(&name) || !content.contains("\"bomFormat\""))
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let package_json = path.parent().and_then(|d| std::fs::read_to_string(d.join("package.json")).ok());
        let mut result = parse(content, package_json.as_deref())?;
        result.source_path = path.display().to_string();
        result.format = self.format_id().into();
        Ok(result)
    }
}

/// `node_modules/...` のパスごとの記録。
#[derive(Default, Debug)]
struct Entry {
    name: String,
    version: Option<String>,
    dev: bool,
    optional: bool,
    peer: bool,
    /// ワークスペースへのリンク（`resolved` が指すパス）
    link: Option<String>,
    license: Option<String>,
    /// 依存先のパッケージ名
    deps: Vec<String>,
}

/// パスの末尾（最後の `node_modules/` より後）からパッケージ名を取り出す。
fn name_from_path(path: &str) -> &str {
    match path.rfind("node_modules/") {
        Some(i) => &path[i + "node_modules/".len()..],
        None => path,
    }
}

fn keys(obj: &Value, field: &str) -> Vec<String> {
    obj.get(field).and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default()
}

fn flag(obj: &Value, field: &str) -> bool {
    obj.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn str_field(obj: &Value, field: &str) -> Option<String> {
    obj.get(field).and_then(Value::as_str).map(str::to_string)
}

/// v2・v3 の `packages` を読む。
fn entries_from_packages(packages: &Map<String, Value>) -> BTreeMap<String, Entry> {
    packages
        .iter()
        .map(|(path, obj)| {
            let is_root_or_workspace = !path.contains("node_modules/");
            let mut deps = keys(obj, "dependencies");
            deps.extend(keys(obj, "optionalDependencies"));
            deps.extend(keys(obj, "peerDependencies"));
            if is_root_or_workspace {
                deps.extend(keys(obj, "devDependencies"));
            }
            let entry = Entry {
                // 別名（"alias": "npm:real@1.0.0"）で入れたものは name に本当の名前が入る
                name: str_field(obj, "name").unwrap_or_else(|| name_from_path(path).to_string()),
                version: str_field(obj, "version"),
                dev: flag(obj, "dev") || flag(obj, "devOptional"),
                optional: flag(obj, "optional"),
                peer: flag(obj, "peer"),
                link: flag(obj, "link").then(|| str_field(obj, "resolved")).flatten(),
                license: str_field(obj, "license"),
                deps,
            };
            (path.clone(), entry)
        })
        .collect()
}

/// v1 の入れ子の `dependencies` を、v2 と同じパスの形に直す。
fn entries_from_v1(deps: &Map<String, Value>, parent: &str, out: &mut BTreeMap<String, Entry>) {
    for (name, obj) in deps {
        let path = if parent.is_empty() { format!("node_modules/{name}") } else { format!("{parent}/node_modules/{name}") };
        // 別名は "version": "npm:real-name@1.0.0"
        let raw_version = str_field(obj, "version");
        let (real_name, version) = match raw_version.as_deref().and_then(|v| v.strip_prefix("npm:")) {
            Some(alias) => match alias.rfind('@').filter(|&i| i > 0) {
                Some(i) => (alias[..i].to_string(), Some(alias[i + 1..].to_string())),
                None => (alias.to_string(), None),
            },
            None => (name.clone(), raw_version),
        };
        out.insert(
            path.clone(),
            Entry {
                name: real_name,
                version,
                dev: flag(obj, "dev"),
                optional: flag(obj, "optional"),
                deps: keys(obj, "requires"),
                ..Default::default()
            },
        );
        if let Some(children) = obj.get("dependencies").and_then(Value::as_object) {
            entries_from_v1(children, &path, out);
        }
    }
}

/// `from` の場所から見た依存先 `name` のパス（Node.js の探し方）。
fn resolve(entries: &BTreeMap<String, Entry>, from: &str, name: &str) -> Option<String> {
    let mut base = from.to_string();
    loop {
        let candidate =
            if base.is_empty() { format!("node_modules/{name}") } else { format!("{base}/node_modules/{name}") };
        if entries.contains_key(&candidate) {
            return Some(candidate);
        }
        if base.is_empty() {
            return None;
        }
        base = match base.rfind("node_modules/") {
            Some(i) => base[..i].trim_end_matches('/').to_string(),
            None => String::new(),
        };
    }
}

/// レジストリの版として照会できる値か（`file:`・git の URL などは除く）。
fn registry_version(v: &str) -> bool {
    !v.is_empty() && !v.contains(':') && !v.contains('/')
}

fn split_scope(name: &str) -> (Option<&str>, &str) {
    match name.strip_prefix('@').and_then(|_| name.split_once('/')) {
        Some((scope, rest)) => (Some(scope), rest),
        None => (None, name),
    }
}

/// npm の部品（ID は `npm:名前@版`）。版がレジストリの版でなければ照会しない印を付ける。
pub(super) fn npm_component(name: &str, version: Option<&str>) -> Component {
    let version = version.filter(|v| registry_version(v));
    let (scope, short) = split_scope(name);
    let id = match version {
        Some(v) => format!("npm:{name}@{v}"),
        None => format!("npm:{name}"),
    };
    let mut c = Component::new(id, Ecosystem::Npm, short);
    c.group = scope.map(str::to_string);
    c.version = version.map(str::to_string);
    c.purl = version.map(|v| Purl::build("npm", scope, short, Some(v)).to_string_canonical(true));
    c
}

/// package.json の依存の名前（実行時に使うもの、開発用）。読めなければ `None`。
pub(super) fn package_json_roots(content: &str) -> Option<(Vec<String>, Vec<String>)> {
    let pj: Value = serde_json::from_str(content).ok()?;
    let mut prod = keys(&pj, "dependencies");
    prod.extend(keys(&pj, "optionalDependencies"));
    prod.extend(keys(&pj, "peerDependencies"));
    Some((prod, keys(&pj, "devDependencies")))
}

/// package-lock.json の中身（と、あれば同じ場所の package.json）から取り込む。
fn parse(content: &str, package_json: Option<&str>) -> Result<ImportResult, String> {
    let json: Value =
        serde_json::from_str(content).map_err(|e| tr!("package-lock.json が不正です: {e}", "Invalid package-lock.json: {e}"))?;
    let lock_version = json.get("lockfileVersion").and_then(Value::as_u64).unwrap_or(1);
    let mut warnings = Vec::new();

    let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
    match json.get("packages").and_then(Value::as_object) {
        Some(packages) if lock_version >= 2 => entries = entries_from_packages(packages),
        _ => {
            if let Some(deps) = json.get("dependencies").and_then(Value::as_object) {
                entries_from_v1(deps, "", &mut entries);
            }
            // v1 にはルートの依存の記録が無いので package.json から補う
            if let Some(pj) = package_json.and_then(|s| serde_json::from_str::<Value>(s).ok()) {
                let mut deps = keys(&pj, "dependencies");
                deps.extend(keys(&pj, "optionalDependencies"));
                deps.extend(keys(&pj, "peerDependencies"));
                deps.extend(keys(&pj, "devDependencies"));
                entries.insert(String::new(), Entry { name: str_field(&pj, "name").unwrap_or_default(), deps, ..Default::default() });
            } else {
                warnings.push(tr!(
                    "lockfileVersion 1 で、同じ場所に package.json が無いため、直接依存か推移的依存かを判定できません。",
                    "This is lockfileVersion 1 and there is no package.json next to it, so direct and transitive dependencies cannot be told apart."
                ));
            }
        }
    }

    let project_name = entries.get("").map(|root| {
        let name = if root.name.is_empty() { str_field(&json, "name").unwrap_or_default() } else { root.name.clone() };
        match root.version.as_deref().or(json.get("version").and_then(Value::as_str)) {
            Some(v) => format!("{name}@{v}"),
            None => name,
        }
    });

    // ルートとワークスペース（自分たちのコード）は部品にしない
    let is_own = |path: &str, e: &Entry| !path.contains("node_modules/") || e.link.is_some();

    // パス → 部品 ID（同じ名前・版はまとめる）
    let mut id_of: HashMap<&str, String> = HashMap::new();
    let mut components: BTreeMap<String, Component> = BTreeMap::new();
    // 部品ごとの、すべての出現が dev / optional / peer か
    let mut all_dev: HashMap<String, bool> = HashMap::new();
    let mut all_optional: HashMap<String, bool> = HashMap::new();
    let mut all_peer: HashMap<String, bool> = HashMap::new();
    let mut not_registry = 0;
    for (path, e) in &entries {
        if is_own(path, e) {
            continue;
        }
        let version = e.version.clone().filter(|v| registry_version(v));
        let id = match &version {
            Some(v) => format!("npm:{}@{v}", e.name),
            None => format!("npm:{}@{}", e.name, path),
        };
        id_of.insert(path.as_str(), id.clone());
        *all_dev.entry(id.clone()).or_insert(true) &= e.dev;
        *all_optional.entry(id.clone()).or_insert(true) &= e.optional;
        *all_peer.entry(id.clone()).or_insert(true) &= e.peer;
        components.entry(id.clone()).or_insert_with(|| {
            let (scope, name) = split_scope(&e.name);
            let mut c = Component::new(id.clone(), Ecosystem::Npm, name);
            c.group = scope.map(str::to_string);
            c.version = version.clone();
            if version.is_some() {
                c.purl = Some(Purl::build("npm", scope, name, version.as_deref()).to_string_canonical(true));
            } else {
                not_registry += 1;
                c.notes.push(tr!(
                    "レジストリ以外から入れたもの（{}）のため、チェック対象外です。",
                    "Installed from outside the registry ({}), so it is not checked.",
                    e.version.as_deref().unwrap_or("-")
                ));
            }
            c.licenses.extend(e.license.clone());
            c
        });
    }

    // 依存関係（パスごとに解決し、部品 ID でまとめる）
    let mut edges: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut root_children: BTreeSet<String> = BTreeSet::new();
    for (path, e) in &entries {
        // ワークスペースへのリンクは、リンク先の依存として扱う
        let from = e.link.as_deref().unwrap_or(path);
        for dep in &e.deps {
            let Some(target) = resolve(&entries, from, dep) else { continue };
            let target_entry = &entries[&target];
            let Some(target_id) = id_of.get(target.as_str()) else { continue };
            if target_entry.link.is_some() {
                continue;
            }
            if is_own(path, e) {
                root_children.insert(target_id.clone());
            } else if let Some(id) = id_of.get(path.as_str()) {
                edges.entry(id.clone()).or_default().insert(target_id.clone());
            }
        }
    }

    let has_root = entries.contains_key("");
    let mut components: Vec<Component> = components.into_values().collect();
    for c in &mut components {
        c.dependencies = edges.remove(&c.id).map(|s| s.into_iter().collect()).unwrap_or_default();
        if has_root {
            c.direct = Some(root_children.contains(&c.id));
        }
        c.scope = if all_dev[&c.id] {
            Some("dev".into())
        } else if all_optional[&c.id] {
            Some("optional".into())
        } else if all_peer[&c.id] {
            Some("peer".into())
        } else {
            None
        };
    }
    if not_registry > 0 {
        warnings.push(tr!(
            "{not_registry} 件はレジストリ以外（ローカルのパス・git など）から入れたもので、チェック対象外です。",
            "{not_registry} packages were installed from outside the registry (local paths, git, etc.) and are not checked."
        ));
    }

    Ok(ImportResult {
        source_path: String::new(),
        format: String::new(),
        project_name,
        components,
        warnings,
        includes_transitive: true,
        related_files: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(r: &'a ImportResult, id: &str) -> &'a Component {
        r.components.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} がありません"))
    }

    #[test]
    fn reads_lockfile_v3() {
        let lock = r#"{
          "name": "app", "version": "1.0.0", "lockfileVersion": 3,
          "packages": {
            "": { "name": "app", "version": "1.0.0",
                  "dependencies": { "express": "^4.17.0", "@scope/util": "^1.0.0" },
                  "devDependencies": { "jest": "^29.0.0" } },
            "node_modules/express": { "version": "4.17.1", "license": "MIT",
                  "dependencies": { "debug": "2.6.9", "ms": "^2.0.0" } },
            "node_modules/express/node_modules/ms": { "version": "2.0.0" },
            "node_modules/debug": { "version": "2.6.9", "dependencies": { "ms": "2.1.3" } },
            "node_modules/ms": { "version": "2.1.3" },
            "node_modules/@scope/util": { "version": "1.2.0" },
            "node_modules/jest": { "version": "29.7.0", "dev": true, "dependencies": { "ms": "^2.1.0" } },
            "node_modules/local-thing": { "version": "file:../local-thing" }
          }
        }"#;
        let r = parse(lock, None).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("app@1.0.0"));

        let express = find(&r, "npm:express@4.17.1");
        assert_eq!(express.direct, Some(true));
        assert_eq!(express.scope, None);
        assert_eq!(express.licenses, vec!["MIT"]);
        assert_eq!(express.purl.as_deref(), Some("pkg:npm/express@4.17.1"));
        // express の中の ms は入れ子の 2.0.0、debug の ms は上の 2.1.3
        assert_eq!(express.dependencies, vec!["npm:debug@2.6.9", "npm:ms@2.0.0"]);
        assert_eq!(find(&r, "npm:debug@2.6.9").dependencies, vec!["npm:ms@2.1.3"]);
        assert_eq!(find(&r, "npm:ms@2.1.3").direct, Some(false));

        let util = find(&r, "npm:@scope/util@1.2.0");
        assert_eq!(util.group.as_deref(), Some("@scope"));
        assert_eq!(util.name, "util");
        assert_eq!(util.purl.as_deref(), Some("pkg:npm/%40scope/util@1.2.0"));

        let jest = find(&r, "npm:jest@29.7.0");
        assert_eq!(jest.direct, Some(true));
        assert_eq!(jest.scope.as_deref(), Some("dev"));

        let local = r.components.iter().find(|c| c.name == "local-thing").unwrap();
        assert!(local.version.is_none() && local.purl.is_none());
        assert_eq!(r.warnings.len(), 1);
    }

    #[test]
    fn follows_workspace_links() {
        let lock = r#"{
          "lockfileVersion": 3,
          "packages": {
            "": { "name": "mono", "workspaces": ["packages/*"] },
            "packages/a": { "name": "a", "version": "0.1.0", "dependencies": { "left-pad": "^1.3.0" } },
            "node_modules/a": { "resolved": "packages/a", "link": true },
            "node_modules/left-pad": { "version": "1.3.0" }
          }
        }"#;
        let r = parse(lock, None).unwrap();
        assert_eq!(r.components.len(), 1);
        assert_eq!(r.components[0].id, "npm:left-pad@1.3.0");
        assert_eq!(r.components[0].direct, Some(true));
    }

    #[test]
    fn reads_lockfile_v1_with_package_json() {
        let lock = r#"{
          "name": "old", "version": "0.0.1", "lockfileVersion": 1, "requires": true,
          "dependencies": {
            "a": { "version": "1.0.0", "requires": { "b": "^2.0.0" } },
            "b": { "version": "2.1.0" },
            "c": { "version": "3.0.0", "dev": true, "requires": { "b": "^1.0.0" },
                   "dependencies": { "b": { "version": "1.5.0", "dev": true } } },
            "alias": { "version": "npm:real-pkg@4.0.0" }
          }
        }"#;
        let pj = r#"{ "name": "old", "dependencies": { "a": "^1.0.0", "alias": "npm:real-pkg@^4" }, "devDependencies": { "c": "^3" } }"#;
        let r = parse(lock, Some(pj)).unwrap();
        assert_eq!(find(&r, "npm:a@1.0.0").dependencies, vec!["npm:b@2.1.0"]);
        assert_eq!(find(&r, "npm:a@1.0.0").direct, Some(true));
        assert_eq!(find(&r, "npm:b@2.1.0").direct, Some(false));
        assert_eq!(find(&r, "npm:c@3.0.0").dependencies, vec!["npm:b@1.5.0"]);
        assert_eq!(find(&r, "npm:b@1.5.0").scope.as_deref(), Some("dev"));
        assert_eq!(find(&r, "npm:real-pkg@4.0.0").direct, Some(true));

        // package.json が無ければ直接依存は判定しない
        let r = parse(lock, None).unwrap();
        assert!(r.components.iter().all(|c| c.direct.is_none()));
        assert_eq!(r.warnings.len(), 1);
    }
}

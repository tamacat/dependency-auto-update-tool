//! Python の依存ファイルの取り込み。
//!
//! - requirements.txt（`==` で版を固定したもの）。pip-compile が付ける `# via` の注記があれば、
//!   そこから依存関係と直接依存を判定する。
//! - poetry.lock: 直接依存は同じ場所の pyproject.toml から判定する。
//! - uv.lock: ワークスペースの部品（`source` が editable / virtual のもの）の依存を直接依存とする。
//!
//! パッケージ名は PEP 503 の正規形（小文字、`-` `_` `.` の連続を `-` に）にそろえる。

use super::Importer;
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// PEP 503 の正規化。
pub(crate) fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_sep = false;
    for ch in name.trim().chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !last_sep {
                out.push('-');
            }
            last_sep = true;
        } else {
            out.push(ch.to_ascii_lowercase());
            last_sep = false;
        }
    }
    out
}

fn pypi_component(name: &str, version: Option<&str>) -> Component {
    let name = normalize(name);
    let id = match version {
        Some(v) => format!("pypi:{name}@{v}"),
        None => format!("pypi:{name}"),
    };
    let mut c = Component::new(id, Ecosystem::Pypi, name.clone());
    c.version = version.map(str::to_string);
    if version.is_some() {
        c.purl = Some(Purl::build("pypi", None, &name, version).to_string_canonical(true));
    }
    c
}

/// PEP 508 の依存指定（`requests[socks]>=2.0; python_version<"3.8"` など）から名前を取り出す。
fn requirement_name(spec: &str) -> Option<String> {
    let name: String =
        spec.trim().chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')).collect();
    (!name.is_empty()).then(|| normalize(&name))
}

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
}

fn result(format: &str, path: &Path, project_name: Option<String>, components: Vec<Component>, warnings: Vec<String>, transitive: bool) -> ImportResult {
    ImportResult {
        source_path: path.display().to_string(),
        format: format.into(),
        project_name,
        components,
        warnings,
        includes_transitive: transitive,
        related_files: Vec::new(),
    }
}

// ---------------------------------------------------------------- requirements.txt

pub struct RequirementsImporter;

impl Importer for RequirementsImporter {
    fn format_id(&self) -> &'static str {
        "pip-requirements"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        let name = file_name(path).to_ascii_lowercase();
        name.ends_with(".txt") && (name.contains("requirements") || name.contains("constraints"))
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let (components, warnings, transitive) = parse_requirements(content);
        Ok(result(self.format_id(), path, None, components, warnings, transitive))
    }
}

/// requirements.txt を読む。戻り値は（部品、警告、推移的依存を含むか）。
fn parse_requirements(content: &str) -> (Vec<Component>, Vec<String>, bool) {
    // 行末の `\` による継続をつなぐ（コメント行はそのまま）
    let mut lines: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in content.lines() {
        if let Some(head) = raw.strip_suffix('\\') {
            pending.push_str(head);
            pending.push(' ');
            continue;
        }
        pending.push_str(raw);
        lines.push(std::mem::take(&mut pending));
    }
    if !pending.is_empty() {
        lines.push(pending);
    }

    let mut components: Vec<Component> = Vec::new();
    // 部品の位置 → `# via` に書かれた依存元
    let mut via: HashMap<usize, Vec<String>> = HashMap::new();
    let mut current: Option<usize> = None;
    let mut in_via = false;
    let mut skipped: Vec<String> = Vec::new();
    let mut includes: Vec<String> = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if let Some(comment) = trimmed.strip_prefix('#') {
            let comment = comment.trim();
            if let Some(i) = current {
                if let Some(rest) = comment.strip_prefix("via") {
                    in_via = true;
                    if !rest.trim().is_empty() {
                        via.entry(i).or_default().push(rest.trim().to_string());
                    }
                } else if in_via && line.starts_with(char::is_whitespace) && !comment.is_empty() {
                    via.entry(i).or_default().push(comment.to_string());
                } else {
                    in_via = false;
                }
            }
            continue;
        }
        in_via = false;
        if trimmed.is_empty() {
            continue;
        }
        current = None;
        if trimmed.starts_with('-') {
            let option = trimmed.split_whitespace().next().unwrap_or_default();
            if matches!(option, "-r" | "--requirement" | "-c" | "--constraint") {
                includes.push(trimmed.to_string());
            } else if matches!(option, "-e" | "--editable") {
                skipped.push(trimmed.to_string());
            }
            continue;
        }
        // 行内のコメント・ハッシュなどのオプション・環境マーカーを落とす
        let spec = trimmed.split(" #").next().unwrap_or_default();
        let spec = spec.split(" --").next().unwrap_or_default();
        let spec = spec.split(';').next().unwrap_or_default().trim();
        // `name @ https://...` や URL だけの行はレジストリの版ではない
        if spec.contains("://") || spec.contains(" @ ") {
            skipped.push(spec.to_string());
            continue;
        }
        let Some(name) = requirement_name(spec) else { continue };
        // extras（`[socks]`）の後ろが版の指定
        let after_name = spec.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        let constraint = match after_name.trim_start().strip_prefix('[') {
            Some(rest) => rest.split_once(']').map(|(_, r)| r).unwrap_or_default(),
            None => after_name,
        }
        .trim();
        let pinned = constraint
            .strip_prefix("===")
            .or_else(|| constraint.strip_prefix("=="))
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.contains([',', '*']));

        let mut c = pypi_component(&name, pinned);
        if pinned.is_none() {
            c.notes.push(if constraint.is_empty() {
                tr!("版が指定されていません。", "No version is specified.")
            } else {
                tr!("版が固定されていません（{constraint}）。", "The version is not pinned ({constraint}).")
            });
        }
        if let Some(i) = components.iter().position(|x| x.id == c.id) {
            current = Some(i);
            continue;
        }
        components.push(c);
        current = Some(components.len() - 1);
    }

    // pip-compile の `# via` から依存関係を組み立てる
    let has_via = !via.is_empty();
    if has_via {
        let index: HashMap<String, usize> = components.iter().enumerate().map(|(i, c)| (c.name.clone(), i)).collect();
        let mut edges: Vec<(usize, String)> = Vec::new();
        for (i, c) in components.iter_mut().enumerate() {
            let parents = via.get(&i).cloned().unwrap_or_default();
            c.direct = Some(parents.iter().any(|p| p.starts_with("-r") || p.starts_with("--requirement")));
            for p in parents.iter().filter(|p| !p.starts_with('-')) {
                if let Some(&parent) = requirement_name(p).and_then(|n| index.get(&n)) {
                    edges.push((parent, c.id.clone()));
                }
            }
        }
        for (parent, child) in edges {
            if !components[parent].dependencies.contains(&child) {
                components[parent].dependencies.push(child);
            }
        }
    }

    let mut warnings = Vec::new();
    let unpinned = components.iter().filter(|c| c.version.is_none()).count();
    if unpinned > 0 {
        warnings.push(tr!(
            "{unpinned} 件は版が `==` で固定されていないため、最新版の確認だけを行います（脆弱性は照会しません）。",
            "{unpinned} packages are not pinned with `==`, so only the latest version is checked (no vulnerability lookup)."
        ));
    }
    if !has_via {
        warnings.push(tr!(
            "依存関係の情報（pip-compile の `# via`）が無いため、直接依存か推移的依存かを判定できません。",
            "There is no dependency information (pip-compile's `# via`), so direct and transitive dependencies cannot be told apart."
        ));
    }
    if !includes.is_empty() {
        warnings.push(tr!(
            "別のファイルの読み込みは辿っていません: {}",
            "Included files are not followed: {}",
            includes.join(", ")
        ));
    }
    if !skipped.is_empty() {
        warnings.push(tr!(
            "レジストリ以外（ローカルのパス・URL・git）を指す {} 件は取り込んでいません。",
            "{} entries that point outside the registry (local paths, URLs, git) were not imported.",
            skipped.len()
        ));
    }
    (components, warnings, has_via)
}

// ---------------------------------------------------------------- pyproject.toml

/// pyproject.toml に書かれた直接依存の名前と、プロジェクト名。
#[derive(Default)]
struct PyProject {
    name: Option<String>,
    direct: HashSet<String>,
}

fn read_pyproject(content: &str) -> Option<PyProject> {
    let doc: toml::Table = toml::from_str(content).ok()?;
    let mut p = PyProject::default();
    let table = |v: Option<&toml::Value>| v.and_then(toml::Value::as_table).cloned();
    let strings = |v: &toml::Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().and_then(requirement_name)).collect()).unwrap_or_default()
    };

    // PEP 621（[project]）と PEP 735（[dependency-groups]）
    if let Some(project) = table(doc.get("project")) {
        p.name = project.get("name").and_then(toml::Value::as_str).map(str::to_string);
        if let Some(deps) = project.get("dependencies") {
            p.direct.extend(strings(deps));
        }
        for list in table(project.get("optional-dependencies")).into_iter().flat_map(|t| t.into_iter().map(|(_, v)| v)) {
            p.direct.extend(strings(&list));
        }
    }
    for list in table(doc.get("dependency-groups")).into_iter().flat_map(|t| t.into_iter().map(|(_, v)| v)) {
        p.direct.extend(strings(&list));
    }

    // Poetry（[tool.poetry]）
    if let Some(poetry) = table(doc.get("tool").and_then(|t| t.get("poetry"))) {
        if p.name.is_none() {
            p.name = poetry.get("name").and_then(toml::Value::as_str).map(str::to_string);
        }
        let mut tables: Vec<toml::Table> = Vec::new();
        tables.extend(table(poetry.get("dependencies")));
        tables.extend(table(poetry.get("dev-dependencies")));
        for (_, group) in table(poetry.get("group")).unwrap_or_default() {
            tables.extend(table(group.get("dependencies")));
        }
        for t in tables {
            p.direct.extend(t.keys().filter(|k| k.as_str() != "python").map(|k| normalize(k)));
        }
    }
    Some(p)
}

// ---------------------------------------------------------------- poetry.lock

pub struct PoetryLockImporter;

#[derive(Deserialize)]
struct PoetryLock {
    #[serde(default)]
    package: Vec<PoetryPackage>,
}

#[derive(Deserialize)]
struct PoetryPackage {
    name: String,
    version: String,
    /// Poetry 1.x
    category: Option<String>,
    /// Poetry 2.x
    groups: Option<Vec<String>>,
    #[serde(default)]
    dependencies: toml::Table,
    source: Option<toml::Table>,
}

impl Importer for PoetryLockImporter {
    fn format_id(&self) -> &'static str {
        "poetry-lock"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        file_name(path) == "poetry.lock"
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let pyproject = path.parent().and_then(|d| std::fs::read_to_string(d.join("pyproject.toml")).ok());
        let (project_name, components, warnings) = parse_poetry(content, pyproject.as_deref())?;
        Ok(result(self.format_id(), path, project_name, components, warnings, true))
    }
}

/// （プロジェクト名、部品、警告）
type Parsed = (Option<String>, Vec<Component>, Vec<String>);

fn parse_poetry(content: &str, pyproject: Option<&str>) -> Result<Parsed, String> {
    let lock: PoetryLock =
        toml::from_str(content).map_err(|e| tr!("poetry.lock が不正です: {e}", "Invalid poetry.lock: {e}"))?;
    let project = pyproject.and_then(read_pyproject);
    let mut warnings = Vec::new();

    let mut components: Vec<Component> = Vec::new();
    for p in &lock.package {
        let mut c = pypi_component(&p.name, Some(&p.version));
        let groups = p.groups.clone().or_else(|| p.category.clone().map(|c| vec![c])).unwrap_or_default();
        if !groups.is_empty() && !groups.iter().any(|g| g == "main") {
            c.scope = Some(groups.join(","));
        }
        if let Some(kind) = p.source.as_ref().and_then(|s| s.get("type")).and_then(toml::Value::as_str) {
            if kind != "legacy" {
                c.notes.push(tr!("PyPI 以外から入れたものです（{kind}）。", "Installed from outside PyPI ({kind})."));
            }
        }
        components.push(c);
    }

    let by_name: HashMap<String, Vec<String>> = components.iter().fold(HashMap::new(), |mut m, c| {
        m.entry(c.name.clone()).or_default().push(c.id.clone());
        m
    });
    for (c, p) in components.iter_mut().zip(&lock.package) {
        let deps: BTreeSet<String> =
            p.dependencies.keys().filter_map(|k| by_name.get(&normalize(k))).flatten().cloned().collect();
        c.dependencies = deps.into_iter().collect();
    }

    match &project {
        Some(proj) => {
            for c in &mut components {
                c.direct = Some(proj.direct.contains(&c.name));
            }
        }
        None => warnings.push(tr!(
            "同じ場所に pyproject.toml が無いため、直接依存か推移的依存かを判定できません。",
            "There is no pyproject.toml next to it, so direct and transitive dependencies cannot be told apart."
        )),
    }
    Ok((project.and_then(|p| p.name), components, warnings))
}

// ---------------------------------------------------------------- uv.lock

pub struct UvLockImporter;

#[derive(Deserialize)]
struct UvLock {
    #[serde(default)]
    package: Vec<UvPackage>,
}

#[derive(Deserialize)]
struct UvPackage {
    name: String,
    version: Option<String>,
    #[serde(default)]
    source: toml::Table,
    #[serde(default)]
    dependencies: Vec<UvDep>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<UvDep>>,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: BTreeMap<String, Vec<UvDep>>,
}

#[derive(Deserialize)]
struct UvDep {
    name: String,
    /// 同じ名前の版が複数あるときだけ付く
    version: Option<String>,
}

impl Importer for UvLockImporter {
    fn format_id(&self) -> &'static str {
        "uv-lock"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        file_name(path) == "uv.lock"
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let (project_name, components, warnings) = parse_uv(content)?;
        Ok(result(self.format_id(), path, project_name, components, warnings, true))
    }
}

fn parse_uv(content: &str) -> Result<Parsed, String> {
    let lock: UvLock = toml::from_str(content).map_err(|e| tr!("uv.lock が不正です: {e}", "Invalid uv.lock: {e}"))?;
    let mut warnings = Vec::new();

    // ワークスペースの部品（自分たちのコード）
    let is_own = |p: &UvPackage| p.source.contains_key("editable") || p.source.contains_key("virtual");
    let project_name = lock
        .package
        .iter()
        .find(|p| is_own(p) && ["editable", "virtual"].iter().any(|k| p.source.get(*k).and_then(toml::Value::as_str) == Some(".")))
        .or_else(|| lock.package.iter().find(|p| is_own(p)))
        .map(|p| p.name.clone());

    let mut components: Vec<Component> = Vec::new();
    let mut index: HashMap<String, Vec<(Option<String>, String)>> = HashMap::new();
    let mut not_registry = 0;
    for p in lock.package.iter().filter(|p| !is_own(p)) {
        let from_registry = p.source.contains_key("registry");
        let version = p.version.as_deref().filter(|_| from_registry);
        let mut c = pypi_component(&p.name, version);
        if !from_registry {
            not_registry += 1;
            let kind = p.source.keys().next().cloned().unwrap_or_default();
            let ver = p.version.as_deref().unwrap_or("-");
            c.notes.push(tr!(
                "PyPI 以外から入れたもの（{kind}、版 {ver}）のため、脆弱性は照会しません。",
                "Installed from outside PyPI ({kind}, version {ver}), so vulnerabilities are not looked up."
            ));
        }
        index.entry(c.name.clone()).or_default().push((p.version.clone(), c.id.clone()));
        components.push(c);
    }
    let resolve = |d: &UvDep| -> Vec<String> {
        index
            .get(&normalize(&d.name))
            .map(|list| {
                list.iter().filter(|(v, _)| d.version.is_none() || v == &d.version).map(|(_, id)| id.clone()).collect()
            })
            .unwrap_or_default()
    };

    // 依存関係（optional-dependencies も含める）
    let mut edges: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut prod_roots: BTreeSet<String> = BTreeSet::new();
    let mut dev_roots: BTreeSet<String> = BTreeSet::new();
    for p in &lock.package {
        let runtime: Vec<&UvDep> = p.dependencies.iter().chain(p.optional_dependencies.values().flatten()).collect();
        if is_own(p) {
            prod_roots.extend(runtime.iter().flat_map(|d| resolve(d)));
            dev_roots.extend(p.dev_dependencies.values().flatten().flat_map(&resolve));
        } else if let Some(id) = index.get(&normalize(&p.name)).and_then(|l| l.iter().find(|(v, _)| v == &p.version)).map(|(_, id)| id.clone()) {
            edges.entry(id).or_default().extend(runtime.iter().flat_map(|d| resolve(d)));
        }
    }

    // 実行時の依存から辿れないものは開発用
    let mut reachable: HashSet<String> = HashSet::new();
    let mut stack: Vec<String> = prod_roots.iter().cloned().collect();
    while let Some(id) = stack.pop() {
        if reachable.insert(id.clone()) {
            stack.extend(edges.get(&id).into_iter().flatten().cloned());
        }
    }
    let has_roots = lock.package.iter().any(is_own);
    for c in &mut components {
        c.dependencies = edges.remove(&c.id).map(|s| s.into_iter().collect()).unwrap_or_default();
        if has_roots {
            c.direct = Some(prod_roots.contains(&c.id) || dev_roots.contains(&c.id));
            if !reachable.contains(&c.id) {
                c.scope = Some("dev".into());
            }
        }
    }
    if !has_roots {
        warnings.push(tr!(
            "uv.lock にプロジェクト自身の記録が無いため、直接依存か推移的依存かを判定できません。",
            "uv.lock has no entry for the project itself, so direct and transitive dependencies cannot be told apart."
        ));
    }
    if not_registry > 0 {
        warnings.push(tr!(
            "{not_registry} 件は PyPI 以外（ローカルのパス・git・URL）から入れたもので、チェック対象外です。",
            "{not_registry} packages were installed from outside PyPI (local paths, git, URLs) and are not checked."
        ));
    }
    Ok((project_name, components, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(list: &'a [Component], id: &str) -> &'a Component {
        list.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} がありません"))
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize("Flask_SQLAlchemy"), "flask-sqlalchemy");
        assert_eq!(normalize("zope.interface"), "zope-interface");
        assert_eq!(normalize("a--b__c"), "a-b-c");
        assert_eq!(requirement_name("requests[socks]>=2.0; python_version<'3.8'").as_deref(), Some("requests"));
    }

    #[test]
    fn reads_plain_requirements() {
        let txt = "# comment\n\
                   Django==4.2.7\n\
                   requests[socks] == 2.31.0 ; python_version >= \"3.8\"\n\
                   numpy>=1.24\n\
                   pytz\n\
                   cryptography==41.0.5 \\\n    --hash=sha256:abc \\\n    --hash=sha256:def\n\
                   -r base.txt\n\
                   -e ./local\n\
                   mypkg @ https://example.com/mypkg.whl\n";
        let (list, warnings, transitive) = parse_requirements(txt);
        assert!(!transitive);
        let ids: Vec<&str> = list.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["pypi:django@4.2.7", "pypi:requests@2.31.0", "pypi:numpy", "pypi:pytz", "pypi:cryptography@41.0.5"]);
        assert_eq!(list[0].purl.as_deref(), Some("pkg:pypi/django@4.2.7"));
        assert!(list[2].version.is_none() && list[2].notes[0].contains(">=1.24"));
        assert!(list.iter().all(|c| c.direct.is_none()));
        // 版の未固定・依存関係なし・-r・レジストリ外の 4 件
        assert_eq!(warnings.len(), 4);
    }

    #[test]
    fn reads_pip_compile_output() {
        let txt = "#\n# This file is autogenerated by pip-compile\n#\n\
                   certifi==2024.2.2\n    # via\n    #   httpx\n    #   requests\n\
                   httpx==0.27.0\n    # via -r requirements.in\n\
                   requests==2.31.0\n    # via -r requirements.in\n";
        let (list, warnings, transitive) = parse_requirements(txt);
        assert!(transitive);
        assert!(warnings.is_empty());
        assert_eq!(find(&list, "pypi:certifi@2024.2.2").direct, Some(false));
        assert_eq!(find(&list, "pypi:httpx@0.27.0").direct, Some(true));
        assert_eq!(find(&list, "pypi:requests@2.31.0").dependencies, vec!["pypi:certifi@2024.2.2"]);
        assert_eq!(find(&list, "pypi:httpx@0.27.0").dependencies, vec!["pypi:certifi@2024.2.2"]);
    }

    #[test]
    fn reads_poetry_lock_with_pyproject() {
        let lock = r#"
[[package]]
name = "requests"
version = "2.31.0"
groups = ["main"]

[package.dependencies]
certifi = ">=2017.4.17"
urllib3 = ">=1.21.1,<3"

[[package]]
name = "certifi"
version = "2024.2.2"
groups = ["main"]

[[package]]
name = "urllib3"
version = "2.2.1"
groups = ["main"]

[[package]]
name = "pytest"
version = "8.0.0"
category = "dev"

[package.dependencies]
Pluggy = ">=1.3"

[[package]]
name = "pluggy"
version = "1.4.0"
groups = ["dev"]

[metadata]
lock-version = "2.1"
"#;
        let pyproject = r#"
[project]
name = "myapp"
dependencies = ["requests>=2.31"]

[tool.poetry.group.dev.dependencies]
pytest = "^8"
"#;
        let (name, list, warnings) = parse_poetry(lock, Some(pyproject)).unwrap();
        assert_eq!(name.as_deref(), Some("myapp"));
        assert!(warnings.is_empty());
        let requests = find(&list, "pypi:requests@2.31.0");
        assert_eq!(requests.direct, Some(true));
        assert_eq!(requests.scope, None);
        assert_eq!(requests.dependencies, vec!["pypi:certifi@2024.2.2", "pypi:urllib3@2.2.1"]);
        assert_eq!(find(&list, "pypi:certifi@2024.2.2").direct, Some(false));
        let pytest = find(&list, "pypi:pytest@8.0.0");
        assert_eq!(pytest.direct, Some(true));
        assert_eq!(pytest.scope.as_deref(), Some("dev"));
        assert_eq!(pytest.dependencies, vec!["pypi:pluggy@1.4.0"]);

        let (_, list, warnings) = parse_poetry(lock, None).unwrap();
        assert!(list.iter().all(|c| c.direct.is_none()));
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn reads_uv_lock() {
        let lock = r#"
version = 1
requires-python = ">=3.12"

[[package]]
name = "myapp"
version = "0.1.0"
source = { editable = "." }
dependencies = [{ name = "httpx" }]

[package.dev-dependencies]
dev = [{ name = "pytest" }]

[[package]]
name = "httpx"
version = "0.27.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [{ name = "certifi" }, { name = "anyio" }]

[[package]]
name = "certifi"
version = "2024.2.2"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "anyio"
version = "4.3.0"
source = { git = "https://github.com/agronholm/anyio?rev=4.3.0#abc" }

[[package]]
name = "pytest"
version = "8.0.0"
source = { registry = "https://pypi.org/simple" }
dependencies = [{ name = "certifi" }]
"#;
        let (name, list, warnings) = parse_uv(lock).unwrap();
        assert_eq!(name.as_deref(), Some("myapp"));
        assert_eq!(list.len(), 4);
        let httpx = find(&list, "pypi:httpx@0.27.0");
        assert_eq!(httpx.direct, Some(true));
        assert_eq!(httpx.scope, None);
        assert_eq!(httpx.dependencies, vec!["pypi:anyio", "pypi:certifi@2024.2.2"]);
        let pytest = find(&list, "pypi:pytest@8.0.0");
        assert_eq!(pytest.direct, Some(true));
        assert_eq!(pytest.scope.as_deref(), Some("dev"));
        // certifi は実行時の依存（httpx）から辿れる
        assert_eq!(find(&list, "pypi:certifi@2024.2.2").scope, None);
        let anyio = find(&list, "pypi:anyio");
        assert!(anyio.purl.is_none() && anyio.version.is_none());
        assert!(anyio.notes[0].contains("4.3.0"));
        assert_eq!(warnings.len(), 1);
    }
}

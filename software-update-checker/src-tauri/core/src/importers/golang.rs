//! Go の go.mod の取り込み。
//! 仕様: https://go.dev/ref/mod#go-mod-file
//!
//! - Go 1.17 以降の go.mod には、ビルドに使うすべてのモジュールが `require` に並ぶ。
//!   `// indirect` の付いていないものを直接依存とする。
//! - `replace` で別のモジュール・版に置き換えたものは置き換え先を、ローカルのパスに置き換えたものは照会しない。
//! - go.mod には依存関係のつながりが無いので、ツリーは作らない。

use super::Importer;
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use std::collections::HashMap;
use std::path::Path;

pub struct GoModImporter;

impl Importer for GoModImporter {
    fn format_id(&self) -> &'static str {
        "go-mod"
    }

    fn can_import(&self, path: &Path, _content: &str) -> bool {
        path.file_name().and_then(|n| n.to_str()) == Some("go.mod")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let (project_name, components, warnings) = parse(content);
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

/// `replace` の置き換え先。
enum Replacement {
    Module(String, String),
    Local(String),
}

/// 行末のコメントと前後の空白を落とし、`// indirect` かどうかを返す。
fn strip_comment(line: &str) -> (&str, bool) {
    match line.split_once("//") {
        Some((code, comment)) => (code.trim(), comment.split(';').any(|c| c.trim() == "indirect")),
        None => (line.trim(), false),
    }
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches(|c| c == '"' || c == '`')
}

fn parse(content: &str) -> (Option<String>, Vec<Component>, Vec<String>) {
    let mut module = None;
    let mut requires: Vec<(String, String, bool)> = Vec::new();
    // 置き換え元（版の指定があれば `path@version`、無ければ `path`）→ 置き換え先
    let mut replaces: HashMap<String, Replacement> = HashMap::new();
    let mut block: Option<&str> = None;

    for raw in content.lines() {
        let (line, indirect) = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            block = None;
            continue;
        }
        let (directive, rest) = match block {
            Some(b) => (b, line),
            None => {
                let (d, r) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
                let r = r.trim();
                if r == "(" {
                    block = Some(match d {
                        "require" => "require",
                        "replace" => "replace",
                        _ => "other",
                    });
                    continue;
                }
                (d, r)
            }
        };
        match directive {
            "module" => module = Some(unquote(rest).to_string()),
            "require" => {
                let mut parts = rest.split_whitespace();
                if let (Some(path), Some(version)) = (parts.next(), parts.next()) {
                    requires.push((unquote(path).to_string(), version.to_string(), indirect));
                }
            }
            "replace" => {
                let Some((from, to)) = rest.split_once("=>") else { continue };
                let from: Vec<&str> = from.split_whitespace().collect();
                let to: Vec<&str> = to.split_whitespace().collect();
                let key = match from.as_slice() {
                    [p, v] => format!("{}@{v}", unquote(p)),
                    [p] => unquote(p).to_string(),
                    _ => continue,
                };
                let replacement = match to.as_slice() {
                    [p, v] => Replacement::Module(unquote(p).to_string(), v.to_string()),
                    [p] => Replacement::Local(unquote(p).to_string()),
                    _ => continue,
                };
                replaces.insert(key, replacement);
            }
            _ => {}
        }
    }

    let mut warnings = Vec::new();
    let mut local = 0;
    let mut components = Vec::new();
    for (path, version, indirect) in requires {
        let replacement = replaces.get(&format!("{path}@{version}")).or_else(|| replaces.get(&path));
        let (real_path, real_version, note) = match replacement {
            Some(Replacement::Module(p, v)) => {
                (p.clone(), Some(v.clone()), Some(tr!("{path} {version} を置き換えたもの", "Replaces {path} {version}")))
            }
            Some(Replacement::Local(dir)) => {
                local += 1;
                (path.clone(), None, Some(tr!("ローカルの {dir} に置き換えているため、照会しません。", "Replaced with the local {dir}, so it is not looked up.")))
            }
            None => (path.clone(), Some(version.clone()), None),
        };
        // `github.com/foo/bar` → group `github.com/foo`, name `bar`
        let (group, name) = match real_path.rsplit_once('/') {
            Some((g, n)) => (Some(g.to_string()), n.to_string()),
            None => (None, real_path.clone()),
        };
        let mut c = Component::new(format!("go:{real_path}@{}", real_version.as_deref().unwrap_or("local")), Ecosystem::Golang, name.clone());
        c.group = group.clone();
        c.version = real_version.clone();
        c.purl = real_version.as_deref().map(|v| Purl::build("golang", group.as_deref(), &name, Some(v)).to_string_canonical(true));
        c.direct = Some(!indirect);
        c.notes.extend(note);
        components.push(c);
    }
    if local > 0 {
        warnings.push(tr!(
            "{local} 件はローカルのディレクトリに置き換えているため、チェック対象外です。",
            "{local} modules are replaced with local directories and are not checked."
        ));
    }
    warnings.push(tr!(
        "go.mod には依存関係のつながりが無いため、ツリーは表示できません（直接依存かどうかは // indirect から判定）。",
        "go.mod has no dependency graph, so no tree is shown (direct dependencies are taken from // indirect)."
    ));
    (module, components, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_go_mod() {
        let gomod = r#"module example.com/app

go 1.22

require (
	github.com/gin-gonic/gin v1.9.1
	golang.org/x/net v0.17.0 // indirect
	gopkg.in/yaml.v3 v3.0.1 // indirect
	example.com/internal v0.0.0-00010101000000-000000000000
	github.com/old/lib v1.0.0
)

require github.com/stretchr/testify v1.8.4

replace example.com/internal => ../internal

replace github.com/old/lib v1.0.0 => github.com/new/lib v1.2.0
"#;
        let (module, list, warnings) = parse(gomod);
        assert_eq!(module.as_deref(), Some("example.com/app"));
        assert_eq!(list.len(), 6);
        let find = |name: &str| list.iter().find(|c| c.name == name).unwrap();
        let gin = find("gin");
        assert_eq!(gin.group.as_deref(), Some("github.com/gin-gonic"));
        assert_eq!(gin.package_name(), "github.com/gin-gonic/gin");
        assert_eq!(gin.direct, Some(true));
        assert_eq!(gin.purl.as_deref(), Some("pkg:golang/github.com/gin-gonic/gin@v1.9.1"));
        assert_eq!(find("net").direct, Some(false));
        assert_eq!(find("yaml.v3").group.as_deref(), Some("gopkg.in"));
        assert_eq!(find("testify").direct, Some(true));
        let internal = find("internal");
        assert!(internal.version.is_none() && internal.purl.is_none());
        let lib = find("lib");
        assert_eq!((lib.group.as_deref(), lib.version.as_deref()), (Some("github.com/new"), Some("v1.2.0")));
        assert_eq!(warnings.len(), 2);
    }
}

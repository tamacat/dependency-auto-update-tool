//! pom.xml の直接読込。
//!
//! Maven を使わずに読める範囲（このファイル内の `${property}`、
//! dependencyManagement、parent の version）だけを解決する。
//! 親 POM や import された BOM の中身は追わないため、推移的依存は含まない。
//! 推移的依存まで必要なら [`super::maven`] で SBOM を生成して読み込む。

use super::Importer;
use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use roxmltree::{Document, Node};
use std::collections::HashMap;
use std::path::Path;

pub struct PomImporter;

impl Importer for PomImporter {
    fn format_id(&self) -> &'static str {
        "pom"
    }

    fn can_import(&self, path: &Path, content: &str) -> bool {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        if file_name == "pom.xml" || file_name.ends_with(".pom") {
            return true;
        }
        file_name.ends_with(".xml") && content.contains("maven.apache.org/POM")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let doc = Document::parse(content).map_err(|e| tr!("pom.xml の XML が不正です: {e}", "Invalid XML in pom.xml: {e}"))?;
        let project = doc.root_element();
        if project.tag_name().name() != "project" {
            return Err(tr!("pom.xml のルート要素が <project> ではありません", "The root element of pom.xml is not <project>"));
        }

        let parent = child(project, "parent");
        let parent_group = parent.and_then(|p| child_text(p, "groupId"));
        let parent_version = parent.and_then(|p| child_text(p, "version"));
        let group_id = child_text(project, "groupId").or_else(|| parent_group.clone());
        let artifact_id = child_text(project, "artifactId");
        let version = child_text(project, "version").or_else(|| parent_version.clone());

        let mut props: HashMap<String, String> = HashMap::new();
        if let Some(p) = child(project, "properties") {
            for n in p.children().filter(Node::is_element) {
                props.insert(n.tag_name().name().to_string(), n.text().unwrap_or("").trim().to_string());
            }
        }
        for (key, value) in [
            ("project.groupId", &group_id),
            ("project.artifactId", &artifact_id),
            ("project.version", &version),
            ("pom.version", &version),
            ("project.parent.version", &parent_version),
            ("project.parent.groupId", &parent_group),
        ] {
            if let Some(v) = value {
                props.entry(key.to_string()).or_insert_with(|| v.clone());
            }
        }

        let mut warnings = vec![tr!(
            "pom.xml の直接依存のみを表示しています。推移的依存を見るには「Maven で SBOM を生成」を使ってください。",
            "Only the direct dependencies in pom.xml are shown. Use \"Generate SBOM with Maven\" to see transitive dependencies."
        )];
        let mut components = Vec::new();

        if let Some(p) = parent {
            if let Some(c) = dependency_component(p, &props, &HashMap::new(), "parent") {
                components.push(c);
            }
        }

        // dependencyManagement: バージョン補完用の表と、import スコープの BOM
        let mut managed: HashMap<String, String> = HashMap::new();
        let dm_deps: Vec<Node> = child(project, "dependencyManagement")
            .and_then(|dm| child(dm, "dependencies"))
            .map(|d| d.children().filter(|n| n.has_tag_name("dependency")).collect())
            .unwrap_or_default();
        for dep in &dm_deps {
            let (Some(g), Some(a)) = (child_text(*dep, "groupId"), child_text(*dep, "artifactId")) else {
                continue;
            };
            if let Some(v) = child_text(*dep, "version") {
                managed.insert(format!("{}:{}", resolve(&g, &props), resolve(&a, &props)), resolve(&v, &props));
            }
            if child_text(*dep, "scope").as_deref() == Some("import") {
                if let Some(c) = dependency_component(*dep, &props, &HashMap::new(), "import") {
                    components.push(c);
                }
            }
        }

        let deps = child(project, "dependencies")
            .map(|d| d.children().filter(|n| n.has_tag_name("dependency")).collect::<Vec<_>>())
            .unwrap_or_default();
        for dep in deps {
            if let Some(c) = dependency_component(dep, &props, &managed, "compile") {
                components.push(c);
            }
        }

        if components.iter().any(|c| c.version.is_none()) {
            warnings.push(
                tr!(
                    "バージョンを解決できない依存があります（親 POM や BOM で管理されている可能性があります）。",
                    "Some dependency versions cannot be resolved (they may be managed by a parent POM or BOM)."
                ),
            );
        }

        let project_name = match (&group_id, &artifact_id) {
            (Some(g), Some(a)) => Some(match &version {
                Some(v) => format!("{g}:{a}:{}", resolve(v, &props)),
                None => format!("{g}:{a}"),
            }),
            _ => artifact_id.clone(),
        };

        Ok(ImportResult {
            source_path: path.display().to_string(),
            format: self.format_id().into(),
            project_name,
            components,
            warnings,
            includes_transitive: false,
        })
    }
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.is_element() && n.tag_name().name() == name)
}

fn child_text(node: Node, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// `${name}` を繰り返し展開する。解決できないものはそのまま残す。
fn resolve(value: &str, props: &HashMap<String, String>) -> String {
    let mut current = value.to_string();
    for _ in 0..10 {
        let Some(start) = current.find("${") else { break };
        let Some(len) = current[start..].find('}') else { break };
        let key = &current[start + 2..start + len];
        let Some(replacement) = props.get(key) else { break };
        current = format!("{}{}{}", &current[..start], replacement, &current[start + len + 1..]);
    }
    current
}

fn dependency_component(
    dep: Node,
    props: &HashMap<String, String>,
    managed: &HashMap<String, String>,
    default_scope: &str,
) -> Option<Component> {
    let group = resolve(&child_text(dep, "groupId")?, props);
    let artifact = resolve(&child_text(dep, "artifactId")?, props);
    let key = format!("{group}:{artifact}");
    let raw_version = child_text(dep, "version");
    let scope = child_text(dep, "scope").unwrap_or_else(|| default_scope.to_string());

    let mut c = Component::new(format!("{key}#{scope}"), Ecosystem::Maven, artifact.clone());
    c.group = Some(group.clone());
    c.scope = Some(scope);
    c.direct = Some(true);

    let version = match &raw_version {
        Some(v) => Some(resolve(v, props)),
        None => managed.get(&key).cloned(),
    };
    match version {
        Some(v) if v.contains("${") => {
            c.notes.push(tr!("バージョン {v} を解決できません", "Cannot resolve version {v}"));
        }
        Some(v) => {
            if v.starts_with(['[', '(']) {
                c.notes.push(tr!(
                    "バージョン範囲指定 {v}（実際に使われるバージョンは Maven の解決結果による）",
                    "Version range {v} (the actual version depends on Maven's resolution)"
                ));
            }
            if raw_version.is_none() {
                c.notes.push(tr!("dependencyManagement からバージョンを補完", "Version taken from dependencyManagement"));
            }
            c.version = Some(v);
        }
        None => c.notes.push(tr!("バージョン指定なし（親 POM / BOM で管理）", "No version (managed by a parent POM / BOM)")),
    }
    if child_text(dep, "optional").as_deref() == Some("true") {
        c.notes.push("optional".into());
    }
    c.purl = Some(Purl::build("maven", Some(&group), &artifact, c.version.as_deref()).to_string_canonical(true));
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.5</version>
  </parent>
  <groupId>org.example</groupId>
  <artifactId>demo</artifactId>
  <version>1.0.0</version>
  <properties>
    <tomcat.version>10.1.20</tomcat.version>
    <jackson.version>${jackson.base}.1</jackson.version>
    <jackson.base>2.17</jackson.base>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId><artifactId>slf4j-api</artifactId><version>2.0.13</version>
      </dependency>
      <dependency>
        <groupId>org.junit</groupId><artifactId>junit-bom</artifactId><version>5.10.2</version>
        <type>pom</type><scope>import</scope>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.apache.tomcat.embed</groupId><artifactId>tomcat-embed-core</artifactId>
      <version>${tomcat.version}</version>
    </dependency>
    <dependency>
      <groupId>com.fasterxml.jackson.core</groupId><artifactId>jackson-databind</artifactId>
      <version>${jackson.version}</version>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId><artifactId>slf4j-api</artifactId>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><scope>test</scope>
    </dependency>
    <dependency>
      <groupId>x</groupId><artifactId>y</artifactId><version>${undefined.version}</version>
    </dependency>
  </dependencies>
</project>"#;

    fn find<'a>(r: &'a ImportResult, name: &str) -> &'a Component {
        r.components.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn imports_direct_dependencies_with_property_resolution() {
        let r = PomImporter.import(Path::new("pom.xml"), POM).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("org.example:demo:1.0.0"));
        assert!(!r.includes_transitive);

        assert_eq!(find(&r, "spring-boot-starter-parent").scope.as_deref(), Some("parent"));
        assert_eq!(find(&r, "junit-bom").scope.as_deref(), Some("import"));

        let tomcat = find(&r, "tomcat-embed-core");
        assert_eq!(tomcat.version.as_deref(), Some("10.1.20"));
        assert_eq!(tomcat.purl.as_deref(), Some("pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20"));

        assert_eq!(find(&r, "jackson-databind").version.as_deref(), Some("2.17.1"));
        assert_eq!(find(&r, "slf4j-api").version.as_deref(), Some("2.0.13"));

        let junit = find(&r, "junit-jupiter");
        assert_eq!(junit.version, None);
        assert_eq!(junit.scope.as_deref(), Some("test"));

        let y = find(&r, "y");
        assert_eq!(y.version, None);
        assert!(y.notes[0].contains("undefined.version"));
    }
}

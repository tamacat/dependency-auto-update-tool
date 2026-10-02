//! CycloneDX SBOM（JSON / XML）の取り込み。
//! 仕様: https://cyclonedx.org/specification/overview/

use super::{apply_purl, mark_direct, Importer};
use crate::model::{Component, Ecosystem, ImportResult};
use roxmltree::{Document, Node};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// 形式に依存しない中間表現。JSON と XML の両方からここへ変換する。
#[derive(Default)]
struct Bom {
    root_ref: Option<String>,
    project_name: Option<String>,
    components: Vec<Component>,
    edges: HashMap<String, Vec<String>>,
}

impl Bom {
    fn into_result(mut self, path: &Path, format: &str) -> ImportResult {
        let has_edges = !self.edges.is_empty();
        for c in self.components.iter_mut() {
            if let Some(children) = self.edges.get(&c.id) {
                c.dependencies = children.clone();
            }
        }
        let root_children = self
            .root_ref
            .as_ref()
            .and_then(|r| self.edges.get(r))
            .cloned()
            .unwrap_or_default();
        mark_direct(&mut self.components, &root_children, has_edges && self.root_ref.is_some());

        let mut warnings = Vec::new();
        if !has_edges {
            warnings.push(tr!(
                "SBOM に依存関係（dependencies）が無いため、直接依存か推移的依存かを判定できません。",
                "The SBOM has no dependency graph (dependencies), so direct and transitive dependencies cannot be told apart."
            ));
        }
        let no_purl = self.components.iter().filter(|c| c.purl.is_none()).count();
        if no_purl > 0 {
            warnings.push(tr!(
                "{no_purl} 件のコンポーネントに purl が無く、チェック対象外です。",
                "{no_purl} components have no purl and are not checked."
            ));
        }
        ImportResult {
            source_path: path.display().to_string(),
            format: format.into(),
            project_name: self.project_name,
            components: self.components,
            warnings,
            includes_transitive: has_edges,
        }
    }
}

// ---------------------------------------------------------------- JSON

pub struct CycloneDxJsonImporter;

impl Importer for CycloneDxJsonImporter {
    fn format_id(&self) -> &'static str {
        "cyclonedx-json"
    }

    fn can_import(&self, _path: &Path, content: &str) -> bool {
        content.trim_start().starts_with('{') && content.contains("\"bomFormat\"") && content.contains("CycloneDX")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let json: Value = serde_json::from_str(content).map_err(|e| tr!("CycloneDX JSON が不正です: {e}", "Invalid CycloneDX JSON: {e}"))?;
        let mut bom = Bom::default();

        if let Some(meta) = json.pointer("/metadata/component") {
            bom.root_ref = str_field(meta, "bom-ref");
            bom.project_name = component_display_name(meta);
        }
        if let Some(list) = json.get("components").and_then(Value::as_array) {
            collect_json_components(list, &mut bom.components);
        }
        if let Some(deps) = json.get("dependencies").and_then(Value::as_array) {
            for d in deps {
                let Some(r) = str_field(d, "ref") else { continue };
                let children: Vec<String> = d
                    .get("dependsOn")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                bom.edges.insert(r, children);
            }
        }
        Ok(bom.into_result(path, self.format_id()))
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

fn component_display_name(v: &Value) -> Option<String> {
    let name = str_field(v, "name")?;
    let mut s = match str_field(v, "group") {
        Some(g) => format!("{g}:{name}"),
        None => name,
    };
    if let Some(ver) = str_field(v, "version") {
        s.push(':');
        s.push_str(&ver);
    }
    Some(s)
}

fn collect_json_components(list: &[Value], out: &mut Vec<Component>) {
    for v in list {
        let name = str_field(v, "name").unwrap_or_default();
        let id = str_field(v, "bom-ref")
            .or_else(|| str_field(v, "purl"))
            .unwrap_or_else(|| format!("component-{}", out.len()));
        let mut c = Component::new(id, Ecosystem::Other, name);
        c.group = str_field(v, "group");
        c.version = str_field(v, "version");
        c.scope = str_field(v, "scope");
        if let Some(purl) = str_field(v, "purl") {
            apply_purl(&mut c, &purl);
        }
        if let Some(lics) = v.get("licenses").and_then(Value::as_array) {
            for l in lics {
                let text = l
                    .pointer("/license/id")
                    .or_else(|| l.pointer("/license/name"))
                    .or_else(|| l.get("expression"))
                    .and_then(Value::as_str);
                if let Some(t) = text {
                    c.licenses.push(t.to_string());
                }
            }
        }
        out.push(c);
        if let Some(nested) = v.get("components").and_then(Value::as_array) {
            collect_json_components(nested, out);
        }
    }
}

// ---------------------------------------------------------------- XML

pub struct CycloneDxXmlImporter;

impl Importer for CycloneDxXmlImporter {
    fn format_id(&self) -> &'static str {
        "cyclonedx-xml"
    }

    fn can_import(&self, _path: &Path, content: &str) -> bool {
        content.trim_start().starts_with('<') && content.contains("cyclonedx.org/schema/bom")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let doc = Document::parse(content).map_err(|e| tr!("CycloneDX XML が不正です: {e}", "Invalid CycloneDX XML: {e}"))?;
        let root = doc.root_element();
        let mut bom = Bom::default();

        if let Some(meta) = child(root, "metadata").and_then(|m| child(m, "component")) {
            bom.root_ref = meta.attribute("bom-ref").map(str::to_string);
            bom.project_name = child_text(meta, "name").map(|n| match child_text(meta, "group") {
                Some(g) => format!("{g}:{n}"),
                None => n,
            });
        }
        if let Some(list) = child(root, "components") {
            collect_xml_components(list, &mut bom.components);
        }
        if let Some(deps) = child(root, "dependencies") {
            for d in deps.children().filter(|n| n.has_tag_name("dependency")) {
                let Some(r) = d.attribute("ref") else { continue };
                let children = d
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "dependency")
                    .filter_map(|n| n.attribute("ref").map(str::to_string))
                    .collect();
                bom.edges.insert(r.to_string(), children);
            }
        }
        Ok(bom.into_result(path, self.format_id()))
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

fn collect_xml_components(list: Node, out: &mut Vec<Component>) {
    for n in list.children().filter(|n| n.is_element() && n.tag_name().name() == "component") {
        let name = child_text(n, "name").unwrap_or_default();
        let id = n
            .attribute("bom-ref")
            .map(str::to_string)
            .or_else(|| child_text(n, "purl"))
            .unwrap_or_else(|| format!("component-{}", out.len()));
        let mut c = Component::new(id, Ecosystem::Other, name);
        c.group = child_text(n, "group");
        c.version = child_text(n, "version");
        c.scope = child_text(n, "scope");
        if let Some(purl) = child_text(n, "purl") {
            apply_purl(&mut c, &purl);
        }
        if let Some(lics) = child(n, "licenses") {
            for l in lics.children().filter(Node::is_element) {
                let text = match l.tag_name().name() {
                    "license" => child_text(l, "id").or_else(|| child_text(l, "name")),
                    "expression" => l.text().map(|t| t.trim().to_string()),
                    _ => None,
                };
                if let Some(t) = text {
                    c.licenses.push(t);
                }
            }
        }
        out.push(c);
        if let Some(nested) = child(n, "components") {
            collect_xml_components(nested, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
      "bomFormat": "CycloneDX", "specVersion": "1.5",
      "metadata": { "component": { "bom-ref": "root", "group": "org.example", "name": "demo", "version": "1.0" } },
      "components": [
        { "bom-ref": "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20?type=jar",
          "group": "org.apache.tomcat.embed", "name": "tomcat-embed-core", "version": "10.1.20",
          "purl": "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20?type=jar",
          "scope": "required", "licenses": [ { "license": { "id": "Apache-2.0" } } ] },
        { "bom-ref": "annotations", "name": "tomcat-annotations-api", "group": "org.apache.tomcat",
          "version": "10.1.20", "purl": "pkg:maven/org.apache.tomcat/tomcat-annotations-api@10.1.20" },
        { "bom-ref": "nopurl", "name": "internal-lib", "version": "1.0" }
      ],
      "dependencies": [
        { "ref": "root", "dependsOn": [ "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20?type=jar", "nopurl" ] },
        { "ref": "pkg:maven/org.apache.tomcat.embed/tomcat-embed-core@10.1.20?type=jar", "dependsOn": [ "annotations" ] }
      ]
    }"#;

    #[test]
    fn imports_json_with_direct_flags() {
        let r = CycloneDxJsonImporter.import(Path::new("bom.json"), JSON).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("org.example:demo:1.0"));
        assert!(r.includes_transitive);
        assert_eq!(r.components.len(), 3);
        let tomcat = &r.components[0];
        assert_eq!(tomcat.ecosystem, Ecosystem::Maven);
        assert_eq!(tomcat.direct, Some(true));
        assert_eq!(tomcat.licenses, vec!["Apache-2.0"]);
        assert_eq!(tomcat.dependencies, vec!["annotations"]);
        assert_eq!(r.components[1].direct, Some(false));
        assert_eq!(r.components[2].ecosystem, Ecosystem::Other);
        assert!(r.warnings.iter().any(|w| w.contains("purl")));
    }

    const XML: &str = r#"<?xml version="1.0"?>
<bom xmlns="http://cyclonedx.org/schema/bom/1.5" version="1">
  <metadata><component type="application" bom-ref="root"><group>org.example</group><name>demo</name></component></metadata>
  <components>
    <component type="library" bom-ref="a">
      <group>com.fasterxml.jackson.core</group><name>jackson-databind</name><version>2.17.1</version>
      <licenses><license><id>Apache-2.0</id></license></licenses>
      <purl>pkg:maven/com.fasterxml.jackson.core/jackson-databind@2.17.1?type=jar</purl>
      <components>
        <component type="library" bom-ref="b"><name>left-pad</name><version>1.3.0</version><purl>pkg:npm/left-pad@1.3.0</purl></component>
      </components>
    </component>
  </components>
  <dependencies>
    <dependency ref="root"><dependency ref="a"/></dependency>
    <dependency ref="a"><dependency ref="b"/></dependency>
  </dependencies>
</bom>"#;

    #[test]
    fn imports_xml_including_nested_components() {
        assert!(CycloneDxXmlImporter.can_import(Path::new("bom.xml"), XML));
        let r = CycloneDxXmlImporter.import(Path::new("bom.xml"), XML).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("org.example:demo"));
        assert_eq!(r.components.len(), 2);
        assert_eq!(r.components[0].direct, Some(true));
        assert_eq!(r.components[0].licenses, vec!["Apache-2.0"]);
        assert_eq!(r.components[1].ecosystem, Ecosystem::Npm);
        assert_eq!(r.components[1].direct, Some(false));
    }
}

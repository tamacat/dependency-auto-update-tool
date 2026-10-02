//! SPDX 2.x JSON の取り込み。
//! 仕様: https://spdx.github.io/spdx-spec/

use super::{apply_purl, mark_direct, Importer};
use crate::model::{Component, Ecosystem, ImportResult};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub struct SpdxJsonImporter;

impl Importer for SpdxJsonImporter {
    fn format_id(&self) -> &'static str {
        "spdx-json"
    }

    fn can_import(&self, _path: &Path, content: &str) -> bool {
        content.trim_start().starts_with('{') && content.contains("\"spdxVersion\"")
    }

    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String> {
        let json: Value = serde_json::from_str(content).map_err(|e| tr!("SPDX JSON が不正です: {e}", "Invalid SPDX JSON: {e}"))?;
        let doc_id = str_field(&json, "SPDXID").unwrap_or_else(|| "SPDXRef-DOCUMENT".into());

        // ルート（文書が記述しているパッケージ）
        let mut roots: HashSet<String> = json
            .get("documentDescribes")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();

        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        for rel in json.get("relationships").and_then(Value::as_array).into_iter().flatten() {
            let (Some(from), Some(kind), Some(to)) = (
                str_field(rel, "spdxElementId"),
                str_field(rel, "relationshipType"),
                str_field(rel, "relatedSpdxElement"),
            ) else {
                continue;
            };
            match kind.as_str() {
                "DESCRIBES" if from == doc_id => {
                    roots.insert(to);
                }
                "DESCRIBED_BY" if to == doc_id => {
                    roots.insert(from);
                }
                "DEPENDS_ON" | "CONTAINS" => {
                    edges.entry(from).or_default().push(to);
                }
                "DEPENDENCY_OF" | "DEV_DEPENDENCY_OF" | "OPTIONAL_DEPENDENCY_OF" | "RUNTIME_DEPENDENCY_OF" => {
                    edges.entry(to).or_default().push(from);
                }
                _ => {}
            }
        }

        let mut project_name = None;
        let mut components = Vec::new();
        for p in json.get("packages").and_then(Value::as_array).into_iter().flatten() {
            let Some(id) = str_field(p, "SPDXID") else { continue };
            let name = str_field(p, "name").unwrap_or_default();
            if roots.contains(&id) {
                if project_name.is_none() {
                    project_name = Some(match str_field(p, "versionInfo") {
                        Some(v) => format!("{name}:{v}"),
                        None => name,
                    });
                }
                continue;
            }
            let mut c = Component::new(id.clone(), Ecosystem::Other, name);
            c.version = str_field(p, "versionInfo");
            for r in p.get("externalRefs").and_then(Value::as_array).into_iter().flatten() {
                if str_field(r, "referenceType").as_deref() == Some("purl") {
                    if let Some(locator) = str_field(r, "referenceLocator") {
                        apply_purl(&mut c, &locator);
                        break;
                    }
                }
            }
            for key in ["licenseConcluded", "licenseDeclared"] {
                if let Some(l) = str_field(p, key) {
                    if l != "NOASSERTION" && l != "NONE" && !c.licenses.contains(&l) {
                        c.licenses.push(l);
                    }
                }
            }
            if let Some(children) = edges.get(&id) {
                c.dependencies = children.clone();
            }
            components.push(c);
        }

        let root_children: Vec<String> = roots.iter().filter_map(|r| edges.get(r)).flatten().cloned().collect();
        // ルートからの DEPENDS_ON が 1 本も無ければ直接/推移の判定はしない
        let has_edges = !root_children.is_empty();
        mark_direct(&mut components, &root_children, has_edges);

        let mut warnings = Vec::new();
        if !has_edges {
            warnings.push(tr!(
                "SPDX にルートからの依存関係が無いため、直接依存か推移的依存かを判定できません。",
                "The SPDX document has no relationships from the root, so direct and transitive dependencies cannot be told apart."
            ));
        }
        let no_purl = components.iter().filter(|c| c.purl.is_none()).count();
        if no_purl > 0 {
            warnings.push(tr!(
                "{no_purl} 件のパッケージに purl が無く、チェック対象外です。",
                "{no_purl} packages have no purl and are not checked."
            ));
        }

        Ok(ImportResult {
            source_path: path.display().to_string(),
            format: self.format_id().into(),
            project_name,
            components,
            warnings,
            includes_transitive: !edges.is_empty(),
        })
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPDX: &str = r#"{
      "spdxVersion": "SPDX-2.3", "SPDXID": "SPDXRef-DOCUMENT", "name": "demo",
      "documentDescribes": ["SPDXRef-root"],
      "packages": [
        { "SPDXID": "SPDXRef-root", "name": "demo", "versionInfo": "1.0" },
        { "SPDXID": "SPDXRef-a", "name": "jackson-databind", "versionInfo": "2.17.1",
          "licenseConcluded": "Apache-2.0",
          "externalRefs": [ { "referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl",
            "referenceLocator": "pkg:maven/com.fasterxml.jackson.core/jackson-databind@2.17.1" } ] },
        { "SPDXID": "SPDXRef-b", "name": "jackson-core", "versionInfo": "2.17.1", "licenseConcluded": "NOASSERTION",
          "externalRefs": [ { "referenceType": "purl",
            "referenceLocator": "pkg:maven/com.fasterxml.jackson.core/jackson-core@2.17.1" } ] }
      ],
      "relationships": [
        { "spdxElementId": "SPDXRef-root", "relationshipType": "DEPENDS_ON", "relatedSpdxElement": "SPDXRef-a" },
        { "spdxElementId": "SPDXRef-b", "relationshipType": "DEPENDENCY_OF", "relatedSpdxElement": "SPDXRef-a" }
      ]
    }"#;

    #[test]
    fn imports_spdx_packages_and_relationships() {
        let r = SpdxJsonImporter.import(Path::new("sbom.spdx.json"), SPDX).unwrap();
        assert_eq!(r.project_name.as_deref(), Some("demo:1.0"));
        assert_eq!(r.components.len(), 2);
        let a = &r.components[0];
        assert_eq!(a.ecosystem, Ecosystem::Maven);
        assert_eq!(a.group.as_deref(), Some("com.fasterxml.jackson.core"));
        assert_eq!(a.direct, Some(true));
        assert_eq!(a.licenses, vec!["Apache-2.0"]);
        assert_eq!(a.dependencies, vec!["SPDXRef-b"]);
        let b = &r.components[1];
        assert_eq!(b.direct, Some(false));
        assert!(b.licenses.is_empty());
    }
}

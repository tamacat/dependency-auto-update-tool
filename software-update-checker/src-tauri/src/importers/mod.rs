//! 入力ファイルの取り込み。
//!
//! 入力形式ごとに [`Importer`] を実装し、[`registry`] に登録する。
//! 新しい形式（package-lock.json、requirements.txt、Gradle など）に対応するときは、
//! このディレクトリにモジュールを追加して `registry` に足すだけでよい。

mod cyclonedx;
pub mod maven;
mod pom;
mod spdx;

use crate::model::{Component, Ecosystem, ImportResult};
use crate::purl::Purl;
use std::path::Path;

pub trait Importer: Send + Sync {
    /// 形式の識別子（`ImportResult::format` に入る）。
    fn format_id(&self) -> &'static str;
    /// このファイルを扱えるか。拡張子だけでなく中身でも判定する。
    fn can_import(&self, path: &Path, content: &str) -> bool;
    fn import(&self, path: &Path, content: &str) -> Result<ImportResult, String>;
}

pub fn registry() -> Vec<Box<dyn Importer>> {
    vec![
        Box::new(pom::PomImporter),
        Box::new(cyclonedx::CycloneDxJsonImporter),
        Box::new(cyclonedx::CycloneDxXmlImporter),
        Box::new(spdx::SpdxJsonImporter),
    ]
}

/// 対応する Importer を探して取り込む。
pub fn import_file(path: &Path) -> Result<ImportResult, String> {
    let bytes = std::fs::read(path).map_err(|e| tr!("{} を読めません: {e}", "Cannot read {}: {e}", path.display()))?;
    let content = String::from_utf8_lossy(&bytes);
    let content = content.trim_start_matches('\u{feff}');
    for importer in registry() {
        if importer.can_import(path, content) {
            return importer.import(path, content);
        }
    }
    Err(tr!(
        "{} は対応していない形式です（対応形式: pom.xml / CycloneDX JSON・XML / SPDX JSON）",
        "{} is not a supported format (supported: pom.xml / CycloneDX JSON, XML / SPDX JSON)",
        path.display()
    ))
}

/// purl 文字列からコンポーネントの基本項目を埋める。
pub(crate) fn apply_purl(component: &mut Component, purl: &str) {
    if let Some(p) = Purl::parse(purl) {
        component.ecosystem = Ecosystem::from_purl_type(&p.purl_type);
        if component.group.is_none() {
            component.group = p.namespace.clone();
        }
        if component.version.is_none() {
            component.version = p.version.clone();
        }
        component.purl = Some(purl.to_string());
    }
}

/// 依存関係のエッジから、ルートが直接依存しているものに `direct = true` を付ける。
/// エッジ情報が無い入力では `direct` は `None` のまま。
pub(crate) fn mark_direct(components: &mut [Component], root_children: &[String], has_edges: bool) {
    if !has_edges {
        return;
    }
    for c in components.iter_mut() {
        c.direct = Some(root_children.contains(&c.id));
    }
}

//! サポート終了（EOL）のチェック（endoflife.date API v1）。
//! https://endoflife.date/docs/api/v1/
//!
//! 製品の特定は次の順で行う。
//! 1. endoflife.date の purl 識別子と完全一致
//! 2. 同梱の対応表（resources/eol-mapping.json）
//! 3. 同じ namespace（groupId）を持つ製品が 1 つだけなら、それと推定
//!
//! 製品が決まったら、現行バージョンの先頭が一致するリリースサイクル
//! （例: 10.1.20 → "10.1"）を探して EOL を判定する。

use super::{canonical_purl, has_concrete_version, ttl, ProgressFn};
use crate::http::Http;
use crate::model::{CheckOptions, CheckProgress, CheckResult, Component, EolCycle, EolInfo, EolMatch, EolStatus, KeyValue};
use crate::version;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

const PRODUCTS_URL: &str = "https://endoflife.date/api/v1/products/full";
const BUNDLED_MAPPING: &str = include_str!("../../resources/eol-mapping.json");
/// 詳細画面に出すサイクルの最大数
const MAX_CYCLES: usize = 8;

#[derive(Deserialize)]
struct ProductsResponse {
    #[serde(default)]
    result: Vec<Product>,
}

#[derive(Deserialize)]
struct Product {
    name: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    identifiers: Vec<Identifier>,
    links: Option<Links>,
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
struct Identifier {
    #[serde(rename = "type")]
    kind: String,
    id: String,
}

#[derive(Deserialize)]
struct Links {
    html: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    name: String,
    #[serde(default)]
    is_eol: bool,
    eol_from: Option<String>,
    latest: Option<LatestRelease>,
    /// 製品固有の項目（minJavaVersion、supportedJavaVersions など）
    custom: Option<serde_json::Map<String, serde_json::Value>>,
}

impl Release {
    fn status(&self) -> EolStatus {
        if self.is_eol {
            EolStatus::Eol
        } else {
            EolStatus::Supported
        }
    }

    fn requirements(&self) -> Vec<KeyValue> {
        self.custom
            .iter()
            .flatten()
            .filter_map(|(k, v)| {
                let value = match v {
                    serde_json::Value::Null => return None,
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                Some(KeyValue { key: k.clone(), value })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct LatestRelease {
    name: String,
}

#[derive(Deserialize)]
struct Mapping {
    prefixes: HashMap<String, String>,
}

struct Index<'a> {
    products: HashMap<&'a str, &'a Product>,
    by_purl: HashMap<String, &'a str>,
    by_namespace: HashMap<String, HashSet<&'a str>>,
    mapping: Vec<(String, String)>,
}

impl<'a> Index<'a> {
    fn build(products: &'a [Product], mapping: Mapping) -> Self {
        let mut index = Index {
            products: HashMap::new(),
            by_purl: HashMap::new(),
            by_namespace: HashMap::new(),
            mapping: mapping.prefixes.into_iter().collect(),
        };
        // 長いキーを優先
        index.mapping.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        for p in products {
            index.products.insert(p.name.as_str(), p);
            for id in p.identifiers.iter().filter(|i| i.kind == "purl") {
                let base = id.id.split(['@', '?']).next().unwrap_or(&id.id).to_string();
                if let Some((ns, _)) = base.rsplit_once('/') {
                    index.by_namespace.entry(ns.to_string()).or_default().insert(p.name.as_str());
                }
                index.by_purl.insert(base, p.name.as_str());
            }
        }
        index
    }

    fn find(&self, base_purl: &str) -> Option<(&'a Product, EolMatch)> {
        if let Some(name) = self.by_purl.get(base_purl) {
            return Some((self.products.get(name)?, EolMatch::Exact));
        }
        for (key, product) in &self.mapping {
            let hit = if key.ends_with('/') { base_purl.starts_with(key.as_str()) } else { base_purl == key };
            if hit {
                if let Some(p) = self.products.get(product.as_str()) {
                    return Some((p, EolMatch::Inferred));
                }
            }
        }
        let (ns, _) = base_purl.rsplit_once('/')?;
        // "pkg:npm" のように namespace を持たない purl では推定しない
        if !ns.contains('/') {
            return None;
        }
        let candidates = self.by_namespace.get(ns)?;
        if candidates.len() == 1 {
            let name = candidates.iter().next()?;
            return Some((self.products.get(name)?, EolMatch::Inferred));
        }
        None
    }
}

/// バージョンに対応するリリースサイクル（最も長く一致する名前）。
fn find_cycle<'a>(product: &'a Product, version: &str) -> Option<&'a Release> {
    product
        .releases
        .iter()
        .filter(|r| {
            version == r.name
                || version.strip_prefix(r.name.as_str()).is_some_and(|rest| rest.starts_with(['.', '-', '_']))
        })
        .max_by_key(|r| r.name.len())
}

fn to_eol_info(product: &Product, match_kind: EolMatch, version: Option<&str>) -> EolInfo {
    let cycle = version.and_then(|v| find_cycle(product, v));
    EolInfo {
        product: product.name.clone(),
        product_label: if product.label.is_empty() { product.name.clone() } else { product.label.clone() },
        cycle: cycle.map(|c| c.name.clone()),
        status: cycle.map_or(EolStatus::Unknown, Release::status),
        eol_from: cycle.and_then(|c| c.eol_from.clone()),
        latest_in_cycle: cycle.and_then(|c| c.latest.as_ref().map(|l| l.name.clone())),
        match_kind,
        link: product
            .links
            .as_ref()
            .and_then(|l| l.html.clone())
            .unwrap_or_else(|| format!("https://endoflife.date/{}", product.name)),
        cycles: cycles_from(product, cycle),
    }
}

/// 現行サイクル以降のサイクル（新しい順）。現行が分からなければ新しいものから数件。
fn cycles_from(product: &Product, current: Option<&Release>) -> Vec<EolCycle> {
    let mut releases: Vec<&Release> = product
        .releases
        .iter()
        .filter(|r| current.is_none_or(|c| version::compare(&r.name, &c.name) != std::cmp::Ordering::Less))
        .collect();
    releases.sort_by(|a, b| version::compare(&b.name, &a.name));
    releases.truncate(MAX_CYCLES);
    releases
        .into_iter()
        .map(|r| EolCycle {
            name: r.name.clone(),
            status: r.status(),
            eol_from: r.eol_from.clone(),
            latest: r.latest.as_ref().map(|l| l.name.clone()),
            is_current: current.is_some_and(|c| std::ptr::eq(c, r)),
            requirements: r.requirements(),
        })
        .collect()
}

pub async fn check(
    http: &Http,
    targets: &[&Component],
    opts: &CheckOptions,
    results: &mut HashMap<String, CheckResult>,
    progress: ProgressFn,
) {
    progress(CheckProgress { phase: "eol".into(), done: 0, total: 1 });
    let products = match http.get(PRODUCTS_URL, ttl(opts, 24 * 7)).await {
        Ok(res) if res.is_success() => serde_json::from_str::<ProductsResponse>(&res.body).map_err(|e| e.to_string()),
        Ok(res) => Err(format!("HTTP {}", res.status)),
        Err(e) => Err(e),
    };
    let products = match products {
        Ok(p) => p.result,
        Err(e) => {
            for c in targets {
                if let Some(r) = results.get_mut(&c.id) {
                    r.errors.push(tr!("EOL 情報の取得に失敗: {e}", "Failed to fetch EOL information: {e}"));
                }
            }
            return;
        }
    };
    let mapping: Mapping = serde_json::from_str(BUNDLED_MAPPING).expect("同梱の eol-mapping.json が不正");
    let index = Index::build(&products, mapping);

    for c in targets {
        let Some(base) = canonical_purl(c, false) else { continue };
        let Some((product, kind)) = index.find(&base) else { continue };
        if let Some(r) = results.get_mut(&c.id) {
            let version = has_concrete_version(c).then(|| c.version.as_deref()).flatten();
            r.eol = Some(to_eol_info(product, kind, version));
        }
    }
    progress(CheckProgress { phase: "eol".into(), done: 1, total: 1 });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn products() -> Vec<Product> {
        serde_json::from_str::<ProductsResponse>(
            r#"{ "result": [
              { "name": "tomcat", "label": "Apache Tomcat",
                "identifiers": [ { "type": "purl", "id": "pkg:maven/org.apache.tomcat/tomcat" } ],
                "links": { "html": "https://endoflife.date/tomcat" },
                "releases": [
                  { "name": "11.0", "isEol": false, "latest": { "name": "11.0.26" }, "custom": { "minJavaVersion": "17" } },
                  { "name": "10.1", "isEol": false, "eolFrom": null, "latest": { "name": "10.1.60" }, "custom": { "minJavaVersion": "11", "note": null } },
                  { "name": "10.0", "isEol": true, "eolFrom": "2022-10-31", "latest": { "name": "10.0.27" } },
                  { "name": "1", "isEol": true }
                ] },
              { "name": "spring-framework", "label": "Spring Framework",
                "identifiers": [ { "type": "purl", "id": "pkg:maven/org.springframework/spring-core" } ],
                "releases": [ { "name": "6.1", "isEol": false } ] },
              { "name": "log4j", "label": "Log4j",
                "identifiers": [ { "type": "purl", "id": "pkg:maven/org.apache.logging.log4j/log4j-core" } ],
                "releases": [] }
            ] }"#,
        )
        .unwrap()
        .result
    }

    #[test]
    fn matches_exact_mapping_and_namespace() {
        let products = products();
        let index = Index::build(&products, serde_json::from_str(BUNDLED_MAPPING).unwrap());

        let (p, kind) = index.find("pkg:maven/org.apache.tomcat/tomcat").unwrap();
        assert_eq!((p.name.as_str(), kind), ("tomcat", EolMatch::Exact));

        let (p, kind) = index.find("pkg:maven/org.apache.tomcat.embed/tomcat-embed-core").unwrap();
        assert_eq!((p.name.as_str(), kind), ("tomcat", EolMatch::Inferred));

        // 対応表に無く、namespace に製品が 1 つだけ
        let (p, _) = index.find("pkg:maven/org.apache.logging.log4j/log4j-api").unwrap();
        assert_eq!(p.name, "log4j");

        assert!(index.find("pkg:maven/com.example/unknown").is_none());
        // 完全一致キー（'/' で終わらない）は前方一致しない
        assert!(index.find("pkg:npm/react-router").is_none());
    }

    #[test]
    fn determines_cycle_status() {
        let products = products();
        let tomcat = &products[0];
        let info = to_eol_info(tomcat, EolMatch::Exact, Some("10.1.20"));
        assert_eq!(info.cycle.as_deref(), Some("10.1"));
        assert_eq!(info.status, EolStatus::Supported);
        assert_eq!(info.latest_in_cycle.as_deref(), Some("10.1.60"));
        let cycles: Vec<(&str, bool)> = info.cycles.iter().map(|c| (c.name.as_str(), c.is_current)).collect();
        assert_eq!(cycles, vec![("11.0", false), ("10.1", true)]);
        assert_eq!(info.cycles[0].requirements, vec![KeyValue { key: "minJavaVersion".into(), value: "17".into() }]);

        let info = to_eol_info(tomcat, EolMatch::Exact, Some("10.0.27"));
        assert_eq!(info.status, EolStatus::Eol);
        assert_eq!(info.eol_from.as_deref(), Some("2022-10-31"));

        // "1" は "12.x" に誤一致しない
        let info = to_eol_info(tomcat, EolMatch::Exact, Some("12.0.0"));
        assert_eq!(info.status, EolStatus::Unknown);
    }
}

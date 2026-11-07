//! npm・PyPI のパッケージが必要とする実行環境の確認。
//!
//! - npm: レジストリの版ごとの記録の `engines.node`（例: `^20.19 || ^22.12 || >=24`）
//! - PyPI: 版ごとの記録の `requires_python`（例: `>=3.8`）
//!
//! Java（[`super::java`]）と違い、どちらもパッケージ自身の宣言なので、ナレッジには記録しない。
//! 範囲は npm の semver と PEP 440 のよく使われる書き方だけを読み、
//! 下限（必要な最低の版）と、利用者が設定した版が範囲に入るかを求める。

use super::{eol, ttl};
use crate::http::Http;
use crate::model::{CheckOptions, Component, Ecosystem, RuntimeRequirement, RuntimeStatus};
use crate::sources;
use serde_json::Value;
use std::cmp::Ordering;
use std::time::Duration;

const NPM_BASE: &str = "https://registry.npmjs.org";
const PYPI_BASE: &str = "https://pypi.org/pypi";
/// 公開済みの版の記録は変わらないので長めにキャッシュする
const CACHE_HOURS: u64 = 24 * 30;

type V = [u64; 3];

/// 範囲の条件 1 つ。
#[derive(Debug, Clone, PartialEq)]
enum Cond {
    /// 下限（含むか）
    Lo(V, bool),
    /// 上限（含むか）
    Hi(V, bool),
    /// この前方一致を除く（PEP 440 の `!=3.0.*`）
    Not(Vec<u64>),
}

/// `||` でつないだ候補（それぞれは条件の AND）。空の候補は「何でもよい」。
type Range = Vec<Vec<Cond>>;

/// 版の数字部分（最大 3 つ）。`x` や `*` が出たらそこまで。読めなければ `None`。
fn parts(s: &str) -> Option<Vec<u64>> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    // npm のプレリリース・ビルド情報は比較に使わない
    let core = s.split(['-', '+']).next().unwrap_or_default();
    if core.is_empty() || matches!(core, "x" | "X" | "*") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for p in core.split('.').take(3) {
        if matches!(p, "x" | "X" | "*") {
            break;
        }
        // PEP 440 の `3.8rc1`・`1.0.post1` などは先頭の数字だけ使う
        let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        out.push(digits.parse().ok()?);
        if digits.len() != p.len() {
            break;
        }
    }
    Some(out)
}

fn fill(p: &[u64], value: u64) -> V {
    [0, 1, 2].map(|i| p.get(i).copied().unwrap_or(value))
}

/// 最後に書かれた部分を 1 つ上げた版（`18` → 19.0.0、`18.2` → 18.3.0）。
fn bump(p: &[u64]) -> V {
    let mut v = fill(p, 0);
    if let Some(last) = p.len().checked_sub(1) {
        v[last] += 1;
    }
    v
}

/// npm の比較 1 つ（`>=18`、`^20.19`、`~1.2`、`18.x` など）。
fn npm_comparator(token: &str) -> Option<Vec<Cond>> {
    let token = token.trim();
    let ops = [">=", "<=", "~>", ">", "<", "=", "^", "~"];
    let op = ops.iter().find(|o| token.starts_with(**o)).copied().unwrap_or("");
    let p = parts(&token[op.len()..])?;
    if p.is_empty() {
        // `*`・`>=*` など。`<*` や `>*` は満たす版が無い
        return Some(if matches!(op, "<" | ">") { vec![Cond::Hi([0, 0, 0], false)] } else { Vec::new() });
    }
    let lo = fill(&p, 0);
    let full = p.len() == 3;
    Some(match op {
        "" | "=" if full => vec![Cond::Lo(lo, true), Cond::Hi(lo, true)],
        "" | "=" => vec![Cond::Lo(lo, true), Cond::Hi(bump(&p), false)],
        ">=" => vec![Cond::Lo(lo, true)],
        ">" if full => vec![Cond::Lo(lo, false)],
        ">" => vec![Cond::Lo(bump(&p), true)],
        "<" => vec![Cond::Hi(lo, false)],
        "<=" if full => vec![Cond::Hi(lo, true)],
        "<=" => vec![Cond::Hi(bump(&p), false)],
        "~" | "~>" => vec![Cond::Lo(lo, true), Cond::Hi(bump(&p[..p.len().min(2)]), false)],
        // `^` は左から最初の 0 でない部分を上げない範囲
        _ => {
            let keep = if p[0] > 0 || p.len() == 1 {
                1
            } else if p.len() == 2 || p[1] > 0 {
                2
            } else {
                3
            };
            vec![Cond::Lo(lo, true), Cond::Hi(bump(&p[..keep]), false)]
        }
    })
}

/// npm の範囲（`^20.19 || ^22.12 || >=24`、`>= 18`、`14 - 16` など）。
fn parse_npm(range: &str) -> Option<Range> {
    range
        .split("||")
        .map(|alt| {
            let alt = alt.trim();
            if let Some((a, b)) = alt.split_once(" - ") {
                let (a, b) = (parts(a)?, parts(b)?);
                let hi = if b.len() == 3 { Cond::Hi(fill(&b, 0), true) } else { Cond::Hi(bump(&b), false) };
                return Some(if b.is_empty() { vec![Cond::Lo(fill(&a, 0), true)] } else { vec![Cond::Lo(fill(&a, 0), true), hi] });
            }
            // `>= 18` のように演算子と版の間に空白があればつなぐ
            let mut tokens: Vec<String> = Vec::new();
            for t in alt.split_whitespace() {
                match tokens.last_mut() {
                    Some(prev) if prev.chars().all(|c| "<>=^~".contains(c)) => prev.push_str(t),
                    _ => tokens.push(t.to_string()),
                }
            }
            let mut conds = Vec::new();
            for t in tokens {
                conds.extend(npm_comparator(&t)?);
            }
            Some(conds)
        })
        .collect()
}

/// PEP 440 の指定（`>=3.8`、`>=3.8,<4`、`~=3.9`、`!=3.0.*` など）。
fn parse_pep440(spec: &str) -> Option<Range> {
    let mut conds = Vec::new();
    for clause in spec.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        let ops = ["===", "~=", "==", "!=", "<=", ">=", "<", ">"];
        let op = ops.iter().find(|o| clause.starts_with(**o)).copied()?;
        let rest = clause[op.len()..].trim();
        let (ver, wildcard) = match rest.strip_suffix(".*") {
            Some(v) => (v, true),
            None => (rest, false),
        };
        let p = parts(ver)?;
        if p.is_empty() {
            return None;
        }
        let v = fill(&p, 0);
        conds.extend(match op {
            "==" if wildcard => vec![Cond::Lo(v, true), Cond::Hi(bump(&p), false)],
            "==" | "===" => vec![Cond::Lo(v, true), Cond::Hi(v, true)],
            "!=" => vec![Cond::Not(p)],
            ">=" => vec![Cond::Lo(v, true)],
            ">" => vec![Cond::Lo(v, false)],
            "<=" => vec![Cond::Hi(v, true)],
            "<" => vec![Cond::Hi(v, false)],
            // `~=3.9` → >=3.9, <4 ／ `~=3.9.1` → >=3.9.1, <3.10
            _ if p.len() >= 2 => vec![Cond::Lo(v, true), Cond::Hi(bump(&p[..p.len() - 1]), false)],
            _ => return None,
        });
    }
    Some(vec![conds])
}

fn satisfies(range: &Range, target: V) -> bool {
    range.iter().any(|alt| {
        alt.iter().all(|c| match c {
            Cond::Lo(v, inclusive) => match target.cmp(v) {
                Ordering::Greater => true,
                Ordering::Equal => *inclusive,
                Ordering::Less => false,
            },
            Cond::Hi(v, inclusive) => match target.cmp(v) {
                Ordering::Less => true,
                Ordering::Equal => *inclusive,
                Ordering::Greater => false,
            },
            Cond::Not(prefix) => target[..prefix.len()] != prefix[..],
        })
    })
}

/// 範囲の下限。どれかの候補に下限が無ければ（何でもよければ）`None`。
fn minimum(range: &Range) -> Option<V> {
    range
        .iter()
        .map(|alt| alt.iter().filter_map(|c| if let Cond::Lo(v, _) = c { Some(*v) } else { None }).max())
        .collect::<Option<Vec<V>>>()?
        .into_iter()
        .min()
}

/// `[18, 0, 0]` → `18`、`[20, 19, 0]` → `20.19`。
fn display(v: V) -> String {
    let len = if v[2] != 0 { 3 } else if v[1] != 0 { 2 } else { 1 };
    v[..len].iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// 設定された実行環境の版。省略した部分は「その系列の最新」とみなす（`20` は 20.x のどれか）。
fn target_version(s: &str) -> Option<V> {
    let p = parts(s)?;
    (!p.is_empty()).then(|| fill(&p, u64::MAX))
}

/// 宣言された範囲から [`RuntimeRequirement`] を組み立てる（ネットワークを使わない純粋関数）。
fn evaluate(version: &str, runtime: &str, constraint: Option<&str>, target: &str) -> RuntimeRequirement {
    let constraint = constraint.map(str::trim).filter(|c| !c.is_empty());
    let range = constraint.and_then(|c| if runtime == "node" { parse_npm(c) } else { parse_pep440(c) });
    RuntimeRequirement {
        version: version.to_string(),
        runtime: runtime.to_string(),
        constraint: constraint.map(str::to_string),
        minimum: range.as_ref().and_then(minimum).map(display),
        target_ok: match (&range, target_version(target)) {
            (Some(r), Some(t)) => Some(satisfies(r, t)),
            // 宣言が無ければ制限なし
            (None, Some(_)) if constraint.is_none() => Some(true),
            _ => None,
        },
        error: None,
    }
}

/// `engines` は `{ "node": ">=18" }` が普通だが、古いパッケージには `["node >= 0.4"]` もある。
fn node_engine(doc: &Value) -> Option<String> {
    match doc.get("engines")? {
        Value::Object(m) => m.get("node").and_then(Value::as_str).map(str::to_string),
        Value::Array(a) => a.iter().filter_map(Value::as_str).find_map(|s| s.trim().strip_prefix("node").map(|r| r.trim().to_string())),
        _ => None,
    }
}

fn registry_url(base: &str, segments: &[&str]) -> String {
    let mut u = reqwest::Url::parse(base).expect("固定 URL");
    u.path_segments_mut().expect("http URL").pop_if_empty().extend(segments);
    u.to_string()
}

async fn fetch_declared(http: &Http, c: &Component, version: &str, cache: Option<Duration>) -> Result<Option<String>, String> {
    let name = c.package_name();
    let (url, source) = match c.ecosystem {
        Ecosystem::Npm => (registry_url(NPM_BASE, &[&name, version]), "npm"),
        _ => (registry_url(PYPI_BASE, &[&name, version, "json"]), "pypi"),
    };
    let res = http.get(&url, cache).await?;
    if res.status == 404 {
        return Err(tr!("レジストリにこの版がありません", "This version is not in the registry"));
    }
    if !res.is_success() {
        return Err(format!("HTTP {}", res.status));
    }
    let doc: Value = serde_json::from_str(&res.body).map_err(|e| e.to_string())?;
    Ok(match source {
        "npm" => node_engine(&doc),
        _ => doc.get("info").and_then(|i| i.get("requires_python")).and_then(Value::as_str).map(str::to_string),
    })
}

/// 指定した版ごとに、必要な実行環境を答える（npm と PyPI のみ）。
pub async fn requirements(http: &Http, c: &Component, versions: &[String], opts: &CheckOptions) -> Vec<RuntimeRequirement> {
    let (runtime, source, target) = match c.ecosystem {
        Ecosystem::Npm => ("node", sources::NPM_REGISTRY, &opts.runtime_targets.node),
        Ecosystem::Pypi => ("python", sources::PYPI, &opts.runtime_targets.python),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for v in versions {
        if !opts.source_enabled(source) {
            let mut r = evaluate(v, runtime, None, "");
            r.error = Some(tr!("データソースが無効です", "The data source is disabled"));
            out.push(r);
            continue;
        }
        out.push(match fetch_declared(http, c, v, ttl(opts, CACHE_HOURS)).await {
            Ok(declared) => evaluate(v, runtime, declared.as_deref(), target),
            Err(e) => {
                let mut r = evaluate(v, runtime, None, "");
                r.error = Some(e);
                r
            }
        });
    }
    out
}

/// 設定した実行環境の版のサポート状況（Node.js と Python は endoflife.date から）。
pub async fn status(http: &Http, opts: &CheckOptions) -> Vec<RuntimeStatus> {
    let t = &opts.runtime_targets;
    let mut out = Vec::new();
    for (runtime, version, product) in [("java", &t.java, None), ("node", &t.node, Some("nodejs")), ("python", &t.python, Some("python"))] {
        let version = version.trim();
        if version.is_empty() {
            continue;
        }
        let (eol, error) = match product {
            Some(_) if !opts.source_enabled(sources::ENDOFLIFE) => {
                (None, Some(tr!("endoflife.date が無効です", "endoflife.date is disabled")))
            }
            Some(p) => match eol::product_cycle(http, opts, p, version).await {
                Ok(info) => (info, None),
                Err(e) => (None, Some(e)),
            },
            None => (None, None),
        };
        out.push(RuntimeStatus { runtime: runtime.into(), version: version.into(), eol, error });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(range: &str, target: &str) -> bool {
        satisfies(&parse_npm(range).unwrap(), target_version(target).unwrap())
    }

    fn py_ok(spec: &str, target: &str) -> bool {
        satisfies(&parse_pep440(spec).unwrap(), target_version(target).unwrap())
    }

    #[test]
    fn evaluates_npm_ranges() {
        let r = "^20.19 || ^22.12 || >=24";
        assert!(ok(r, "20.19.1"));
        assert!(!ok(r, "20.18.0"));
        assert!(!ok(r, "21.7"));
        assert!(ok(r, "22"));
        assert!(ok(r, "26.1"));
        assert!(!ok(r, "18"));

        assert!(ok(">= 18", "18.0.0"));
        assert!(!ok(">=18", "16.20"));
        assert!(ok("14 - 16", "16.9.1"));
        assert!(!ok("14 - 16", "17"));
        assert!(ok("18.x || 20.x", "20.3"));
        assert!(ok("~1.2", "1.2.9") && !ok("~1.2", "1.3.0"));
        assert!(ok("^0.10.0", "0.10.5") && !ok("^0.10.0", "0.11.0"));
        assert!(ok(">0.8", "0.10") && !ok(">0.8", "0.8.5"));
        assert!(ok("*", "4") && ok("", "4"));
        assert!(ok("<=16", "16.20") && !ok("<16", "16.0.0"));
        assert!(parse_npm(">=foo").is_none());
    }

    #[test]
    fn evaluates_pep440_specifiers() {
        assert!(py_ok(">=3.8", "3.12"));
        assert!(!py_ok(">=3.8", "3.7"));
        assert!(py_ok(">=3.8,<4", "3.13") && !py_ok(">=3.8,<4", "4.0"));
        assert!(py_ok(">=2.7, !=3.0.*, !=3.1.*", "3.9") && !py_ok(">=2.7, !=3.0.*, !=3.1.*", "3.1.4"));
        assert!(py_ok("~=3.9", "3.12") && !py_ok("~=3.9", "3.8"));
        assert!(py_ok("==3.*", "3.11") && !py_ok("==3.*", "4"));
        assert!(py_ok(">3.7", "3.8"));
        assert!(parse_pep440("python3").is_none());
    }

    #[test]
    fn finds_minimum() {
        let min = |r: &str| minimum(&parse_npm(r).unwrap()).map(display);
        assert_eq!(min("^20.19 || ^22.12 || >=24").as_deref(), Some("20.19"));
        assert_eq!(min(">= 18").as_deref(), Some("18"));
        assert_eq!(min(">=14.17.0").as_deref(), Some("14.17"));
        assert_eq!(min("<20 || >=22"), None);
        assert_eq!(min(">0.8").as_deref(), Some("0.9"));
        let pymin = |s: &str| minimum(&parse_pep440(s).unwrap()).map(display);
        assert_eq!(pymin(">=3.8, <4").as_deref(), Some("3.8"));
        assert_eq!(pymin(">=2.7, !=3.0.*").as_deref(), Some("2.7"));
    }

    #[test]
    fn builds_requirement() {
        let r = evaluate("7.3.1", "node", Some("^20.19 || ^22.12 || >=24"), "18");
        assert_eq!(r.minimum.as_deref(), Some("20.19"));
        assert_eq!(r.target_ok, Some(false));
        // 宣言が無ければ制限なし
        let r = evaluate("1.0.0", "node", None, "18");
        assert_eq!((r.constraint, r.minimum, r.target_ok), (None, None, Some(true)));
        // 未設定なら判定しない
        assert_eq!(evaluate("1.0.0", "python", Some(">=3.8"), "").target_ok, None);
        // 読めない範囲は判定しない
        let r = evaluate("1.0.0", "node", Some("node >= weird"), "18");
        assert_eq!((r.minimum, r.target_ok), (None, None));
    }

    #[test]
    fn reads_engines_object_and_array() {
        let doc: Value = serde_json::json!({ "engines": { "node": ">= 18", "npm": ">=9" } });
        assert_eq!(node_engine(&doc).as_deref(), Some(">= 18"));
        let doc: Value = serde_json::json!({ "engines": ["node >= 0.4.0"] });
        assert_eq!(node_engine(&doc).as_deref(), Some(">= 0.4.0"));
        assert_eq!(node_engine(&serde_json::json!({})), None);
    }

    /// 実際のレジストリと endoflife.date に問い合わせる確認用テスト。通常は実行しない。
    /// `cargo test live_runtime -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_runtime() {
        use crate::model::RuntimeTargets;
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let http = Http::new(None, std::sync::Arc::new(crate::activity::ActivityLog::disabled()));
        let opts = CheckOptions {
            runtime_targets: RuntimeTargets { java: "17".into(), node: "18".into(), python: "3.8".into() },
            ..Default::default()
        };
        let mut vite = Component::new("v", Ecosystem::Npm, "vite-plugin-svelte");
        vite.group = Some("@sveltejs".into());
        let versions = ["3.1.2", "7.3.1"].map(String::from);
        for r in rt.block_on(requirements(&http, &vite, &versions, &opts)) {
            println!("{r:?}");
        }
        let django = Component::new("d", Ecosystem::Pypi, "django");
        for r in rt.block_on(requirements(&http, &django, &["4.2.7".into(), "5.2".into()], &opts)) {
            println!("{r:?}");
        }
        for s in rt.block_on(status(&http, &opts)) {
            println!("{} {} -> {:?} {:?} {:?}", s.runtime, s.version, s.eol.as_ref().and_then(|e| e.cycle.clone()), s.eol.map(|e| (e.status, e.eol_from)), s.error);
        }
    }

    #[test]
    fn builds_registry_urls() {
        assert_eq!(
            registry_url(NPM_BASE, &["@sveltejs/vite-plugin-svelte", "7.3.1"]),
            "https://registry.npmjs.org/@sveltejs%2Fvite-plugin-svelte/7.3.1"
        );
        assert_eq!(registry_url(PYPI_BASE, &["django", "4.2.7", "json"]), "https://pypi.org/pypi/django/4.2.7/json");
    }
}

//! 報告の出力（テキスト・JSON・SARIF 2.1.0）。

use serde_json::{json, Value};
use software_update_checker_core::model::{CheckResult, Component, FixAdvice, ImportResult, Severity, VulnInfo};
use software_update_checker_core::policy::{Finding, FindingKind, Policy};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

const TOOL: &str = "software-update-checker";
const INFO_URI: &str = "https://github.com/tamacat/dependency-auto-update-tool";
const EOL_RULE: &str = "end-of-life";
const DEPRECATED_RULE: &str = "deprecated-package";

pub struct Context<'a> {
    /// 報告に書く入力のパス
    pub input: &'a str,
    pub imported: &'a ImportResult,
    pub results: &'a [CheckResult],
    pub findings: &'a [Finding],
    pub policy: &'a Policy,
}

impl Context<'_> {
    fn component(&self, id: &str) -> Option<&Component> {
        self.imported.components.iter().find(|c| c.id == id)
    }

    fn result(&self, id: &str) -> Option<&CheckResult> {
        self.results.iter().find(|r| r.component_id == id)
    }

    fn rules(&self) -> String {
        let names: Vec<&str> = self.policy.fail_on.iter().map(|r| r.name()).collect();
        if names.is_empty() { "none".into() } else { names.join(",") }
    }
}

fn display_name(c: &Component) -> String {
    let name = match &c.group {
        Some(g) if c.ecosystem.purl_type() == Some("npm") => format!("{g}/{}", c.name),
        Some(g) => format!("{g}:{}", c.name),
        None => c.name.clone(),
    };
    match &c.version {
        Some(v) => format!("{name} {v}"),
        None => name,
    }
}

fn severity_name(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "Critical",
        Severity::High => "High",
        Severity::Medium => "Medium",
        Severity::Low => "Low",
        Severity::Unknown => "Unknown",
    }
}

fn cve(v: &VulnInfo) -> Option<&str> {
    std::iter::once(v.id.as_str()).chain(v.aliases.iter().map(String::as_str)).find(|a| a.starts_with("CVE-"))
}

/// 直せる更新先の説明（`2.25.4（同じ系列）` など）。
fn fix_text(fix: Option<&FixAdvice>) -> Option<String> {
    let fix = fix?;
    let mut parts: Vec<String> = Vec::new();
    for (label, v) in [("same series", &fix.in_series), ("same major", &fix.in_major), ("minimum", &fix.minimum)] {
        if let Some(v) = v {
            match parts.iter_mut().find(|p| p.starts_with(&format!("{v} ("))) {
                Some(p) => {
                    p.pop();
                    p.push_str(&format!(", {label})"));
                }
                None => parts.push(format!("{v} ({label})")),
            }
        }
    }
    (!parts.is_empty()).then(|| format!("upgrade to {}", parts.join(" or ")))
}

pub fn summary_line(ctx: &Context) -> String {
    let failing = ctx.findings.iter().filter(|f| f.failing()).count();
    let vulnerable = ctx.results.iter().filter(|r| !r.vulnerabilities.is_empty()).count();
    format!(
        "{} failing finding(s), {} other finding(s); {} of {} component(s) have vulnerabilities (fail on: {})",
        failing,
        ctx.findings.len() - failing,
        vulnerable,
        ctx.imported.components.len(),
        ctx.rules()
    )
}

// ---------------------------------------------------------------- テキスト

pub fn text(ctx: &Context) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{TOOL} {} - {} ({}, {} components)\n",
        env!("CARGO_PKG_VERSION"),
        ctx.input,
        ctx.imported.format,
        ctx.imported.components.len()
    );
    for f in ctx.findings {
        let Some(c) = ctx.component(&f.component_id) else { continue };
        let mark = if f.failing() { "FAIL" } else { "note" };
        let rules = if f.failing() {
            format!("  [{}]", f.matched.iter().map(|r| r.name()).collect::<Vec<_>>().join(", "))
        } else {
            String::new()
        };
        match &f.kind {
            FindingKind::Vulnerability(v) => {
                let mut tags = vec![format!("{}{}", severity_name(v.severity), v.score.map(|s| format!(" {s:.1}")).unwrap_or_default())];
                if v.malicious {
                    tags.insert(0, "MALICIOUS".into());
                }
                if v.kev.is_some() {
                    tags.push("KEV".into());
                }
                if let Some(e) = &v.epss {
                    // 99.99% を 100.0% と丸めないよう切り捨てる
                    tags.push(format!("EPSS {:.1}%", (e.score * 1000.0).floor() / 10.0));
                }
                let id = match cve(v) {
                    Some(cve) if cve != v.id => format!("{} ({cve})", v.id),
                    _ => v.id.clone(),
                };
                let _ = writeln!(out, "{mark}  {}  {id}  {}{rules}", display_name(c), tags.join("  "));
                if let Some(s) = &v.summary {
                    let _ = writeln!(out, "      {s}");
                }
                if let Some(fix) = fix_text(ctx.result(&c.id).and_then(|r| r.fix.as_ref())) {
                    let _ = writeln!(out, "      fix: {fix}");
                } else if !v.fixed_versions.is_empty() {
                    let _ = writeln!(out, "      fixed in: {}", v.fixed_versions.join(", "));
                }
            }
            FindingKind::Eol { product, cycle, eol_from } => {
                let _ = writeln!(
                    out,
                    "{mark}  {}  end of life: {product} {}{}{rules}",
                    display_name(c),
                    cycle.as_deref().unwrap_or(""),
                    eol_from.as_deref().map(|d| format!(" (since {d})")).unwrap_or_default()
                );
            }
            FindingKind::Deprecated { reason } => {
                let _ = writeln!(out, "{mark}  {}  deprecated: {reason}{rules}", display_name(c));
            }
        }
    }
    if ctx.findings.is_empty() {
        let _ = writeln!(out, "No findings.");
    }
    let _ = writeln!(out, "\n{}", summary_line(ctx));
    out
}

// ---------------------------------------------------------------- JSON

fn component_json(c: &Component) -> Value {
    json!({ "id": c.id, "ecosystem": c.ecosystem, "group": c.group, "name": c.name, "version": c.version, "purl": c.purl, "direct": c.direct })
}

pub fn json(ctx: &Context) -> String {
    let failing = ctx.findings.iter().filter(|f| f.failing()).count();
    let findings: Vec<Value> = ctx
        .findings
        .iter()
        .filter_map(|f| {
            let c = ctx.component(&f.component_id)?;
            let mut v = match &f.kind {
                FindingKind::Vulnerability(v) => json!({
                    "type": "vulnerability",
                    "vulnerability": v,
                    "fix": ctx.result(&c.id).and_then(|r| r.fix.clone()),
                }),
                FindingKind::Eol { product, cycle, eol_from } => {
                    json!({ "type": "eol", "product": product, "cycle": cycle, "eolFrom": eol_from })
                }
                FindingKind::Deprecated { reason } => json!({ "type": "deprecated", "reason": reason }),
            };
            v["component"] = component_json(c);
            v["failing"] = json!(f.failing());
            v["matchedRules"] = json!(f.matched.iter().map(|r| r.name()).collect::<Vec<_>>());
            Some(v)
        })
        .collect();
    let results: HashMap<&str, &CheckResult> = ctx.results.iter().map(|r| (r.component_id.as_str(), r)).collect();
    let components: Vec<Value> = ctx
        .imported
        .components
        .iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
            v["check"] = json!(results.get(c.id.as_str()));
            v
        })
        .collect();
    let doc = json!({
        "tool": { "name": TOOL, "version": env!("CARGO_PKG_VERSION") },
        "input": { "path": ctx.input, "format": ctx.imported.format, "projectName": ctx.imported.project_name, "warnings": ctx.imported.warnings },
        "summary": {
            "components": ctx.imported.components.len(),
            "findings": ctx.findings.len(),
            "failing": failing,
            "failOn": ctx.policy.fail_on.iter().map(|r| r.name()).collect::<Vec<_>>(),
        },
        "findings": findings,
        "components": components,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

// ---------------------------------------------------------------- SARIF

/// GitHub の Code scanning が深刻度の判定に使う値（0.0〜10.0）。
fn security_severity(v: &VulnInfo) -> Option<f64> {
    if v.malicious {
        return Some(10.0);
    }
    v.score.or(match v.severity {
        Severity::Critical => Some(9.5),
        Severity::High => Some(8.0),
        Severity::Medium => Some(5.5),
        Severity::Low => Some(2.0),
        Severity::Unknown => None,
    })
}

/// 入力ファイルの中で、その部品の名前が最初に出てくる行（見つからなければ 1 行目）。
fn line_of(content: Option<&str>, c: &Component) -> usize {
    let Some(content) = content else { return 1 };
    let name = c.name.to_ascii_lowercase();
    let quoted = format!("\"{name}\"");
    let lines: Vec<String> = content.lines().map(str::to_ascii_lowercase).collect();
    lines
        .iter()
        .position(|l| l.contains(&quoted) || l.contains(&format!("node_modules/{name}\"")))
        .or_else(|| lines.iter().position(|l| l.contains(&name)))
        .map_or(1, |i| i + 1)
}

pub fn sarif(ctx: &Context, input_content: Option<&str>) -> String {
    let mut rules: BTreeMap<String, Value> = BTreeMap::new();
    let mut results = Vec::new();
    for f in ctx.findings {
        let Some(c) = ctx.component(&f.component_id) else { continue };
        let name = display_name(c);
        let (rule_id, level, message) = match &f.kind {
            FindingKind::Vulnerability(v) => {
                let mut tags = vec!["security", "vulnerability"];
                if v.malicious {
                    tags.push("malicious");
                }
                if v.kev.is_some() {
                    tags.push("known-exploited");
                }
                let mut props = json!({ "tags": tags, "precision": "very-high" });
                if let Some(s) = security_severity(v) {
                    props["security-severity"] = json!(format!("{s:.1}"));
                }
                let summary = v.summary.clone().unwrap_or_else(|| v.id.clone());
                rules.entry(v.id.clone()).or_insert_with(|| {
                    json!({
                        "id": v.id,
                        "name": v.id,
                        "shortDescription": { "text": summary },
                        "fullDescription": { "text": format!("{summary} ({})", std::iter::once(v.id.as_str()).chain(v.aliases.iter().map(String::as_str)).collect::<Vec<_>>().join(", ")) },
                        "helpUri": v.url,
                        "help": { "text": format!("See {} for details.", v.url) },
                        "properties": props,
                    })
                });
                let level = if f.failing() {
                    "error"
                } else if v.severity >= Severity::Medium {
                    "warning"
                } else {
                    "note"
                };
                let mut msg = format!("{name} is affected by {}", v.id);
                if let Some(cve) = cve(v).filter(|c| *c != v.id) {
                    msg.push_str(&format!(" ({cve})"));
                }
                msg.push_str(&format!(": {summary}."));
                if v.malicious {
                    msg.push_str(" Reported as a malicious package: remove it rather than upgrade.");
                }
                if let Some(k) = &v.kev {
                    msg.push_str(&format!(" Listed in CISA KEV ({}).", k.cve));
                }
                if let Some(fix) = fix_text(ctx.result(&c.id).and_then(|r| r.fix.as_ref())) {
                    msg.push_str(&format!(" Fix: {fix}."));
                }
                (v.id.clone(), level, msg)
            }
            FindingKind::Eol { product, cycle, eol_from } => {
                rules.entry(EOL_RULE.into()).or_insert_with(|| {
                    json!({
                        "id": EOL_RULE,
                        "name": "EndOfLife",
                        "shortDescription": { "text": "The component has reached end of life" },
                        "help": { "text": "Upgrade to a supported release cycle. See https://endoflife.date/." },
                        "properties": { "tags": ["maintenance"] },
                    })
                });
                let since = eol_from.as_deref().map(|d| format!(" since {d}")).unwrap_or_default();
                let msg = format!("{name}: {product} {} has reached end of life{since}.", cycle.as_deref().unwrap_or(""));
                (EOL_RULE.into(), if f.failing() { "error" } else { "warning" }, msg)
            }
            FindingKind::Deprecated { reason } => {
                rules.entry(DEPRECATED_RULE.into()).or_insert_with(|| {
                    json!({
                        "id": DEPRECATED_RULE,
                        "name": "DeprecatedPackage",
                        "shortDescription": { "text": "The package version is deprecated" },
                        "help": { "text": "Replace or upgrade the deprecated package." },
                        "properties": { "tags": ["maintenance"] },
                    })
                });
                (DEPRECATED_RULE.into(), if f.failing() { "error" } else { "note" }, format!("{name} is deprecated: {reason}"))
            }
        };
        let package = c.purl.clone().unwrap_or_else(|| c.id.clone());
        results.push(json!({
            "ruleId": rule_id,
            "level": level,
            "message": { "text": message },
            "locations": [{
                "physicalLocation": {
                    "artifactLocation": { "uri": ctx.input },
                    "region": { "startLine": line_of(input_content, c) },
                },
                "logicalLocations": [{ "name": name, "kind": "package" }],
            }],
            // 同じ部品・同じ指摘を、実行をまたいで同じものとして扱うための指紋
            "partialFingerprints": { "packageRule/v1": format!("{package}|{rule_id}") },
        }));
    }
    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": TOOL,
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": INFO_URI,
                "rules": rules.into_values().collect::<Vec<_>>(),
            }},
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use software_update_checker_core::model::{Ecosystem, KevInfo};
    use software_update_checker_core::policy::{self, Rule};

    fn fixture() -> (ImportResult, Vec<CheckResult>) {
        let mut c = Component::new("npm:express@4.17.1", Ecosystem::Npm, "express");
        c.version = Some("4.17.1".into());
        c.purl = Some("pkg:npm/express@4.17.1".into());
        let v = VulnInfo {
            id: "GHSA-1".into(),
            aliases: vec!["CVE-2024-1".into()],
            summary: Some("Open redirect".into()),
            severity: Severity::Critical,
            score: Some(9.1),
            fixed_versions: vec!["4.19.2".into()],
            published: None,
            url: "https://osv.dev/vulnerability/GHSA-1".into(),
            malicious: false,
            kev: Some(KevInfo { cve: "CVE-2024-1".into(), date_added: None, due_date: None, ransomware: false, required_action: None }),
            epss: None,
        };
        let r = CheckResult {
            component_id: c.id.clone(),
            vulnerabilities: vec![v],
            fix: Some(FixAdvice { in_series: None, in_major: Some("4.19.2".into()), minimum: Some("4.19.2".into()), unfixed: vec![] }),
            ..Default::default()
        };
        let imported = ImportResult {
            source_path: "package-lock.json".into(),
            format: "npm-package-lock".into(),
            project_name: None,
            components: vec![c],
            warnings: vec![],
            includes_transitive: true,
            related_files: vec![],
        };
        (imported, vec![r])
    }

    #[test]
    fn writes_sarif_with_rule_location_and_severity() {
        let (imported, results) = fixture();
        let pol = Policy { fail_on: vec![Rule::Kev], ..Default::default() };
        let findings = policy::evaluate(&imported.components, &results, &pol);
        let ctx = Context { input: "app/package-lock.json", imported: &imported, results: &results, findings: &findings, policy: &pol };
        let lock = "{\n  \"packages\": {\n    \"node_modules/express\": {\n      \"version\": \"4.17.1\"\n";
        let doc: Value = serde_json::from_str(&sarif(&ctx, Some(lock))).unwrap();
        let run = &doc["runs"][0];
        assert_eq!(doc["version"], "2.1.0");
        assert_eq!(run["tool"]["driver"]["rules"][0]["id"], "GHSA-1");
        assert_eq!(run["tool"]["driver"]["rules"][0]["properties"]["security-severity"], "9.1");
        let r = &run["results"][0];
        assert_eq!(r["level"], "error");
        assert_eq!(r["locations"][0]["physicalLocation"]["artifactLocation"]["uri"], "app/package-lock.json");
        assert_eq!(r["locations"][0]["physicalLocation"]["region"]["startLine"], 3);
        let msg = r["message"]["text"].as_str().unwrap();
        assert!(msg.contains("CVE-2024-1") && msg.contains("CISA KEV") && msg.contains("4.19.2 (same major, minimum)"), "{msg}");
    }

    #[test]
    fn writes_text_and_json() {
        let (imported, results) = fixture();
        let pol = Policy { fail_on: vec![Rule::Severity(Severity::High)], ..Default::default() };
        let findings = policy::evaluate(&imported.components, &results, &pol);
        let ctx = Context { input: "package-lock.json", imported: &imported, results: &results, findings: &findings, policy: &pol };
        let t = text(&ctx);
        assert!(t.contains("FAIL  express 4.17.1  GHSA-1 (CVE-2024-1)  Critical 9.1  KEV  [high]"), "{t}");
        assert!(t.contains("fix: upgrade to 4.19.2 (same major, minimum)"));
        assert!(t.contains("1 failing finding(s)"));
        let j: Value = serde_json::from_str(&json(&ctx)).unwrap();
        assert_eq!(j["summary"]["failing"], 1);
        assert_eq!(j["findings"][0]["component"]["name"], "express");
        assert_eq!(j["findings"][0]["matchedRules"][0], "high");
        assert_eq!(j["components"][0]["check"]["vulnerabilities"][0]["id"], "GHSA-1");
    }
}

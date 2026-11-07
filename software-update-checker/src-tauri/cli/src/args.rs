//! 引数の解析（依存を増やさないよう手で書く）。

use software_update_checker_core::policy::{Policy, DEFAULT_FAIL_ON};
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: software-update-checker-cli <input> [options]

Checks the dependencies in <input> for known vulnerabilities, end of life and
deprecation, and exits with 1 when a finding matches --fail-on.

<input>   pom.xml, CycloneDX (JSON/XML), SPDX JSON, package-lock.json,
          pnpm-lock.yaml, yarn.lock, requirements.txt, poetry.lock, uv.lock,
          Cargo.lock, go.mod or gradle.lockfile

Options:
  --format <text|json|sarif>   Report format (default: text)
  -o, --output <file>          Write the report to a file instead of stdout
  --fail-on <rules>            Comma-separated rules that fail the run
                               (default: malicious,kev,critical)
                               malicious  malicious-package reports (OSV MAL-)
                               kev        in the CISA Known Exploited catalog
                               critical|high|medium|low  that severity or above
                               any        any vulnerability
                               eol        end of life
                               deprecated deprecated packages
                               none       never fail
  --ignore <ids>               Comma-separated vulnerability IDs to ignore
                               (GHSA, CVE, ... aliases match too)
  --direct-only                Only judge direct dependencies
  --no-fail                    Report only; always exit 0 unless an error occurs
  --supply-chain               Also fetch OpenSSF Scorecard and provenance
  --include-prerelease         Consider pre-releases as latest versions
  --disable-source <ids>       Comma-separated data sources not to contact
                               (deps.dev, osv, cisa-kev, epss, endoflife.date)
  --cache <file>               Read and write an HTTP cache file
  --lang <en|ja>               Message language (default: en)
  -v, --verbose                Show progress and requests on stderr
  -h, --help                   Show this help
  -V, --version                Show the version

Exit codes: 0 no failing findings, 1 failing findings, 2 usage or input error
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Sarif,
}

#[derive(Debug)]
pub struct Args {
    pub input: PathBuf,
    /// 報告に書く入力のパス（SARIF ではリポジトリからの相対パスとして使う）
    pub input_display: String,
    pub format: Format,
    pub output: Option<PathBuf>,
    pub policy: Policy,
    pub no_fail: bool,
    pub supply_chain: bool,
    pub include_prerelease: bool,
    pub disabled_sources: Vec<String>,
    pub cache: Option<PathBuf>,
    pub lang: String,
    pub verbose: bool,
}

/// 報告に書く入力のパス。相対パスはそのまま（SARIF ではリポジトリからの相対パスになる）、
/// 絶対パスは `file:///` の URI にする。
fn display_path(input: &str) -> String {
    let p = input.replace('\\', "/");
    let p = p.trim_start_matches("./");
    let bytes = p.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/' {
        format!("file:///{p}")
    } else if p.starts_with('/') {
        format!("file://{p}")
    } else {
        p.to_string()
    }
}

fn list(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// 引数を解析する。`--help` と `--version` は表示して `Ok(None)`。
pub fn parse(argv: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut input: Option<String> = None;
    let mut format = Format::Text;
    let mut output = None;
    let mut fail_on = DEFAULT_FAIL_ON.to_string();
    let mut ignore = Vec::new();
    let mut direct_only = false;
    let mut no_fail = false;
    let mut supply_chain = false;
    let mut include_prerelease = false;
    let mut disabled_sources = Vec::new();
    let mut cache = None;
    let mut lang = "en".to_string();
    let mut verbose = false;

    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        // `--name=value` も受け付ける
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next()).ok_or_else(|| format!("{name} needs a value"));
        match name.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("software-update-checker-cli {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--format" => {
                format = match value()?.as_str() {
                    "text" => Format::Text,
                    "json" => Format::Json,
                    "sarif" => Format::Sarif,
                    other => return Err(format!("unknown format: {other}")),
                }
            }
            "-o" | "--output" => output = Some(PathBuf::from(value()?)),
            "--fail-on" => fail_on = value()?,
            "--ignore" => ignore.extend(list(&value()?)),
            "--direct-only" => direct_only = true,
            "--no-fail" => no_fail = true,
            "--supply-chain" => supply_chain = true,
            "--include-prerelease" => include_prerelease = true,
            "--disable-source" => disabled_sources.extend(list(&value()?)),
            "--cache" => cache = Some(PathBuf::from(value()?)),
            "--lang" => lang = value()?,
            "-v" | "--verbose" => verbose = true,
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            _ if input.is_some() => return Err(format!("only one input file is supported: {arg}")),
            _ => input = Some(arg),
        }
    }
    let input = input.ok_or("no input file")?;
    let fail_on = Policy::parse_rules(&fail_on).map_err(|r| format!("unknown --fail-on rule: {r}"))?;
    Ok(Some(Args {
        input_display: display_path(&input),
        input: PathBuf::from(input),
        format,
        output,
        policy: Policy { fail_on, ignore: ignore.into_iter().collect(), direct_only },
        no_fail,
        supply_chain,
        include_prerelease,
        disabled_sources,
        cache,
        lang,
        verbose,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use software_update_checker_core::model::Severity;
    use software_update_checker_core::policy::Rule;

    fn parse_args(s: &str) -> Result<Option<Args>, String> {
        parse(s.split_whitespace().map(str::to_string))
    }

    #[test]
    fn parses_defaults_and_options() {
        let a = parse_args("./sub\\package-lock.json").unwrap().unwrap();
        assert_eq!(a.format, Format::Text);
        assert_eq!(a.input_display, "sub/package-lock.json");
        assert_eq!(a.policy.fail_on, vec![Rule::Malicious, Rule::Kev, Rule::Severity(Severity::Critical)]);
        assert!(!a.no_fail && !a.supply_chain && a.cache.is_none());

        let a = parse_args("bom.json --format=sarif -o out.sarif --fail-on high,eol --ignore GHSA-1,CVE-2 --direct-only --disable-source epss -v")
            .unwrap()
            .unwrap();
        assert_eq!(a.format, Format::Sarif);
        assert_eq!(a.output, Some(PathBuf::from("out.sarif")));
        assert_eq!(a.policy.fail_on, vec![Rule::Severity(Severity::High), Rule::Eol]);
        assert!(a.policy.ignore.contains("CVE-2") && a.policy.direct_only && a.verbose);
        assert_eq!(a.disabled_sources, vec!["epss"]);
    }

    #[test]
    fn makes_absolute_paths_uris() {
        assert_eq!(display_path("C:\\work\\bom.json"), "file:///C:/work/bom.json");
        assert_eq!(display_path("/home/u/bom.json"), "file:///home/u/bom.json");
        assert_eq!(display_path("./app/uv.lock"), "app/uv.lock");
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(parse_args("").is_err());
        assert!(parse_args("a b").is_err());
        assert!(parse_args("a --format xml").unwrap_err().contains("xml"));
        assert!(parse_args("a --fail-on severe").unwrap_err().contains("severe"));
        assert!(parse_args("a --output").unwrap_err().contains("needs a value"));
        assert!(parse_args("a --bogus").unwrap_err().contains("--bogus"));
    }
}

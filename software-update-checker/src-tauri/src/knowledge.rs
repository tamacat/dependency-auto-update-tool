//! 「どのライブラリのどのバージョンにどの Java が必要か」のナレッジファイル。
//!
//! jar から判定した結果を自動で記録し、利用者が手動で追加・修正した知見も同じ形式で持つ。
//! 手動の記録は、バージョン単位のほか系列（`1.4` → 1.4.x すべて）や groupId 全体（artifact `*`）
//! でも書ける。ファイルはチームで共有（共有フォルダや git）して、読み込み専用で参照できる。
//!
//! 参照の優先順位（上ほど優先）:
//! 1. 対象が具体的なもの（artifact 一致 > `*`、バージョン一致 > 系列 > 指定なし。系列は長い方）
//! 2. 手動の記録 > jar 判定の記録
//! 3. 手元のファイル > 共有ファイル（設定に並べた順）
//!
//! ナレッジに該当があれば通信しない（Maven Central を無効にしていても答えられる）。

use crate::version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FORMAT: &str = "software-update-checker/java-requirements";
pub const FORMAT_VERSION: u32 = 1;
/// 共有されたファイルを他の言語の利用者も読めるよう、説明は日英を併記する。
const COMMENT: &str = "ライブラリのバージョンごとに必要な Java の記録。source=jar はクラスファイルからの自動判定、source=manual は手動で登録した知見。artifact に \"*\" を書くと groupId 全体、version の代わりに series（例 \"1.4\"）を書くとその系列すべてに当てはまる。 / Java version required by each library version. source=jar is detected from class files, source=manual is knowledge registered by hand. artifact \"*\" applies to the whole groupId; series (e.g. \"1.4\") instead of version applies to the whole series.";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum RuleSource {
    /// jar のクラスファイルから自動判定
    Jar,
    /// 利用者が登録
    Manual,
}

fn maven() -> String {
    "maven".into()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JavaRule {
    #[serde(default = "maven")]
    pub ecosystem: String,
    pub group: String,
    /// `*` なら groupId 配下すべて
    pub artifact: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// `1.4` のような系列（その系列のすべてのバージョンに当てはまる）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// 必要な Java。`null` は「jar が無い・判定できない」の記録
    pub java: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_major: Option<u16>,
    pub source: RuleSource,
    /// 記録した日時（UTC の ISO 8601。手で書く場合は YYYY-MM-DD でもよい）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// 記録を一意に決める項目。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuleKey {
    pub ecosystem: String,
    pub group: String,
    pub artifact: String,
    pub version: Option<String>,
    pub series: Option<String>,
}

impl JavaRule {
    pub fn key(&self) -> RuleKey {
        RuleKey {
            ecosystem: self.ecosystem.clone(),
            group: self.group.clone(),
            artifact: self.artifact.clone(),
            version: self.version.clone(),
            series: self.series.clone(),
        }
    }

    /// このバージョンに当てはまるか。当てはまるなら具体性の点数（大きいほど具体的）。
    fn specificity(&self, group: &str, artifact: &str, ver: &str) -> Option<u32> {
        if self.ecosystem != "maven" || self.group != group {
            return None;
        }
        let artifact_score = match self.artifact.as_str() {
            a if a == artifact => 1000,
            "*" => 0,
            _ => return None,
        };
        let version_score = match (&self.version, &self.series) {
            (Some(v), _) if v == ver => 500,
            (Some(_), _) => return None,
            (None, Some(s)) => {
                let matches = ver == s || ver.strip_prefix(s.as_str()).is_some_and(|r| r.starts_with(['.', '-']));
                if !matches {
                    return None;
                }
                100 + s.len() as u32
            }
            (None, None) => 0,
        };
        Some(artifact_score + version_score)
    }

    /// 画面表示用の適用範囲。
    pub fn scope_label(&self) -> String {
        let target = if self.artifact == "*" { tr!("{} 配下すべて", "everything under {}", self.group) } else { self.artifact.clone() };
        match (&self.version, &self.series) {
            (Some(v), _) => format!("{target} {v}"),
            (None, Some(s)) => tr!("{target} {s} 系", "{target} {s}.x"),
            (None, None) => tr!("{target} 全バージョン", "{target} all versions"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeFile {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    pub format: String,
    /// ファイル形式の版（記録の `version` とは別）
    pub format_version: u32,
    #[serde(default)]
    pub entries: Vec<JavaRule>,
}

impl KnowledgeFile {
    fn new(entries: Vec<JavaRule>) -> Self {
        Self { comment: Some(COMMENT.into()), format: FORMAT.into(), format_version: FORMAT_VERSION, entries }
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| tr!("{} を読めません: {e}", "Cannot read {}: {e}", path.display()))?;
        let file: KnowledgeFile = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|e| tr!("{} の形式が不正です: {e}", "{} is malformed: {e}", path.display()))?;
        if file.format != FORMAT {
            return Err(tr!(
                "{} は Java 要件のナレッジファイルではありません（format: {}）",
                "{} is not a Java requirements knowledge file (format: {})",
                path.display(),
                file.format
            ));
        }
        if file.format_version > FORMAT_VERSION {
            return Err(tr!(
                "{} は新しい形式（formatVersion {}）です。このツールを更新してください",
                "{} uses a newer format (formatVersion {}). Please update this tool",
                path.display(),
                file.format_version
            ));
        }
        Ok(file)
    }

    /// 差分が見やすいよう、並びを固定して書き出す（書きかけのファイルを残さないよう置き換えで保存）。
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let mut sorted = self.clone();
        sorted.entries.sort_by(|a, b| {
            (&a.group, &a.artifact)
                .cmp(&(&b.group, &b.artifact))
                .then_with(|| version::compare(a.series.as_deref().or(a.version.as_deref()).unwrap_or(""), b.series.as_deref().or(b.version.as_deref()).unwrap_or("")))
                .then_with(|| a.version.is_some().cmp(&b.version.is_some()))
        });
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&sorted).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json + "\n").map_err(|e| tr!("{} に書き込めません: {e}", "Cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| tr!("{} に保存できません: {e}", "Cannot save {}: {e}", path.display()))
    }
}

/// どこの記録から答えたか。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Origin {
    Local,
    Shared { path: String },
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SharedFile {
    pub path: String,
    pub entries: Vec<JavaRule>,
    pub error: Option<String>,
}

pub fn load_shared(paths: &[String]) -> Vec<SharedFile> {
    paths
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| match KnowledgeFile::read(Path::new(p)) {
            Ok(f) => SharedFile { path: p.clone(), entries: f.entries, error: None },
            Err(e) => SharedFile { path: p.clone(), entries: Vec::new(), error: Some(e) },
        })
        .collect()
}

/// 当てはまる記録のうち最優先のもの。
pub fn best_match(
    local: &[JavaRule],
    shared: &[SharedFile],
    group: &str,
    artifact: &str,
    ver: &str,
) -> Option<(JavaRule, Origin)> {
    let candidates = local
        .iter()
        .map(|r| (r, Origin::Local, 0usize))
        .chain(
            shared
                .iter()
                .enumerate()
                .flat_map(|(i, f)| f.entries.iter().map(move |r| (r, Origin::Shared { path: f.path.clone() }, i + 1))),
        );
    candidates
        .filter_map(|(r, origin, file_rank)| r.specificity(group, artifact, ver).map(|s| (s, r, origin, file_rank)))
        .max_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| (a.1.source == RuleSource::Manual).cmp(&(b.1.source == RuleSource::Manual)))
                .then_with(|| b.3.cmp(&a.3))
        })
        .map(|(_, r, origin, _)| (r.clone(), origin))
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeSummary {
    pub added: usize,
    pub updated: usize,
    /// 手元の手動記録と衝突したため取り込まなかった件数
    pub skipped: usize,
}

/// 手元のナレッジファイル（自動記録と手動記録）。
pub struct KnowledgeStore {
    path: Option<PathBuf>,
    entries: Mutex<Vec<JavaRule>>,
    load_error: Option<String>,
}

impl KnowledgeStore {
    pub fn open(path: Option<PathBuf>) -> Self {
        let (entries, load_error) = match &path {
            Some(p) if p.exists() => match KnowledgeFile::read(p) {
                Ok(f) => (f.entries, None),
                // 壊れたファイルを上書きしないよう、読めなかったときは保存もしない
                Err(e) => (Vec::new(), Some(e)),
            },
            _ => (Vec::new(), None),
        };
        Self { path, entries: Mutex::new(entries), load_error }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn entries(&self) -> Vec<JavaRule> {
        self.entries.lock().map(|e| e.clone()).unwrap_or_default()
    }

    fn save(&self, entries: &[JavaRule]) -> Result<(), String> {
        if let Some(e) = &self.load_error {
            return Err(tr!(
                "ナレッジファイルを読めなかったため保存しません（{e}）",
                "Not saving because the knowledge file could not be read ({e})"
            ));
        }
        match &self.path {
            Some(p) => KnowledgeFile::new(entries.to_vec()).write(p),
            None => Ok(()),
        }
    }

    fn modify<T>(&self, f: impl FnOnce(&mut Vec<JavaRule>) -> T) -> Result<T, String> {
        let mut entries = self.entries.lock().map_err(|_| tr!("ナレッジの状態を読めません", "Cannot read the knowledge state"))?;
        let result = f(&mut entries);
        self.save(&entries)?;
        Ok(result)
    }

    /// jar 判定の結果を記録する。同じ対象の手動記録があれば上書きしない。
    pub fn record_detection(&self, rule: JavaRule) -> Result<(), String> {
        self.modify(|entries| match entries.iter_mut().find(|e| e.key() == rule.key()) {
            Some(existing) if existing.source == RuleSource::Manual => {}
            Some(existing) => *existing = rule,
            None => entries.push(rule),
        })
    }

    /// 手動の追加・修正。`original` を渡すとその記録を置き換える（対象を変える編集）。
    pub fn upsert(&self, rule: JavaRule, original: Option<RuleKey>) -> Result<(), String> {
        validate(&rule)?;
        self.modify(|entries| {
            if let Some(key) = original {
                entries.retain(|e| e.key() != key);
            }
            entries.retain(|e| e.key() != rule.key());
            entries.push(rule);
        })
    }

    pub fn delete(&self, key: &RuleKey) -> Result<bool, String> {
        self.modify(|entries| {
            let before = entries.len();
            entries.retain(|e| &e.key() != key);
            before != entries.len()
        })
    }

    /// 他のファイルの記録を取り込む。手元の手動記録は上書きしない。
    pub fn merge(&self, incoming: Vec<JavaRule>) -> Result<MergeSummary, String> {
        self.modify(|entries| {
            let mut summary = MergeSummary::default();
            for rule in incoming {
                if validate(&rule).is_err() {
                    summary.skipped += 1;
                    continue;
                }
                match entries.iter_mut().find(|e| e.key() == rule.key()) {
                    None => {
                        entries.push(rule);
                        summary.added += 1;
                    }
                    Some(existing) if existing.source == RuleSource::Manual && rule.source != RuleSource::Manual => {
                        summary.skipped += 1;
                    }
                    Some(existing) if *existing == rule => {}
                    Some(existing) if existing.source == RuleSource::Manual => summary.skipped += 1,
                    Some(existing) => {
                        *existing = rule;
                        summary.updated += 1;
                    }
                }
            }
            summary
        })
    }

    pub fn export(&self, path: &Path, include_detected: bool) -> Result<usize, String> {
        let entries: Vec<JavaRule> = self
            .entries()
            .into_iter()
            .filter(|e| include_detected || e.source == RuleSource::Manual)
            .collect();
        let count = entries.len();
        KnowledgeFile::new(entries).write(path)?;
        Ok(count)
    }
}

fn validate(rule: &JavaRule) -> Result<(), String> {
    if rule.group.trim().is_empty() || rule.artifact.trim().is_empty() {
        return Err(tr!("groupId と artifactId（全体なら *）は必須です", "groupId and artifactId (* for all) are required"));
    }
    if rule.version.is_some() && rule.series.is_some() {
        return Err(tr!("version と series はどちらか一方だけ指定してください", "Specify either version or series, not both"));
    }
    if let Some(j) = &rule.java {
        let ok = !j.is_empty() && j.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        if !ok {
            return Err(tr!(
                "Java のバージョン「{j}」は数字で指定してください（例: 8、11、17）",
                "Java version \"{j}\" must be a number (e.g. 8, 11, 17)"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(artifact: &str, version: Option<&str>, series: Option<&str>, java: &str, source: RuleSource) -> JavaRule {
        JavaRule {
            ecosystem: "maven".into(),
            group: "ch.qos.logback".into(),
            artifact: artifact.into(),
            version: version.map(str::to_string),
            series: series.map(str::to_string),
            java: Some(java.into()),
            class_major: None,
            source,
            checked_at: None,
            note: None,
        }
    }

    #[test]
    fn picks_most_specific_then_manual_then_local() {
        let local = vec![
            rule("*", None, Some("1.4"), "11", RuleSource::Manual),
            rule("logback-core", Some("1.3.16"), None, "8", RuleSource::Jar),
        ];
        let shared = vec![SharedFile {
            path: "team.json".into(),
            entries: vec![rule("logback-core", None, Some("1.5"), "11", RuleSource::Manual)],
            error: None,
        }];

        let (r, o) = best_match(&local, &shared, "ch.qos.logback", "logback-core", "1.3.16").unwrap();
        assert_eq!((r.java.as_deref(), o), (Some("8"), Origin::Local));

        // 系列ルール（groupId 全体）
        let (r, _) = best_match(&local, &shared, "ch.qos.logback", "logback-classic", "1.4.14").unwrap();
        assert_eq!(r.series.as_deref(), Some("1.4"));

        // 共有ファイルの artifact 指定の系列ルール
        let (r, o) = best_match(&local, &shared, "ch.qos.logback", "logback-core", "1.5.38").unwrap();
        assert_eq!((r.java.as_deref(), o), (Some("11"), Origin::Shared { path: "team.json".into() }));

        // "1.4" は "1.40.0" に当てはまらない
        assert!(best_match(&local, &shared, "ch.qos.logback", "logback-core", "1.40.0").is_none());
        assert!(best_match(&local, &shared, "org.other", "x", "1.4.0").is_none());
    }

    #[test]
    fn manual_beats_detection_at_same_specificity() {
        let local = vec![
            rule("logback-core", Some("1.3.16"), None, "8", RuleSource::Jar),
        ];
        let shared = vec![SharedFile {
            path: "team.json".into(),
            entries: vec![rule("logback-core", Some("1.3.16"), None, "9", RuleSource::Manual)],
            error: None,
        }];
        let (r, _) = best_match(&local, &shared, "ch.qos.logback", "logback-core", "1.3.16").unwrap();
        assert_eq!(r.java.as_deref(), Some("9"));
    }

    #[test]
    fn store_keeps_manual_rules_and_round_trips() {
        let dir = std::env::temp_dir().join(format!("suc-knowledge-{}", std::process::id()));
        let path = dir.join("java-requirements.json");
        let store = KnowledgeStore::open(Some(path.clone()));

        store.upsert(rule("logback-core", Some("1.3.16"), None, "8", RuleSource::Manual), None).unwrap();
        // 手動記録は自動判定で上書きされない
        store.record_detection(rule("logback-core", Some("1.3.16"), None, "11", RuleSource::Jar)).unwrap();
        store.record_detection(rule("logback-core", Some("1.4.14"), None, "11", RuleSource::Jar)).unwrap();
        assert_eq!(store.entries().len(), 2);

        let reopened = KnowledgeStore::open(Some(path.clone()));
        let e = reopened.entries();
        assert_eq!(e.len(), 2);
        assert_eq!(e.iter().find(|r| r.version.as_deref() == Some("1.3.16")).unwrap().java.as_deref(), Some("8"));

        // 取り込み: 新規は追加、手動記録との衝突は取り込まない
        let summary = reopened
            .merge(vec![
                rule("*", None, Some("1.5"), "11", RuleSource::Manual),
                rule("logback-core", Some("1.3.16"), None, "17", RuleSource::Manual),
                rule("logback-core", Some("1.4.14"), None, "11", RuleSource::Jar),
            ])
            .unwrap();
        assert_eq!(summary, MergeSummary { added: 1, updated: 0, skipped: 1 });

        // 書き出し（手動のみ）
        let out = dir.join("export.json");
        assert_eq!(reopened.export(&out, false).unwrap(), 2);
        let exported = KnowledgeFile::read(&out).unwrap();
        assert!(exported.entries.iter().all(|r| r.source == RuleSource::Manual));

        assert!(reopened.delete(&rule("*", None, Some("1.5"), "11", RuleSource::Manual).key()).unwrap());
        assert_eq!(reopened.entries().len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bundled_example_is_valid_and_answers_logback_series() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/java-requirements.example.json");
        let file = KnowledgeFile::read(&path).unwrap();
        assert!(file.entries.iter().all(|r| validate(r).is_ok()));
        let shared = vec![SharedFile { path: "example".into(), entries: file.entries, error: None }];
        let java = |a: &str, v: &str| best_match(&[], &shared, "ch.qos.logback", a, v).and_then(|(r, _)| r.java);
        assert_eq!(java("logback-classic", "1.3.14").as_deref(), Some("8"));
        assert_eq!(java("logback-core", "1.4.0").as_deref(), Some("11"));
    }

    #[test]
    fn rejects_invalid_rules_and_foreign_files() {
        let mut r = rule("x", Some("1.0"), Some("1"), "8", RuleSource::Manual);
        assert!(validate(&r).is_err());
        r.series = None;
        r.java = Some("Java 8".into());
        assert!(validate(&r).is_err());
        r.java = Some("1.8".into());
        assert!(validate(&r).is_ok());

        let dir = std::env::temp_dir().join(format!("suc-knowledge-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("other.json");
        std::fs::write(&p, r#"{"format":"something-else","formatVersion":1,"entries":[]}"#).unwrap();
        assert!(KnowledgeFile::read(&p).unwrap_err().contains("ナレッジファイルではありません"));
        // 壊れたファイルは上書きしない
        std::fs::write(&p, "not json").unwrap();
        let store = KnowledgeStore::open(Some(p.clone()));
        assert!(store.load_error().is_some());
        assert!(store.upsert(rule("x", Some("1"), None, "8", RuleSource::Manual), None).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "not json");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

//! Maven の jar から、そのバージョンの実行に必要な Java を判定する。
//!
//! logback（1.3 系は Java 8、1.4 系以降は Java 11）のように、同じメジャーバージョンの中でも
//! 系列によって必要な Java が変わり、更新すると動かなくなることがある。
//! POM の記述（maven.compiler.release など）は親 POM 依存や Ant ビルドで取れないことが多いため、
//! 実際のクラスファイルのバージョンを読む。
//!
//! jar 全体はダウンロードせず、HTTP の Range 指定で
//! ①末尾（ZIP の中央ディレクトリ）と ②数個のクラスファイルだけを取得する。
//! 判定結果はナレッジファイル（[`crate::knowledge`]）に記録し、次からは通信せずに答える。

use crate::http::Http;
use crate::knowledge::{self, JavaRule, KnowledgeStore, Origin, RuleSource, SharedFile};
use crate::model::JavaRequirement;

const REPO: &str = "https://repo1.maven.org/maven2";
const TAIL_BYTES: u64 = 64 * 1024;
const SAMPLE_CLASSES: usize = 3;
const MAX_CLASS_COMPRESSED: u64 = 512 * 1024;
/// クラスファイルは先頭 8 バイト（マジックナンバーとバージョン）しか使わない。
/// 細工した jar による展開爆弾（圧縮 512KB → 数百 MB）を防ぐため、これ以上は展開しない。
const MAX_INFLATED: usize = 64;
/// 中央ディレクトリ（jar の目次）の大きさの上限。通常の jar は数百 KB 以下。
const MAX_CENTRAL_DIRECTORY: u64 = 8 * 1024 * 1024;

/// クラスファイルのメジャーバージョンを Java のバージョン表記にする。
pub fn java_version_for_class_major(major: u16) -> Option<String> {
    Some(match major {
        45..=48 => format!("1.{}", major - 44),
        49..=51 => (major - 44).to_string(),
        52..=200 => (major - 44).to_string(),
        _ => return None,
    })
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

#[derive(Debug, PartialEq)]
struct ZipEntry {
    name: String,
    method: u16,
    compressed_size: u64,
    local_offset: u64,
}

/// ZIP 末尾の End of Central Directory から (中央ディレクトリの位置, サイズ) を得る。
fn find_central_directory(tail: &[u8]) -> Option<(u64, u64)> {
    if tail.len() < 22 {
        return None;
    }
    let pos = (0..=tail.len() - 22).rev().find(|&i| u32_at(tail, i) == Some(0x0605_4b50))?;
    let size = u32_at(tail, pos + 12)?;
    let offset = u32_at(tail, pos + 16)?;
    if size == u32::MAX || offset == u32::MAX {
        return None; // ZIP64 は未対応（4GB を超える jar は想定しない）
    }
    Some((u64::from(offset), u64::from(size)))
}

fn parse_central_directory(cd: &[u8]) -> Vec<ZipEntry> {
    let mut entries = Vec::new();
    let mut i = 0;
    while u32_at(cd, i) == Some(0x0201_4b50) {
        let (Some(method), Some(csize), Some(nlen), Some(elen), Some(clen), Some(offset)) = (
            u16_at(cd, i + 10),
            u32_at(cd, i + 20),
            u16_at(cd, i + 28),
            u16_at(cd, i + 30),
            u16_at(cd, i + 32),
            u32_at(cd, i + 42),
        ) else {
            break;
        };
        let name_end = i + 46 + nlen as usize;
        let Some(name) = cd.get(i + 46..name_end) else { break };
        entries.push(ZipEntry {
            name: String::from_utf8_lossy(name).into_owned(),
            method,
            compressed_size: u64::from(csize),
            local_offset: u64::from(offset),
        });
        i = name_end + elen as usize + clen as usize;
    }
    entries
}

/// 判定に使うクラス。module-info など Java 9 以降で別途コンパイルされがちなものは除く。
fn sample_classes(entries: &[ZipEntry]) -> Vec<&ZipEntry> {
    entries
        .iter()
        .filter(|e| {
            e.name.ends_with(".class")
                && !e.name.starts_with("META-INF/")
                && !e.name.ends_with("module-info.class")
                && !e.name.ends_with("package-info.class")
                && e.compressed_size <= MAX_CLASS_COMPRESSED
        })
        .take(SAMPLE_CLASSES)
        .collect()
}

/// ローカルヘッダー付きのエントリ本体からクラスファイルのメジャーバージョンを読む。
fn class_major_from_local_entry(local: &[u8], entry: &ZipEntry) -> Option<u16> {
    if u32_at(local, 0) != Some(0x0403_4b50) {
        return None;
    }
    let start = 30 + u16_at(local, 26)? as usize + u16_at(local, 28)? as usize;
    let end = start.checked_add(usize::try_from(entry.compressed_size).ok()?)?;
    let data = local.get(start..end)?;
    let class = match entry.method {
        0 => data.get(..MAX_INFLATED.min(data.len()))?.to_vec(),
        // 上限に達したら、そこまでに展開できた分（先頭部分）を使う
        8 => match miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_INFLATED) {
            Ok(v) => v,
            Err(e) => e.output,
        },
        _ => return None,
    };
    if class.get(0..4)? != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }
    Some(u16::from_be_bytes(class.get(6..8)?.try_into().ok()?))
}

fn jar_url(group: &str, artifact: &str, version: &str) -> String {
    format!("{REPO}/{}/{artifact}/{version}/{artifact}-{version}.jar", group.replace('.', "/"))
}

/// jar を部分取得して、サンプルしたクラスのうち最大のメジャーバージョンを返す。
/// jar が存在しなければ `Ok(None)`。
async fn read_class_major(http: &Http, url: &str) -> Result<Option<u16>, String> {
    let tail = http.get_range(url, &format!("-{TAIL_BYTES}")).await?;
    let (tail_start, tail_body) = match tail.status {
        206 => {
            let total = tail.total_size.ok_or_else(|| tr!("Content-Range がありません", "No Content-Range header"))?;
            (total.saturating_sub(tail.body.len() as u64), tail.body)
        }
        200 => (0, tail.body), // Range 非対応で全体が返った
        404 => return Ok(None),
        s => return Err(format!("HTTP {s}")),
    };

    let (cd_offset, cd_size) = find_central_directory(&tail_body).ok_or_else(|| tr!("jar の目次を読めません", "Cannot read the jar directory"))?;
    if cd_size == 0 || cd_size > MAX_CENTRAL_DIRECTORY {
        return Err(tr!("jar の目次の大きさが不正です（{cd_size} バイト）", "Invalid jar directory size ({cd_size} bytes)"));
    }
    let truncated = || tr!("jar の目次が途中で切れています", "The jar directory is truncated");
    let cd = if cd_offset >= tail_start {
        let from = usize::try_from(cd_offset - tail_start).map_err(|_| truncated())?;
        let to = from.checked_add(usize::try_from(cd_size).map_err(|_| truncated())?).ok_or_else(truncated)?;
        tail_body.get(from..to).ok_or_else(truncated)?.to_vec()
    } else {
        let last = cd_offset.checked_add(cd_size - 1).ok_or_else(truncated)?;
        http.get_range(url, &format!("{cd_offset}-{last}")).await?.body
    };

    let entries = parse_central_directory(&cd);
    let mut best: Option<u16> = None;
    for entry in sample_classes(&entries) {
        // ローカルヘッダーの拡張領域は中央ディレクトリと長さが違うことがあるので余裕を持たせる
        let len = 30 + entry.name.len() as u64 + 1024 + entry.compressed_size;
        let Some(last) = entry.local_offset.checked_add(len - 1) else { continue };
        let local = http.get_range(url, &format!("{}-{last}", entry.local_offset)).await?;
        if let Some(major) = class_major_from_local_entry(&local.body, entry) {
            best = Some(best.map_or(major, |b| b.max(major)));
        }
    }
    match best {
        Some(m) => Ok(Some(m)),
        None if entries.iter().any(|e| e.name.ends_with(".class")) => Err(tr!("クラスファイルを読めません", "Cannot read class files")),
        None => Err(tr!("jar にクラスファイルがありません", "The jar contains no class files")),
    }
}

/// jar から判定した結果。`class_major` が `None` なら jar が存在しない。
pub struct Detection {
    pub class_major: Option<u16>,
    pub java: Option<String>,
}

pub async fn detect(http: &Http, group: &str, artifact: &str, version: &str) -> Result<Detection, String> {
    let major = read_class_major(http, &jar_url(group, artifact, version)).await?;
    Ok(Detection { class_major: major, java: major.and_then(java_version_for_class_major) })
}

/// ナレッジを引き、無ければ（取得が許されていれば）jar から判定してナレッジに記録する。
pub async fn resolve(
    http: &Http,
    store: &KnowledgeStore,
    shared: &[SharedFile],
    group: &str,
    artifact: &str,
    version: &str,
    can_fetch: bool,
) -> JavaRequirement {
    if let Some((rule, origin)) = knowledge::best_match(&store.entries(), shared, group, artifact, version) {
        let (origin, origin_file) = match origin {
            Origin::Local => ("local", None),
            Origin::Shared { path } => ("shared", Some(path)),
        };
        return JavaRequirement {
            version: version.into(),
            java: rule.java.clone(),
            class_major: rule.class_major,
            note: rule.note.clone().or_else(|| rule.java.is_none().then(|| tr!("jar がありません（記録済み）", "No jar (recorded)"))),
            origin: origin.into(),
            rule_source: Some(match rule.source {
                RuleSource::Jar => "jar".into(),
                RuleSource::Manual => "manual".into(),
            }),
            rule_scope: Some(rule.scope_label()),
            origin_file,
        };
    }
    let none = |note: String| JavaRequirement {
        version: version.into(),
        java: None,
        class_major: None,
        note: Some(note),
        origin: "none".into(),
        rule_source: None,
        rule_scope: None,
        origin_file: None,
    };
    if !can_fetch {
        return none(tr!(
            "ナレッジに記録がありません（Maven Central が無効のため判定していません）",
            "Not in the knowledge base (not detected because Maven Central is disabled)"
        ));
    }
    match detect(http, group, artifact, version).await {
        Ok(d) => {
            let rule = JavaRule {
                ecosystem: "maven".into(),
                group: group.into(),
                artifact: artifact.into(),
                version: Some(version.into()),
                series: None,
                java: d.java.clone(),
                class_major: d.class_major,
                source: RuleSource::Jar,
                checked_at: Some(crate::activity::iso_utc(crate::activity::now_millis())),
                note: d.class_major.is_none().then(|| tr!("jar がありません（pom のみの成果物など）", "No jar (e.g. a pom-only artifact)")),
            };
            let note = rule.note.clone();
            if let Err(e) = store.record_detection(rule) {
                http.log().info(tr!("Java 要件をナレッジに記録できませんでした: {e}", "Could not record the Java requirement: {e}"));
            }
            JavaRequirement {
                version: version.into(),
                java: d.java,
                class_major: d.class_major,
                note,
                origin: "detected".into(),
                rule_source: Some("jar".into()),
                rule_scope: None,
                origin_file: None,
            }
        }
        // 通信失敗などは記録しない（次回また判定する）
        Err(e) => none(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 無圧縮エントリ 1 つだけの ZIP を作る。
    fn tiny_jar(name: &str, data: &[u8]) -> Vec<u8> {
        let mut zip = Vec::new();
        let n = name.len() as u16;
        let sz = data.len() as u32;
        // ローカルヘッダー
        zip.extend(0x0403_4b50u32.to_le_bytes());
        zip.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method=0, time, date, crc
        zip.extend(sz.to_le_bytes());
        zip.extend(sz.to_le_bytes());
        zip.extend(n.to_le_bytes());
        zip.extend(0u16.to_le_bytes());
        zip.extend(name.as_bytes());
        zip.extend(data);
        // 中央ディレクトリ
        let cd_offset = zip.len() as u32;
        zip.extend(0x0201_4b50u32.to_le_bytes());
        zip.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // made by, needed, flags, method, time, date, crc
        zip.extend(sz.to_le_bytes());
        zip.extend(sz.to_le_bytes());
        zip.extend(n.to_le_bytes());
        zip.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // extra, comment, disk, int attr, ext attr
        zip.extend(0u32.to_le_bytes()); // local header offset
        zip.extend(name.as_bytes());
        let cd_size = zip.len() as u32 - cd_offset;
        // EOCD
        zip.extend(0x0605_4b50u32.to_le_bytes());
        zip.extend([0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend(cd_size.to_le_bytes());
        zip.extend(cd_offset.to_le_bytes());
        zip.extend([0, 0]);
        zip
    }

    #[test]
    fn reads_class_major_from_zip_structures() {
        let class = [0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 55, 1, 2, 3];
        let jar = tiny_jar("ch/qos/logback/core/Foo.class", &class);
        let (offset, size) = find_central_directory(&jar).unwrap();
        let entries = parse_central_directory(&jar[offset as usize..(offset + size) as usize]);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "ch/qos/logback/core/Foo.class");
        let picked = sample_classes(&entries);
        assert_eq!(class_major_from_local_entry(&jar[picked[0].local_offset as usize..], picked[0]), Some(55));
    }

    #[test]
    fn inflates_only_the_class_header_of_a_compression_bomb() {
        // 0 が 50MB 続くクラス（圧縮すると 50KB 程度）。先頭だけ展開して止まること
        let mut class = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 61];
        class.resize(50 * 1024 * 1024, 0);
        let compressed = miniz_oxide::deflate::compress_to_vec(&class, 6);
        assert!(compressed.len() < 512 * 1024);
        let entry = ZipEntry {
            name: "a/B.class".into(),
            method: 8,
            compressed_size: compressed.len() as u64,
            local_offset: 0,
        };
        let mut local = 0x0403_4b50u32.to_le_bytes().to_vec();
        local.resize(26, 0);
        local.extend((entry.name.len() as u16).to_le_bytes());
        local.extend(0u16.to_le_bytes());
        local.extend(entry.name.as_bytes());
        local.extend(&compressed);
        assert_eq!(class_major_from_local_entry(&local, &entry), Some(61));
    }

    #[test]
    fn skips_module_info_and_metadata() {
        let entries = vec![
            ZipEntry { name: "META-INF/MANIFEST.MF".into(), method: 8, compressed_size: 10, local_offset: 0 },
            ZipEntry { name: "module-info.class".into(), method: 8, compressed_size: 10, local_offset: 0 },
            ZipEntry { name: "META-INF/versions/11/a/B.class".into(), method: 8, compressed_size: 10, local_offset: 0 },
            ZipEntry { name: "a/C.class".into(), method: 8, compressed_size: 10, local_offset: 0 },
        ];
        let picked = sample_classes(&entries);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].name, "a/C.class");
    }

    #[test]
    fn maps_class_versions_to_java() {
        assert_eq!(java_version_for_class_major(52).as_deref(), Some("8"));
        assert_eq!(java_version_for_class_major(55).as_deref(), Some("11"));
        assert_eq!(java_version_for_class_major(61).as_deref(), Some("17"));
        assert_eq!(java_version_for_class_major(49).as_deref(), Some("5"));
        assert_eq!(java_version_for_class_major(48).as_deref(), Some("1.4"));
        assert_eq!(java_version_for_class_major(10), None);
    }

    /// 実際に Maven Central へ問い合わせる確認用テスト。通常は実行しない。
    /// `cargo test live_java -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_java_detection_is_recorded_and_reused() {
        use crate::activity::ActivityLog;
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("suc-live-java-{}", std::process::id()));
        let store = KnowledgeStore::open(Some(dir.join("java-requirements.json")));
        let log = Arc::new(ActivityLog::disabled());
        let http = Http::new(None, log.clone());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

        let first = rt.block_on(resolve(&http, &store, &[], "ch.qos.logback", "logback-core", "1.4.14", true));
        assert_eq!((first.java.as_deref(), first.origin.as_str()), (Some("11"), "detected"));
        let sent = log.recent().iter().filter(|e| e.outcome == "network").count();
        println!("1 回目: Java {:?}、送信 {sent} 件", first.java);

        let second = rt.block_on(resolve(&http, &store, &[], "ch.qos.logback", "logback-core", "1.4.14", true));
        assert_eq!((second.java.as_deref(), second.origin.as_str()), (Some("11"), "local"));
        assert_eq!(log.recent().iter().filter(|e| e.outcome == "network").count(), sent, "2 回目は通信しない");
        println!("2 回目: Java {:?}（{}）、追加の送信なし", second.java, second.rule_scope.unwrap_or_default());
        println!("{}", std::fs::read_to_string(dir.join("java-requirements.json")).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn builds_jar_url() {
        assert_eq!(
            jar_url("ch.qos.logback", "logback-core", "1.3.16"),
            "https://repo1.maven.org/maven2/ch/qos/logback/logback-core/1.3.16/logback-core-1.3.16.jar"
        );
    }
}

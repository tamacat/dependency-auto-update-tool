//! このツールが情報を取りに行く先（データソース）の一覧。
//!
//! HTTP 通信は [`crate::http::Http`] がここに登録されたホストにしか行わない。
//! 利用者が設定で無効にしたデータソースのホストへの通信は、送信前に遮断して
//! 通信ログに「遮断」として記録する。新しい取得先を使うときは必ずここに登録すること。

use crate::i18n::{self, Language};
use serde::Serialize;

/// 日本語と英語の文言。
#[derive(Clone, Copy, Debug)]
pub struct Text {
    pub ja: &'static str,
    pub en: &'static str,
}

impl Text {
    pub fn get(&self) -> &'static str {
        match i18n::current() {
            Language::Japanese => self.ja,
            Language::English => self.en,
        }
    }
}

const fn text(ja: &'static str, en: &'static str) -> Text {
    Text { ja, en }
}

#[derive(Clone, Debug)]
pub struct DataSource {
    pub id: &'static str,
    pub name: Text,
    /// 何のために使うか（設定画面に表示）
    pub purpose: Text,
    /// 接続先ホスト。空なら HTTP ではなくローカルプロセス。
    pub hosts: &'static [&'static str],
    /// 送信される情報（設定画面に表示）
    pub sends: Text,
    pub docs: &'static str,
}

impl DataSource {
    pub fn localized_name(&self) -> &'static str {
        self.name.get()
    }

    /// 画面に渡す、現在の表示言語の内容。
    pub fn view(&self) -> DataSourceView {
        DataSourceView {
            id: self.id,
            name: self.name.get(),
            purpose: self.purpose.get(),
            hosts: self.hosts,
            sends: self.sends.get(),
            docs: self.docs,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DataSourceView {
    pub id: &'static str,
    pub name: &'static str,
    pub purpose: &'static str,
    pub hosts: &'static [&'static str],
    pub sends: &'static str,
    pub docs: &'static str,
}

pub const DEPS_DEV: &str = "deps.dev";
pub const OSV: &str = "osv";
pub const ENDOFLIFE: &str = "endoflife.date";
pub const MAVEN_CENTRAL: &str = "maven-central";
pub const MAVEN_LOCAL: &str = "maven-local";

pub const SOURCES: &[DataSource] = &[
    DataSource {
        id: DEPS_DEV,
        name: text("deps.dev", "deps.dev"),
        purpose: text(
            "最新バージョン・系列ごとの最新版・非推奨・公開日・ライセンス",
            "Latest versions, latest per series, deprecation, release dates, licenses",
        ),
        hosts: &["api.deps.dev"],
        sends: text("パッケージ名とバージョン（URL に含まれる）", "Package names and versions (in the URL)"),
        docs: "https://docs.deps.dev/api/v3/",
    },
    DataSource {
        id: OSV,
        name: text("OSV.dev", "OSV.dev"),
        purpose: text("既知の脆弱性（GHSA / CVE）", "Known vulnerabilities (GHSA / CVE)"),
        hosts: &["api.osv.dev"],
        sends: text(
            "パッケージの purl とバージョン（POST 本文）、脆弱性 ID",
            "Package purls and versions (POST body), vulnerability IDs",
        ),
        docs: "https://google.github.io/osv.dev/api/",
    },
    DataSource {
        id: ENDOFLIFE,
        name: text("endoflife.date", "endoflife.date"),
        purpose: text(
            "製品のサポート終了日・系列ごとの要件（対応 Java など）",
            "End-of-life dates and per-cycle requirements (supported Java, etc.)",
        ),
        hosts: &["endoflife.date"],
        sends: text(
            "なし（製品一覧をまとめて取得し、照合は手元で行う）",
            "Nothing (the product list is downloaded and matched locally)",
        ),
        docs: "https://endoflife.date/docs/api/v1/",
    },
    DataSource {
        id: MAVEN_CENTRAL,
        name: text("Maven Central", "Maven Central"),
        purpose: text(
            "jar のクラスファイルから必要な Java バージョンを判定（詳細画面を開いたときのみ）",
            "Detects the required Java version from jar class files (only when a detail view is opened)",
        ),
        hosts: &["repo1.maven.org"],
        sends: text(
            "groupId・artifactId・バージョン（URL に含まれる）。jar は末尾と数クラス分だけ部分取得",
            "groupId, artifactId and version (in the URL). Only the end of the jar and a few classes are downloaded",
        ),
        docs: "https://central.sonatype.org/",
    },
    DataSource {
        id: MAVEN_LOCAL,
        name: text("ローカルの Maven", "Local Maven"),
        purpose: text(
            "「Maven で SBOM 生成」で mvn を実行（依存解決のため、Maven の設定にあるリポジトリへ通信する）",
            "Runs mvn for \"Generate SBOM with Maven\" (Maven contacts the repositories in its settings to resolve dependencies)",
        ),
        hosts: &[],
        sends: text(
            "Maven が依存解決で行う通信（このツールの外で行われるため、通信ログには実行の開始・終了のみ記録）",
            "Whatever Maven sends to resolve dependencies (outside this tool; only the start and end are logged)",
        ),
        docs: "https://github.com/CycloneDX/cyclonedx-maven-plugin",
    },
];

/// ホスト名からデータソースを引く。登録されていないホストは `None`。
pub fn by_host(host: &str) -> Option<&'static DataSource> {
    #[cfg(test)]
    if host == TEST_SOURCE.hosts[0] {
        return Some(&TEST_SOURCE);
    }
    SOURCES.iter().find(|s| s.hosts.iter().any(|h| h.eq_ignore_ascii_case(host)))
}

/// テストでだけ使う接続先（手元で立てた HTTP サーバー）。
#[cfg(test)]
pub const TEST_SOURCE: DataSource = DataSource {
    id: "test-local",
    name: text("テスト用", "Test"),
    purpose: text("テスト", "Test"),
    hosts: &["127.0.0.1"],
    sends: text("なし", "Nothing"),
    docs: "",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_by_host() {
        assert_eq!(by_host("api.osv.dev").map(|s| s.id), Some(OSV));
        assert_eq!(by_host("API.DEPS.DEV").map(|s| s.id), Some(DEPS_DEV));
        assert!(by_host("example.com").is_none());
    }
}

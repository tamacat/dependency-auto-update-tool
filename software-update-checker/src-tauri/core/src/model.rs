//! 取り込み・チェックで共通に使うデータモデル。
//!
//! フロントエンド（TypeScript）とは JSON でやり取りするため、すべて
//! `camelCase` でシリアライズする。対応する型定義は `src/lib/types.ts`。

use serde::{Deserialize, Serialize};

/// パッケージのエコシステム。purl の type に対応する。
///
/// 新しいエコシステムに対応するときは、ここに値を足し、
/// [`Ecosystem::from_purl_type`] と [`Ecosystem::deps_dev_system`] を更新する。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Ecosystem {
    Maven,
    Npm,
    Pypi,
    Cargo,
    Golang,
    Nuget,
    Gem,
    Other,
}

impl Ecosystem {
    pub fn from_purl_type(purl_type: &str) -> Self {
        match purl_type.to_ascii_lowercase().as_str() {
            "maven" => Self::Maven,
            "npm" => Self::Npm,
            "pypi" => Self::Pypi,
            "cargo" => Self::Cargo,
            "golang" => Self::Golang,
            "nuget" => Self::Nuget,
            "gem" => Self::Gem,
            _ => Self::Other,
        }
    }

    pub fn purl_type(&self) -> Option<&'static str> {
        match self {
            Self::Maven => Some("maven"),
            Self::Npm => Some("npm"),
            Self::Pypi => Some("pypi"),
            Self::Cargo => Some("cargo"),
            Self::Golang => Some("golang"),
            Self::Nuget => Some("nuget"),
            Self::Gem => Some("gem"),
            Self::Other => None,
        }
    }

    /// deps.dev API の system 名。未対応なら `None`。
    pub fn deps_dev_system(&self) -> Option<&'static str> {
        match self {
            Self::Maven => Some("MAVEN"),
            Self::Npm => Some("NPM"),
            Self::Pypi => Some("PYPI"),
            Self::Cargo => Some("CARGO"),
            Self::Golang => Some("GO"),
            Self::Nuget => Some("NUGET"),
            Self::Gem => Some("RUBYGEMS"),
            Self::Other => None,
        }
    }

    /// deps.dev / OSV で使うパッケージ名（Maven は `groupId:artifactId`）。
    pub fn package_name(&self, group: Option<&str>, name: &str) -> String {
        match (self, group) {
            (Self::Maven, Some(g)) => format!("{g}:{name}"),
            (Self::Npm | Self::Golang, Some(g)) => format!("{g}/{name}"),
            _ => name.to_string(),
        }
    }
}

/// 取り込んだ 1 つのソフトウェア部品。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    /// 取り込み結果の中で一意な ID（bom-ref / SPDXID / 生成値）。
    pub id: String,
    pub ecosystem: Ecosystem,
    /// バージョン付きの purl（取得できた場合）。
    pub purl: Option<String>,
    /// Maven の groupId、npm の scope など。
    pub group: Option<String>,
    pub name: String,
    pub version: Option<String>,
    /// compile / runtime / test / import / parent / required など、入力形式の値そのまま。
    pub scope: Option<String>,
    /// 直接依存か。入力から判定できなければ `None`。
    pub direct: Option<bool>,
    pub licenses: Vec<String>,
    /// 依存先コンポーネントの `id`。
    pub dependencies: Vec<String>,
    /// 取り込み時の注意（バージョン未解決など）。
    pub notes: Vec<String>,
}

impl Component {
    pub fn new(id: impl Into<String>, ecosystem: Ecosystem, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            ecosystem,
            purl: None,
            group: None,
            name: name.into(),
            version: None,
            scope: None,
            direct: None,
            licenses: Vec::new(),
            dependencies: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// deps.dev / OSV で照会するときのパッケージ名。
    pub fn package_name(&self) -> String {
        self.ecosystem.package_name(self.group.as_deref(), &self.name)
    }
}

/// 1 ファイルの取り込み結果。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub source_path: String,
    /// `pom`, `cyclonedx-json`, `cyclonedx-xml`, `spdx-json` など。
    pub format: String,
    pub project_name: Option<String>,
    pub components: Vec<Component>,
    pub warnings: Vec<String>,
    /// 推移的依存まで含んでいるか（pom.xml の直接読込では false）。
    pub includes_transitive: bool,
    /// 同じ場所にある、より詳しい入力（pom の隣の CycloneDX SBOM など）。画面から読み込める。
    #[serde(default)]
    pub related_files: Vec<String>,
}

/// チェック実行時のオプション（フロントエンドの設定画面から渡る）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckOptions {
    /// 無効にしたデータソースの ID（[`crate::sources`]）。ここにあるものへは通信しない。
    pub disabled_sources: Vec<String>,
    /// 最新版の判定にプレリリース版を含めるか。
    pub include_prerelease: bool,
    /// 最終リリースからこの年数を超えたら「長期間更新なし」とみなす。
    pub stale_years: u32,
    /// false ならキャッシュを使わずに取り直す。
    pub use_cache: bool,
    /// 再試行・待ち時間・流量制御の設定
    pub network: crate::netguard::NetworkSettings,
    /// 読み込み専用で参照する、共有の Java 要件ナレッジファイル
    pub shared_knowledge_files: Vec<String>,
    /// 全部品について deps.dev のバージョン詳細とプロジェクト情報を取り、
    /// OpenSSF Scorecard と来歴（SLSA provenance・attestation）を確認する。
    /// false なら、ライセンスが入力に無い部品のバージョン詳細だけを取る。
    pub supply_chain: bool,
    /// 利用者が使っている実行環境の版。合わない更新先に警告を出す。
    pub runtime_targets: RuntimeTargets,
}

/// 利用者が使っている実行環境の版（空なら未設定）。`21`、`20.11`、`3.12` のように書く。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeTargets {
    pub java: String,
    pub node: String,
    pub python: String,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            disabled_sources: Vec::new(),
            include_prerelease: false,
            stale_years: 3,
            use_cache: true,
            network: crate::netguard::NetworkSettings::default(),
            shared_knowledge_files: Vec::new(),
            supply_chain: true,
            runtime_targets: RuntimeTargets::default(),
        }
    }
}

impl CheckOptions {
    pub fn source_enabled(&self, id: &str) -> bool {
        !self.disabled_sources.iter().any(|d| d == id)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum UpdateKind {
    None,
    Patch,
    Minor,
    Major,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    /// 全体での最新版。
    pub latest: Option<String>,
    pub latest_published_at: Option<String>,
    /// 現行版と同じメジャーバージョン内での最新版。
    pub latest_in_major: Option<String>,
    /// 現行版と同じ系列（メジャー.マイナー）内での最新版。
    /// 系列が変わると必要な JDK や API が変わることがあるため、最も安全な更新先になる。
    pub latest_in_minor: Option<String>,
    /// 現行版の系列以降の、系列ごとの最新版（新しい系列から順）。
    pub series: Vec<SeriesInfo>,
    pub update_kind: UpdateKind,
    pub current_published_at: Option<String>,
    /// いずれかのバージョンの最終公開日（メンテ状況の推定に使う）。
    pub last_release_at: Option<String>,
    /// 最終リリースが `stale_years` より古い。
    pub stale: bool,
    pub deprecated: Option<String>,
    /// 現行版がレジストリに存在しない（社内ライブラリ等）。
    pub current_not_found: bool,
}

/// バージョン系列（メジャー.マイナー）ごとの最新版。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SeriesInfo {
    /// `1.3` のような系列名
    pub series: String,
    pub latest: String,
    pub published_at: Option<String>,
    /// 現行版が属する系列か
    pub is_current: bool,
}

/// そのバージョンの実行に必要な Java（ナレッジファイルまたは jar のクラスファイルから）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JavaRequirement {
    pub version: String,
    /// 必要な Java のバージョン（`8`、`11` など）。分からなければ `None`。
    pub java: Option<String>,
    /// クラスファイルのメジャーバージョン（52 = Java 8）
    pub class_major: Option<u16>,
    /// 記録のメモ、または判定できなかった理由
    pub note: Option<String>,
    /// どこから得たか: `detected`（今回 jar から判定）/ `local`（手元のナレッジ）/ `shared`（共有ナレッジ）/ `none`
    pub origin: String,
    /// ナレッジの記録の種類（`jar` / `manual`）
    pub rule_source: Option<String>,
    /// ナレッジの記録の適用範囲（例: 「logback-core 1.4 系」）
    pub rule_scope: Option<String>,
    /// 共有ナレッジのファイル
    pub origin_file: Option<String>,
}

/// そのバージョンの実行に必要な実行環境（npm の `engines.node`、PyPI の `requires_python`）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRequirement {
    pub version: String,
    /// `node` / `python`
    pub runtime: String,
    /// 宣言された範囲そのまま（`^20.19 || >=22`、`>=3.8` など）。宣言が無ければ `None`
    pub constraint: Option<String>,
    /// 範囲の下限（`20.19`、`3.8` など）。下限が無い・読めなければ `None`
    pub minimum: Option<String>,
    /// 設定した実行環境の版が範囲に入るか。未設定や範囲を読めないときは `None`
    pub target_ok: Option<bool>,
    /// 取得できなかった理由
    pub error: Option<String>,
}

/// 推移的依存の脆弱性を、どの直接依存を上げれば解消できるか。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransitiveFix {
    /// 直接依存 → … → 対象 の経路（部品 ID の列）
    pub paths: Vec<Vec<String>>,
    /// 経路の起点の直接依存ごとの結果
    pub routes: Vec<RouteFix>,
    /// 直接依存を上げずに対象の版を固定する方法の案内
    pub pin_hint: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteFix {
    /// 直接依存の部品 ID
    pub direct_id: String,
    pub current: Option<String>,
    /// この版以上に上げれば解消する（deps.dev の解決結果からの推定）。見つからなければ `None`
    pub fix_version: Option<String>,
    /// その版が引き込む対象の版。引き込まなくなる場合は `None`
    pub target_version: Option<String>,
    /// 調べた版の数
    pub probes: usize,
    pub error: Option<String>,
}

/// 設定した実行環境のサポート状況（endoflife.date）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// `java` / `node` / `python`
    pub runtime: String,
    pub version: String,
    /// endoflife.date の製品が対応しない（Java は配布元ごとに違うため調べない）か、取得できなければ `None`
    pub eol: Option<EolInfo>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct VulnInfo {
    pub id: String,
    pub aliases: Vec<String>,
    pub summary: Option<String>,
    pub severity: Severity,
    pub score: Option<f64>,
    pub fixed_versions: Vec<String>,
    pub published: Option<String>,
    pub url: String,
    /// 悪意あるパッケージとして報告されたもの（OSV の `MAL-` ID。OpenSSF Malicious Packages 由来）。
    #[serde(default)]
    pub malicious: bool,
    /// CISA KEV（実際に悪用が確認された脆弱性の一覧）に載っているか。
    #[serde(default)]
    pub kev: Option<KevInfo>,
    /// EPSS（30 日以内に悪用される確率の推定）。
    #[serde(default)]
    pub epss: Option<EpssInfo>,
}

/// CISA KEV の記録。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KevInfo {
    pub cve: String,
    pub date_added: Option<String>,
    /// 米国政府機関向けの対応期限
    pub due_date: Option<String>,
    /// ランサムウェアでの悪用が確認されているか
    pub ransomware: bool,
    pub required_action: Option<String>,
}

/// EPSS のスコア。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EpssInfo {
    pub cve: String,
    /// 0.0〜1.0
    pub score: f64,
    /// 全 CVE の中での順位（0.0〜1.0。0.99 なら上位 1%）
    pub percentile: f64,
    pub date: Option<String>,
}

/// 脆弱性を解消できる更新先。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FixAdvice {
    /// 現行と同じ系列（メジャー.マイナー）で、直せるものをすべて直せる最小の版
    pub in_series: Option<String>,
    /// 現行と同じメジャーバージョンで、直せるものをすべて直せる最小の版
    pub in_major: Option<String>,
    /// 系列を問わず、直せるものをすべて直せる最小の版
    pub minimum: Option<String>,
    /// どの版に上げても直らない（修正版が無い・判定できない）脆弱性の ID
    pub unfixed: Vec<String>,
}

/// サプライチェーンの情報（deps.dev のバージョン詳細とプロジェクト情報から）。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SupplyChainInfo {
    /// ソースリポジトリ（`github.com/owner/repo` 形式）
    pub source_repo: Option<String>,
    /// ソースリポジトリとの対応が来歴で裏付けられているか（自己申告のメタデータだけなら false）
    pub source_repo_verified: bool,
    pub scorecard: Option<ScorecardInfo>,
    /// 来歴（SLSA provenance・attestation）。無ければ空。
    pub provenance: Vec<ProvenanceInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardInfo {
    /// 0〜10
    pub overall: f64,
    pub date: Option<String>,
    pub checks: Vec<ScorecardCheck>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScorecardCheck {
    pub name: String,
    /// 0〜10。判定対象外なら `None`
    pub score: Option<i32>,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProvenanceInfo {
    /// 証明の種類（`https://slsa.dev/provenance/v1` など）
    pub kind: Option<String>,
    pub verified: bool,
    pub source_repository: Option<String>,
    pub commit: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum EolStatus {
    Supported,
    Eol,
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum EolMatch {
    /// endoflife.date の purl 識別子と完全一致。
    Exact,
    /// 同じ groupId や同梱の対応表から推定。
    Inferred,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EolInfo {
    pub product: String,
    pub product_label: String,
    pub cycle: Option<String>,
    pub status: EolStatus,
    pub eol_from: Option<String>,
    pub latest_in_cycle: Option<String>,
    pub match_kind: EolMatch,
    pub link: String,
    /// 現行サイクル以降のサイクル（新しい順）。サイクルごとの要件の比較に使う。
    pub cycles: Vec<EolCycle>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EolCycle {
    pub name: String,
    pub status: EolStatus,
    pub eol_from: Option<String>,
    pub latest: Option<String>,
    pub is_current: bool,
    /// 製品固有の項目（Tomcat の minJavaVersion、Spring Boot の supportedJavaVersions など）
    pub requirements: Vec<KeyValue>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
}

/// 1 コンポーネント分のチェック結果。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub component_id: String,
    pub version: Option<VersionInfo>,
    pub vulnerabilities: Vec<VulnInfo>,
    pub eol: Option<EolInfo>,
    /// 取り込み時にライセンスがなかった場合、deps.dev から補完した値。
    pub licenses: Vec<String>,
    /// 脆弱性がある場合の、解消できる更新先
    pub fix: Option<FixAdvice>,
    pub supply_chain: Option<SupplyChainInfo>,
    pub errors: Vec<String>,
}

/// チェック進捗（`check-progress` イベントでフロントへ送る）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CheckProgress {
    pub phase: String,
    pub done: usize,
    pub total: usize,
}

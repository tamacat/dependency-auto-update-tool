// Rust 側 src-tauri/src/model.rs と対応する型。変更するときは両方そろえること。

export type Ecosystem = 'maven' | 'npm' | 'pypi' | 'cargo' | 'golang' | 'nuget' | 'gem' | 'other';

export interface Component {
  id: string;
  ecosystem: Ecosystem;
  purl: string | null;
  group: string | null;
  name: string;
  version: string | null;
  scope: string | null;
  direct: boolean | null;
  licenses: string[];
  dependencies: string[];
  notes: string[];
}

export interface ImportResult {
  sourcePath: string;
  format: string;
  projectName: string | null;
  components: Component[];
  warnings: string[];
  includesTransitive: boolean;
  /** 同じ場所にある、より詳しい入力（pom の隣の SBOM など） */
  relatedFiles?: string[];
}

export interface CheckOptions {
  /** 無効にしたデータソースの ID。ここにあるものへは通信しない。 */
  disabledSources: string[];
  includePrerelease: boolean;
  staleYears: number;
  useCache: boolean;
  network: NetworkSettings;
  /** 読み込み専用で参照する共有の Java 要件ナレッジファイル */
  sharedKnowledgeFiles: string[];
  /** OpenSSF Scorecard と来歴（SLSA provenance）を deps.dev から取得する */
  supplyChain: boolean;
  /** 利用者が使っている実行環境の版（空なら未設定） */
  runtimeTargets: RuntimeTargets;
}

export interface RuntimeTargets {
  java: string;
  node: string;
  python: string;
}

/** npm の engines.node・PyPI の requires_python（src-tauri/src/model.rs の RuntimeRequirement と対応） */
export interface RuntimeRequirement {
  version: string;
  runtime: 'node' | 'python';
  /** 宣言された範囲そのまま。宣言が無ければ null */
  constraint: string | null;
  /** 範囲の下限 */
  minimum: string | null;
  /** 設定した版が範囲に入るか。未設定や範囲を読めなければ null */
  targetOk: boolean | null;
  error: string | null;
}

/** 推移的依存の直し方（src-tauri/src/model.rs の TransitiveFix と対応） */
export interface TransitiveFix {
  /** 直接依存 → … → 対象 の経路（部品 ID） */
  paths: string[][];
  routes: RouteFix[];
  /** 直接依存を上げずに固定する方法の案内 */
  pinHint: string | null;
}

export interface RouteFix {
  directId: string;
  current: string | null;
  /** この版以上に上げれば解消する（推定） */
  fixVersion: string | null;
  /** その版が引き込む対象の版。引き込まなくなるなら null */
  targetVersion: string | null;
  probes: number;
  error: string | null;
}

export interface RuntimeStatus {
  runtime: 'java' | 'node' | 'python';
  version: string;
  eol: EolInfo | null;
  error: string | null;
}

/** src-tauri/src/netguard.rs の NetworkSettings と対応。範囲外の値はバックエンドで丸められる。 */
export interface NetworkSettings {
  maxRetries: number;
  retryBaseMs: number;
  maxRetryWaitMs: number;
  maxConcurrency: number;
  minIntervalMs: number;
  maxRequestsPerMinute: number;
  maxSameRequestPerMinute: number;
  failureThreshold: number;
  cooldownSecs: number;
  timeoutSecs: number;
}

/** 設定画面の入力範囲（バックエンドの clamped と同じ） */
export const NETWORK_LIMITS: Record<keyof NetworkSettings, [number, number]> = {
  maxRetries: [0, 5],
  retryBaseMs: [200, 30000],
  maxRetryWaitMs: [1000, 120000],
  maxConcurrency: [1, 16],
  minIntervalMs: [0, 5000],
  maxRequestsPerMinute: [10, 1200],
  maxSameRequestPerMinute: [1, 20],
  failureThreshold: [1, 50],
  cooldownSecs: [10, 3600],
  timeoutSecs: [5, 300],
};

export const DEFAULT_NETWORK: NetworkSettings = {
  maxRetries: 2,
  retryBaseMs: 1000,
  maxRetryWaitMs: 30000,
  maxConcurrency: 6,
  minIntervalMs: 100,
  maxRequestsPerMinute: 300,
  maxSameRequestPerMinute: 3,
  failureThreshold: 5,
  cooldownSecs: 120,
  timeoutSecs: 60,
};

export type UpdateKind = 'none' | 'patch' | 'minor' | 'major' | 'unknown';

export interface VersionInfo {
  latest: string | null;
  latestPublishedAt: string | null;
  latestInMajor: string | null;
  /** 同じ系列（メジャー.マイナー）内の最新 */
  latestInMinor: string | null;
  /** 現行の系列以降の、系列ごとの最新版（新しい順） */
  series: SeriesInfo[];
  updateKind: UpdateKind;
  currentPublishedAt: string | null;
  lastReleaseAt: string | null;
  stale: boolean;
  deprecated: string | null;
  currentNotFound: boolean;
}

export interface SeriesInfo {
  series: string;
  latest: string;
  publishedAt: string | null;
  isCurrent: boolean;
}

export interface JavaRequirement {
  version: string;
  java: string | null;
  classMajor: number | null;
  note: string | null;
  /** detected（今回 jar から判定）/ local（手元のナレッジ）/ shared（共有ナレッジ）/ none */
  origin: 'detected' | 'local' | 'shared' | 'none';
  ruleSource: 'jar' | 'manual' | null;
  ruleScope: string | null;
  originFile: string | null;
}

/** Java 要件ナレッジの 1 件（src-tauri/src/knowledge.rs の JavaRule と対応） */
export interface JavaRule {
  ecosystem: string;
  group: string;
  /** '*' なら groupId 配下すべて */
  artifact: string;
  version?: string | null;
  series?: string | null;
  java: string | null;
  classMajor?: number | null;
  source: 'jar' | 'manual';
  checkedAt?: string | null;
  note?: string | null;
}

export interface RuleKey {
  ecosystem: string;
  group: string;
  artifact: string;
  version: string | null;
  series: string | null;
}

export const ruleKey = (r: JavaRule): RuleKey => ({
  ecosystem: r.ecosystem,
  group: r.group,
  artifact: r.artifact,
  version: r.version ?? null,
  series: r.series ?? null,
});

export interface SharedKnowledgeFile {
  path: string;
  entries: JavaRule[];
  error: string | null;
}

export interface KnowledgeView {
  localPath: string | null;
  localError: string | null;
  local: JavaRule[];
  shared: SharedKnowledgeFile[];
}

export interface MergeSummary {
  added: number;
  updated: number;
  skipped: number;
}

export type Severity = 'unknown' | 'low' | 'medium' | 'high' | 'critical';

export interface VulnInfo {
  id: string;
  aliases: string[];
  summary: string | null;
  severity: Severity;
  score: number | null;
  fixedVersions: string[];
  published: string | null;
  url: string;
  /** 悪意あるパッケージの報告（OSV の MAL- ID） */
  malicious?: boolean;
  /** CISA KEV（悪用が確認された脆弱性）に載っている */
  kev?: KevInfo | null;
  epss?: EpssInfo | null;
}

export interface KevInfo {
  cve: string;
  dateAdded: string | null;
  dueDate: string | null;
  ransomware: boolean;
  requiredAction: string | null;
}

export interface EpssInfo {
  cve: string;
  /** 0〜1 */
  score: number;
  /** 0〜1（0.99 なら上位 1%） */
  percentile: number;
  date: string | null;
}

/** 脆弱性をすべて直せる更新先 */
export interface FixAdvice {
  inSeries: string | null;
  inMajor: string | null;
  minimum: string | null;
  /** どの版に上げても直らない脆弱性の ID */
  unfixed: string[];
}

export interface SupplyChainInfo {
  /** github.com/owner/repo 形式 */
  sourceRepo: string | null;
  /** ソースリポジトリとの対応が来歴で裏付けられているか */
  sourceRepoVerified: boolean;
  scorecard: ScorecardInfo | null;
  provenance: ProvenanceInfo[];
}

export interface ScorecardInfo {
  overall: number;
  date: string | null;
  checks: { name: string; score: number | null; reason: string | null }[];
}

export interface ProvenanceInfo {
  kind: string | null;
  verified: boolean;
  sourceRepository: string | null;
  commit: string | null;
}

export type EolStatus = 'supported' | 'eol' | 'unknown';

export interface EolInfo {
  product: string;
  productLabel: string;
  cycle: string | null;
  status: EolStatus;
  eolFrom: string | null;
  latestInCycle: string | null;
  matchKind: 'exact' | 'inferred';
  link: string;
  cycles: EolCycle[];
}

export interface EolCycle {
  name: string;
  status: EolStatus;
  eolFrom: string | null;
  latest: string | null;
  isCurrent: boolean;
  requirements: { key: string; value: string }[];
}

export interface CheckResult {
  componentId: string;
  version: VersionInfo | null;
  vulnerabilities: VulnInfo[];
  eol: EolInfo | null;
  licenses: string[];
  fix?: FixAdvice | null;
  supplyChain?: SupplyChainInfo | null;
  errors: string[];
}

export interface CheckProgress {
  phase: string;
  done: number;
  total: number;
}

export interface DataSource {
  id: string;
  name: string;
  purpose: string;
  hosts: string[];
  sends: string;
  docs: string;
}

export type LogOutcome = 'network' | 'cache' | 'blocked' | 'error' | 'info';

export interface LogEntry {
  ts: string;
  kind: 'http' | 'process' | 'app';
  source: string | null;
  method: string | null;
  url: string | null;
  status: number | null;
  durationMs: number | null;
  bytes: number | null;
  outcome: LogOutcome;
  message: string | null;
}

/** 表示用に Component と CheckResult を結合したもの。 */
export interface Row {
  component: Component;
  result: CheckResult | null;
  /** 脆弱性の最大深刻度（なければ null） */
  maxSeverity: Severity | null;
  licenses: string[];
}

export const DEFAULT_OPTIONS: CheckOptions = {
  disabledSources: [],
  includePrerelease: false,
  staleYears: 3,
  useCache: true,
  network: { ...DEFAULT_NETWORK },
  sharedKnowledgeFiles: [],
  supplyChain: true,
  runtimeTargets: { java: '', node: '', python: '' },
};

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
};

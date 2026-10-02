// 開発用: Tauri の外（通常のブラウザで `npm run dev`）で画面を確認するための模擬バックエンド。
// main.ts から開発ビルドかつ Tauri 外のときだけ読み込まれ、本番ビルドには含まれない。
// 模擬データは実際のチェック結果を保存したもの（作り方は README の「画面だけを確認する」）。
// 通信ログは、実際に行われるはずの通信を模して生成する（実際には通信しない）。

import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import sample from './fixtures/sample.json';
import { ruleKey } from './types';
import type { CheckOptions, CheckResult, Component, DataSource, ImportResult, JavaRequirement, JavaRule, LogEntry, RuleKey } from './types';

const fixture = sample as unknown as {
  import: ImportResult;
  results: CheckResult[];
  java: Record<string, JavaRequirement[]>;
};
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// src-tauri/src/sources.rs と同じ内容
const SOURCES: DataSource[] = [
  { id: 'deps.dev', name: 'deps.dev', purpose: '最新バージョン・系列ごとの最新版・非推奨・公開日・ライセンス', hosts: ['api.deps.dev'], sends: 'パッケージ名とバージョン（URL に含まれる）', docs: 'https://docs.deps.dev/api/v3/' },
  { id: 'osv', name: 'OSV.dev', purpose: '既知の脆弱性（GHSA / CVE）', hosts: ['api.osv.dev'], sends: 'パッケージの purl とバージョン（POST 本文）、脆弱性 ID', docs: 'https://google.github.io/osv.dev/api/' },
  { id: 'endoflife.date', name: 'endoflife.date', purpose: '製品のサポート終了日・系列ごとの要件（対応 Java など）', hosts: ['endoflife.date'], sends: 'なし（製品一覧をまとめて取得し、照合は手元で行う）', docs: 'https://endoflife.date/docs/api/v1/' },
  { id: 'maven-central', name: 'Maven Central', purpose: 'jar のクラスファイルから必要な Java バージョンを判定（詳細画面を開いたときのみ）', hosts: ['repo1.maven.org'], sends: 'groupId・artifactId・バージョン（URL に含まれる）。jar は末尾と数クラス分だけ部分取得', docs: 'https://central.sonatype.org/' },
  { id: 'maven-local', name: 'ローカルの Maven', purpose: '「Maven で SBOM 生成」で mvn を実行（依存解決のため、Maven の設定にあるリポジトリへ通信する）', hosts: [], sends: 'Maven が依存解決で行う通信（このツールの外で行われるため、通信ログには実行の開始・終了のみ記録）', docs: 'https://github.com/CycloneDX/cyclonedx-maven-plugin' },
];

// 英語表示のときのデータソースの説明（src-tauri/src/sources.rs の en と同じ）
const SOURCES_EN: Record<string, Pick<DataSource, 'name' | 'purpose' | 'sends'>> = {
  'deps.dev': { name: 'deps.dev', purpose: 'Latest versions, latest per series, deprecation, release dates, licenses', sends: 'Package names and versions (in the URL)' },
  osv: { name: 'OSV.dev', purpose: 'Known vulnerabilities (GHSA / CVE)', sends: 'Package purls and versions (POST body), vulnerability IDs' },
  'endoflife.date': { name: 'endoflife.date', purpose: 'End-of-life dates and per-cycle requirements (supported Java, etc.)', sends: 'Nothing (the product list is downloaded and matched locally)' },
  'maven-central': { name: 'Maven Central', purpose: 'Detects the required Java version from jar class files (only when a detail view is opened)', sends: 'groupId, artifactId and version (in the URL). Only the end of the jar and a few classes are downloaded' },
  'maven-local': { name: 'Local Maven', purpose: 'Runs mvn for "Generate SBOM with Maven" (Maven contacts the repositories in its settings to resolve dependencies)', sends: 'Whatever Maven sends to resolve dependencies (outside this tool; only the start and end are logged)' },
};
let language = 'ja';

const log: LogEntry[] = [];

// Java 要件ナレッジ（手元のファイルの代わり）。手動記録の例を 1 件入れておく
const knowledge: JavaRule[] = [
  {
    ecosystem: 'maven',
    group: 'ch.qos.logback',
    artifact: '*',
    series: '1.4',
    java: '11',
    source: 'manual',
    checkedAt: '2026-10-03',
    note: '1.4 系から Jakarta EE 9+（jakarta.servlet）対応になり Java 11 必須。Java 8 環境は 1.3 系を使う',
  },
];
const sameKey = (a: RuleKey, b: RuleKey) => JSON.stringify(a) === JSON.stringify(b);

/** knowledge.rs の best_match と同じ優先順位（具体性 → 手動 → 手元） */
function lookup(group: string, artifact: string, version: string): JavaRule | undefined {
  const score = (r: JavaRule) => {
    if (r.group !== group || (r.artifact !== artifact && r.artifact !== '*')) return -1;
    let s = r.artifact === artifact ? 1000 : 0;
    if (r.version) {
      if (r.version !== version) return -1;
      s += 500;
    } else if (r.series) {
      if (version !== r.series && !version.startsWith(`${r.series}.`) && !version.startsWith(`${r.series}-`)) return -1;
      s += 100 + r.series.length;
    }
    return s * 2 + (r.source === 'manual' ? 1 : 0);
  };
  return knowledge.map((r) => [score(r), r] as const).filter(([s]) => s >= 0).sort((a, b) => b[0] - a[0])[0]?.[1];
}

async function record(partial: Partial<LogEntry>) {
  const entry: LogEntry = {
    ts: new Date().toISOString(),
    kind: 'http',
    source: null,
    method: null,
    url: null,
    status: null,
    durationMs: null,
    bytes: null,
    outcome: 'info',
    message: null,
    ...partial,
  };
  log.push(entry);
  await emit('activity-log', entry);
}

async function http(options: CheckOptions, source: string, method: string, url: string, message: string | null = null) {
  if (options.disabledSources.includes(source)) {
    const name = SOURCES.find((s) => s.id === source)?.name;
    await record({ source, method, url, outcome: 'blocked', message: `データソース「${name}」は設定で無効になっています` });
    return;
  }
  await record({
    source, method, url, message,
    outcome: 'network',
    status: 200,
    durationMs: 80 + Math.round(Math.random() * 300),
    bytes: 2000 + Math.round(Math.random() * 30000),
  });
}

async function simulateChecks(components: Component[], options: CheckOptions) {
  const enabled = SOURCES.filter((s) => !options.disabledSources.includes(s.id)).map((s) => s.id);
  await record({ kind: 'app', message: `チェック開始: ${components.length} 件。有効なデータソース: ${enabled.join(', ')}` });
  const sample = components.filter((c) => c.ecosystem === 'maven').slice(0, 8);
  for (const [i, c] of sample.entries()) {
    await emit('check-progress', { phase: 'versions', done: i + 1, total: sample.length });
    await http(options, 'deps.dev', 'GET', `https://api.deps.dev/v3/systems/MAVEN/packages/${c.group}:${c.name}`);
    await sleep(40);
  }
  await emit('check-progress', { phase: 'vulnerabilities', done: 1, total: 1 });
  await http(options, 'osv', 'POST', 'https://api.osv.dev/v1/querybatch', `${components.length} 件の purl を照会`);
  for (const id of ['GHSA-jhq6-gfmj-v8fx', 'GHSA-p47f-322f-whfh', 'GHSA-qqpg-mvqg-649v']) {
    await http(options, 'osv', 'GET', `https://api.osv.dev/v1/vulns/${id}`);
  }
  await emit('check-progress', { phase: 'eol', done: 1, total: 1 });
  await http(options, 'endoflife.date', 'GET', 'https://endoflife.date/api/v1/products/full');
  await record({ kind: 'app', message: 'チェック完了: 1.2 秒' });
}

/** 無効にしたデータソースの結果を取り除く（実物と同じく、そのチェックは行われない）。 */
function filterResults(options: CheckOptions): CheckResult[] {
  const off = (id: string) => options.disabledSources.includes(id);
  return fixture.results.map((r) => ({
    ...r,
    version: off('deps.dev') ? null : r.version,
    licenses: off('deps.dev') ? [] : r.licenses,
    vulnerabilities: off('osv') ? [] : r.vulnerabilities,
    eol: off('endoflife.date') ? null : r.eol,
  }));
}

export function install() {
  mockWindows('main');
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case 'import_file':
          await sleep(300);
          await record({ kind: 'app', message: `取り込み: ${fixture.import.sourcePath}（${fixture.import.format}、${fixture.import.components.length} 件）` });
          return fixture.import;
        case 'generate_sbom_with_maven':
          await sleep(800);
          await record({ kind: 'process', source: 'maven-local', method: 'mvn', url: `org.cyclonedx:cyclonedx-maven-plugin:2.9.3:makeAggregateBom -f ${a.pomPath}`, outcome: 'network', durationMs: 800, message: 'SBOM を生成' });
          return fixture.import;
        case 'run_checks': {
          const options = a.options as CheckOptions;
          await simulateChecks(a.components as Component[], options);
          return filterResults(options);
        }
        case 'java_requirements': {
          const c = a.component as Component;
          const options = a.options as CheckOptions;
          const versions = a.versions as string[];
          const detected = fixture.java[c.id] ?? [];
          const out: JavaRequirement[] = [];
          for (const v of versions) {
            const rule = lookup(c.group ?? '', c.name, v);
            if (rule) {
              out.push({
                version: v, java: rule.java, classMajor: rule.classMajor ?? null, note: rule.note ?? null,
                origin: 'local', ruleSource: rule.source, originFile: null,
                ruleScope: `${rule.artifact === '*' ? `${rule.group} 配下すべて` : rule.artifact} ${rule.version ?? `${rule.series} 系`}`,
              });
              continue;
            }
            if (options.disabledSources.includes('maven-central')) {
              out.push({ version: v, java: null, classMajor: null, note: 'ナレッジに記録がありません（Maven Central が無効のため判定していません）', origin: 'none', ruleSource: null, ruleScope: null, originFile: null });
              continue;
            }
            const url = `https://repo1.maven.org/maven2/${c.group?.replace(/\./g, '/')}/${c.name}/${v}/${c.name}-${v}.jar`;
            await http(options, 'maven-central', 'GET', url, 'bytes=-65536 ほか（部分取得）');
            const d = detected.find((r) => r.version === v);
            out.push({ version: v, java: d?.java ?? null, classMajor: d?.classMajor ?? null, note: d ? null : '（模擬データなし）', origin: 'detected', ruleSource: 'jar', ruleScope: null, originFile: null });
            if (d) {
              knowledge.push({ ecosystem: 'maven', group: c.group ?? '', artifact: c.name, version: v, java: d.java, classMajor: d.classMajor, source: 'jar', checkedAt: new Date().toISOString().slice(0, 10) });
            }
          }
          await sleep(150);
          return out;
        }
        case 'knowledge_list':
          return {
            localPath: 'C:\\Users\\<user>\\AppData\\Roaming\\org.tamacat.software-update-checker\\java-requirements.json',
            localError: null,
            local: knowledge,
            shared: ((a.sharedFiles as string[]) ?? []).map((path) => ({ path, entries: [], error: null })),
          };
        case 'knowledge_upsert': {
          const rule = a.rule as JavaRule;
          const original = a.original as RuleKey | null;
          for (let i = knowledge.length - 1; i >= 0; i--) {
            const k = ruleKey(knowledge[i]);
            if ((original && sameKey(k, original)) || sameKey(k, ruleKey(rule))) knowledge.splice(i, 1);
          }
          knowledge.push(rule);
          await record({ kind: 'app', message: `Java 要件ナレッジを登録: ${rule.group}:${rule.artifact} → Java ${rule.java}` });
          return null;
        }
        case 'knowledge_delete': {
          const i = knowledge.findIndex((r) => sameKey(ruleKey(r), a.key as RuleKey));
          if (i >= 0) knowledge.splice(i, 1);
          return i >= 0;
        }
        case 'knowledge_import':
          return { added: 0, updated: 0, skipped: 0 };
        case 'knowledge_export':
          return knowledge.length;
        case 'set_language':
          language = String(a.language ?? 'ja');
          return null;
        case 'data_sources':
          return language === 'en' ? SOURCES.map((x) => ({ ...x, ...SOURCES_EN[x.id] })) : SOURCES;
        case 'activity_log':
          return log;
        case 'log_directory':
          return 'C:\\Users\\<user>\\AppData\\Local\\org.tamacat.software-update-checker\\logs';
        case 'plugin:dialog|open':
          return fixture.import.sourcePath;
        case 'plugin:dialog|ask':
          return true;
        case 'plugin:dialog|save':
          return null;
        case 'plugin:opener|open_url':
        case 'plugin:opener|open_path':
        case 'plugin:dialog|message':
        case 'save_text_file':
        case 'clear_cache':
          return null;
        default:
          console.warn('mock: 未対応のコマンド', cmd);
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  console.info('Tauri 外で実行中のため、模擬データで動作しています。');
}

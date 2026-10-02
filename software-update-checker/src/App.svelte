<script lang="ts">
  import { onMount } from 'svelte';
  import { open, save, ask, message } from '@tauri-apps/plugin-dialog';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import * as api from './lib/api';
  import { toCsv, toJson } from './lib/export';
  import { displayName, is, maxSeverity, phaseLabel } from './lib/format';
  import { t } from './lib/i18n';
  import { prefs, savePrefs } from './lib/prefs.svelte';
  import {
    DEFAULT_NETWORK,
    DEFAULT_OPTIONS,
    type CheckOptions,
    type CheckProgress,
    type CheckResult,
    type DataSource,
    type ImportResult,
    type LogEntry,
    type Row,
  } from './lib/types';
  import SummaryCards, { type Filter } from './lib/SummaryCards.svelte';
  import ComponentTable from './lib/ComponentTable.svelte';
  import TreeView from './lib/TreeView.svelte';
  import DetailPane from './lib/DetailPane.svelte';
  import ActivityLogView from './lib/ActivityLogView.svelte';
  import JavaKnowledgeView from './lib/JavaKnowledgeView.svelte';
  import SettingsDialog from './lib/SettingsDialog.svelte';

  const OPTIONS_KEY = 'software-update-checker.options';
  const LOG_CAPACITY = 5000;

  function loadOptions(): CheckOptions {
    try {
      const saved = localStorage.getItem(OPTIONS_KEY);
      if (saved) {
        const parsed = JSON.parse(saved);
        // 旧形式（チェック種別ごとの ON/OFF）からの移行
        const legacy: [string, string][] = [['checkVersions', 'deps.dev'], ['checkVulnerabilities', 'osv'], ['checkEol', 'endoflife.date']];
        const disabled: string[] = Array.isArray(parsed.disabledSources) ? parsed.disabledSources : [];
        for (const [key, id] of legacy) if (parsed[key] === false && !disabled.includes(id)) disabled.push(id);
        return {
          disabledSources: disabled,
          includePrerelease: parsed.includePrerelease ?? DEFAULT_OPTIONS.includePrerelease,
          staleYears: parsed.staleYears ?? DEFAULT_OPTIONS.staleYears,
          useCache: parsed.useCache ?? DEFAULT_OPTIONS.useCache,
          network: { ...DEFAULT_NETWORK, ...(parsed.network ?? {}) },
          sharedKnowledgeFiles: Array.isArray(parsed.sharedKnowledgeFiles) ? parsed.sharedKnowledgeFiles : [],
        };
      }
    } catch {
      /* 保存領域が使えなくても既定値で動かす */
    }
    return { ...DEFAULT_OPTIONS, disabledSources: [], network: { ...DEFAULT_NETWORK }, sharedKnowledgeFiles: [] };
  }

  type View = 'list' | 'tree' | 'java' | 'log';

  let source = $state<ImportResult | null>(null);
  let results = $state<Map<string, CheckResult>>(new Map());
  let options = $state<CheckOptions>(loadOptions());
  let busy = $state<string | null>(null);
  let progress = $state<CheckProgress | null>(null);
  let error = $state<string | null>(null);
  let showSettings = $state(false);
  let selectedId = $state<string | null>(null);
  let view = $state<View>('list');

  let sources = $state<DataSource[]>([]);
  let logEntries = $state<LogEntry[]>([]);
  let logDir = $state<string | null>(null);
  /** チェック開始時点のログ件数（このチェックでの通信件数の表示用） */
  let logMark = $state(0);

  let filter = $state<Filter>('all');
  let query = $state('');
  let directOnly = $state(false);
  let hideTest = $state(false);

  $effect(() => {
    try {
      localStorage.setItem(OPTIONS_KEY, JSON.stringify(options));
    } catch {
      /* 無視 */
    }
  });

  const rows = $derived<Row[]>(
    (source?.components ?? []).map((component) => {
      const result = results.get(component.id) ?? null;
      return {
        component,
        result,
        maxSeverity: maxSeverity(result?.vulnerabilities.map((v) => v.severity) ?? []),
        licenses: component.licenses.length > 0 ? component.licenses : (result?.licenses ?? []),
      };
    }),
  );

  const visibleRows = $derived(
    rows.filter((r) => {
      const c = r.component;
      if (directOnly && c.direct === false) return false;
      if (hideTest && c.scope === 'test') return false;
      if (filter !== 'all' && !is[filter](r)) return false;
      if (query) {
        const q = query.toLowerCase();
        const text = `${displayName(c)} ${c.version ?? ''} ${c.purl ?? ''}`.toLowerCase();
        if (!text.includes(q)) return false;
      }
      return true;
    }),
  );
  const visibleIds = $derived(new Set(visibleRows.map((r) => r.component.id)));
  const filtering = $derived(filter !== 'all' || !!query || hideTest || directOnly);

  const selectedRow = $derived(rows.find((r) => r.component.id === selectedId) ?? null);
  const mavenEnabled = $derived(!options.disabledSources.includes('maven-local'));

  const runTraffic = $derived.by(() => {
    const since = logEntries.slice(logMark);
    return {
      network: since.filter((e) => e.outcome === 'network').length,
      cache: since.filter((e) => e.outcome === 'cache').length,
      blocked: since.filter((e) => e.outcome === 'blocked').length,
      lastHost: (() => {
        const last = [...since].reverse().find((e) => e.outcome === 'network' && e.url);
        try {
          return last?.url ? new URL(last.url).host : null;
        } catch {
          return null;
        }
      })(),
    };
  });

  async function load(task: () => Promise<ImportResult>, label: string) {
    busy = label;
    error = null;
    try {
      source = await task();
      results = new Map();
      selectedId = null;
      filter = 'all';
      if (view === 'log' || view === 'java') view = 'list';
    } catch (e) {
      error = String(e);
    } finally {
      busy = null;
    }
  }

  async function openFile() {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [
        { name: 'pom.xml / SBOM', extensions: ['xml', 'json', 'pom'] },
        { name: t('dialog.allFiles'), extensions: ['*'] },
      ],
    });
    if (typeof path === 'string') await load(() => api.importFile(path), t('busy.loading'));
  }

  async function generateWithMaven() {
    let pom: string | null = source?.format === 'pom' ? source.sourcePath : null;
    if (!pom) {
      const picked = await open({ multiple: false, filters: [{ name: 'pom.xml', extensions: ['xml'] }] });
      if (typeof picked !== 'string') return;
      pom = picked;
    }
    const ok = await ask(
      `${pom}\n\n${t('maven.confirm')}`,
      { title: t('maven.title'), kind: 'info' },
    );
    if (!ok) return;
    const target = pom;
    await load(
      () => api.generateSbomWithMaven(target, $state.snapshot(options)),
      t('busy.maven'),
    );
  }

  async function check() {
    if (!source) return;
    busy = t('busy.checking');
    error = null;
    progress = null;
    logMark = logEntries.length;
    const unlisten = await api.onCheckProgress((p) => (progress = p));
    try {
      const list = await api.runChecks($state.snapshot(source.components), $state.snapshot(options));
      results = new Map(list.map((r) => [r.componentId, r]));
    } catch (e) {
      error = String(e);
    } finally {
      unlisten();
      busy = null;
      progress = null;
    }
  }

  async function exportAs(kind: 'csv' | 'json') {
    if (!source) return;
    const base = (source.projectName ?? 'dependencies').replace(/[^\w.-]+/g, '_');
    const path = await save({
      defaultPath: `${base}.${kind}`,
      filters: [{ name: kind.toUpperCase(), extensions: [kind] }],
    });
    if (!path) return;
    try {
      await api.saveTextFile(path, kind === 'csv' ? toCsv(visibleRows) : toJson(source, visibleRows));
    } catch (e) {
      await message(String(e), { title: t('export.failed'), kind: 'error' });
    }
  }

  // 表示設定（言語・文字サイズ）を画面とバックエンドに反映して保存する
  $effect(() => {
    document.documentElement.lang = prefs.locale;
    document.documentElement.style.setProperty('--font-scale', String(prefs.fontScale));
    savePrefs();
  });
  $effect(() => {
    const locale = prefs.locale;
    // バックエンドのメッセージ言語をそろえてから、データソースの説明を取り直す
    api
      .setLanguage(locale)
      .catch(() => {})
      .then(() => api.dataSources())
      .then((s) => (sources = s))
      .catch(() => {});
  });

  onMount(() => {
    api.logDirectory().then((d) => (logDir = d)).catch(() => {});
    api.activityLog().then((list) => (logEntries = [...list, ...logEntries].slice(-LOG_CAPACITY))).catch(() => {});
    const unlistenLog = api.onActivityLog((entry) => {
      logEntries = logEntries.length >= LOG_CAPACITY ? [...logEntries.slice(1), entry] : [...logEntries, entry];
      if (logEntries.length === LOG_CAPACITY) logMark = Math.max(0, logMark - 1);
    });
    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'drop' && event.payload.paths.length > 0 && !busy) {
        const path = event.payload.paths[0];
        load(() => api.importFile(path), t('busy.loading'));
      }
    });
    return () => {
      unlistenLog.then((f) => f());
      unlistenDrop.then((f) => f());
    };
  });
</script>

<div class="app">
  <header class="toolbar">
    <h1>Software Update Checker</h1>
    <div class="actions">
      <button onclick={openFile} disabled={!!busy}>{t('toolbar.open')}</button>
      <button
        onclick={generateWithMaven}
        disabled={!!busy || !mavenEnabled}
        title={mavenEnabled ? t('toolbar.mavenTitle') : t('toolbar.mavenDisabled')}
      >
        {t('toolbar.maven')}
      </button>
      <button class="primary" onclick={check} disabled={!!busy || !source}>{t('toolbar.check')}</button>
      <button onclick={() => exportAs('csv')} disabled={!!busy || !source}>CSV</button>
      <button onclick={() => exportAs('json')} disabled={!!busy || !source}>JSON</button>
      <button onclick={() => (showSettings = true)} disabled={!!busy} aria-label={t('settings.title')}>{t('settings.title')}</button>
    </div>
  </header>

  {#if busy}
    <div class="status">
      <span>{progress ? phaseLabel(progress.phase) : busy}</span>
      {#if progress && progress.total > 0}
        <progress max={progress.total} value={progress.done}></progress>
        <span class="muted">{progress.done} / {progress.total}</span>
      {:else}
        <progress></progress>
      {/if}
      <span class="muted small traffic">
        {t('status.traffic', { network: runTraffic.network, cache: runTraffic.cache })}{runTraffic.blocked ? t('status.blocked', { n: runTraffic.blocked }) : ''}
        {#if runTraffic.lastHost}({runTraffic.lastHost}){/if}
      </span>
    </div>
  {/if}

  {#if error}
    <div class="banner error">
      <pre>{error}</pre>
      <button class="link" onclick={() => (error = null)}>{t('common.close')}</button>
    </div>
  {/if}

  {#if source}
    <section class="source">
      <div>
        <strong>{source.projectName ?? t('source.unnamed')}</strong>
        <span class="tag">{source.format}</span>
        <span class="tag">{source.includesTransitive ? t('source.transitive') : t('source.directOnly')}</span>
        <span class="muted path">{source.sourcePath}</span>
      </div>
      {#each source.warnings as w}
        <div class="muted small">※ {w}</div>
      {/each}
    </section>

    {#if view === 'list' || view === 'tree'}
      <SummaryCards {rows} checked={results.size > 0} active={filter} onSelect={(f) => (filter = f)} />
      <div class="filters">
        <input type="search" placeholder={t('filter.search')} bind:value={query} />
        <label><input type="checkbox" bind:checked={directOnly} /> {t('filter.directOnly')}</label>
        <label><input type="checkbox" bind:checked={hideTest} /> {t('filter.hideTest')}</label>
        <span class="muted">{t('filter.count', { shown: visibleRows.length, total: rows.length })}</span>
      </div>
    {/if}
  {/if}

  <nav class="tabs" aria-label={t('tabs.label')}>
    <button class:active={view === 'list'} onclick={() => (view = 'list')}>{t('tabs.list')}</button>
    <button class:active={view === 'tree'} onclick={() => (view = 'tree')}>{t('tabs.tree')}</button>
    <button class:active={view === 'java'} onclick={() => (view = 'java')}>{t('tabs.java')}</button>
    <button class:active={view === 'log'} onclick={() => (view = 'log')}>
      {t('tabs.log')} <span class="muted small">{logEntries.filter((e) => e.kind !== 'app').length}</span>
    </button>
  </nav>

  {#if view === 'log'}
    <main class="single">
      <ActivityLogView entries={logEntries} {sources} disabledSources={options.disabledSources} {logDir} />
    </main>
  {:else if view === 'java'}
    <main class="single">
      <JavaKnowledgeView bind:options />
    </main>
  {:else if !source}
    <section class="empty">
      <p class="big">{t('empty.drop')}</p>
      <p class="muted">{t('empty.formats')}</p>
      <button onclick={openFile} disabled={!!busy}>{t('empty.choose')}</button>
    </section>
  {:else}
    <main class="split">
      {#if view === 'tree'}
        <TreeView {rows} matches={visibleIds} filtered={filtering} {selectedId} onSelect={(id) => (selectedId = id)} />
      {:else}
        <ComponentTable rows={visibleRows} {selectedId} onSelect={(id) => (selectedId = id)} />
      {/if}
      <DetailPane row={selectedRow} components={source.components} {options} onSelect={(id) => (selectedId = id)} />
    </main>
  {/if}
</div>

{#if showSettings}
  <SettingsDialog
    bind:options
    {sources}
    onClose={() => (showSettings = false)}
    onClearCache={async () => {
      await api.clearCache();
      await message(t('settings.cacheCleared'), { title: t('settings.title') });
    }}
  />
{/if}

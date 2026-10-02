<script lang="ts">
  import { openPath } from '@tauri-apps/plugin-opener';
  import { t, type MessageKey } from './i18n';
  import { formatDateTime, formatTime, timeZoneLabel } from './prefs.svelte';
  import type { DataSource, LogEntry, LogOutcome } from './types';

  let {
    entries,
    sources,
    disabledSources,
    logDir,
  }: { entries: LogEntry[]; sources: DataSource[]; disabledSources: string[]; logDir: string | null } = $props();

  const OUTCOME_KEY: Record<LogOutcome, MessageKey> = {
    network: 'log.outcome.network',
    cache: 'log.outcome.cache',
    blocked: 'log.outcome.blocked',
    error: 'log.outcome.error',
    info: 'log.outcome.info',
  };

  let sourceFilter = $state('all');
  let outcomeFilter = $state<'all' | LogOutcome>('all');
  let query = $state('');

  const summary = $derived(
    sources.map((s) => {
      const mine = entries.filter((e) => e.source === s.id);
      const count = (o: LogOutcome) => mine.filter((e) => e.outcome === o).length;
      const networked = mine.filter((e) => e.outcome === 'network');
      const last = networked[networked.length - 1];
      return {
        source: s,
        enabled: !disabledSources.includes(s.id),
        network: count('network'),
        cache: count('cache'),
        blocked: count('blocked'),
        error: count('error'),
        bytes: mine.reduce((n, e) => n + (e.outcome === 'network' ? (e.bytes ?? 0) : 0), 0),
        last: last?.ts ?? null,
      };
    }),
  );

  const shown = $derived(
    entries
      .filter((e) => sourceFilter === 'all' || e.source === sourceFilter || (sourceFilter === 'app' && e.kind === 'app'))
      .filter((e) => outcomeFilter === 'all' || e.outcome === outcomeFilter)
      .filter((e) => !query || `${e.url ?? ''} ${e.message ?? ''}`.toLowerCase().includes(query.toLowerCase()))
      .slice(-1000)
      .reverse(),
  );

  const sourceName = (id: string | null) => sources.find((s) => s.id === id)?.name ?? id ?? '';


  function size(bytes: number) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<div class="log-view">
  <section class="log-summary">
    <p class="muted small">
      {t('log.intro')}
    </p>
    <table class="compact">
      <thead>
        <tr><th>{t('log.col.source')}</th><th>{t('log.col.host')}</th><th>{t('detail.status')}</th><th>{t('log.outcome.network')}</th><th>{t('log.outcome.cache')}</th><th>{t('log.outcome.blocked')}</th><th>{t('log.outcome.error')}</th><th>{t('log.col.received')}</th><th>{t('log.col.last')}</th></tr>
      </thead>
      <tbody>
        {#each summary as s (s.source.id)}
          <tr>
            <td title={s.source.purpose}><b>{s.source.name}</b><div class="muted small">{s.source.purpose}</div></td>
            <td class="mono small">{s.source.hosts.join(', ') || t('log.localMvn')}</td>
            <td>{#if s.enabled}<span class="ok">{t('log.enabled')}</span>{:else}<span class="pill danger">{t('log.disabled')}</span>{/if}</td>
            <td class="num">{s.network}</td>
            <td class="num">{s.cache}</td>
            <td class="num" class:warn={s.blocked > 0}>{s.blocked}</td>
            <td class="num" class:warn={s.error > 0}>{s.error}</td>
            <td class="num">{size(s.bytes)}</td>
            <td title={formatDateTime(s.last)}>{s.last ? formatTime(s.last) : '—'}</td>
          </tr>
        {/each}
      </tbody>
    </table>
    <div class="log-file">
      <span class="muted small">{t('log.file')}</span>
      <span class="mono small break">{logDir ?? t('common.notSet')}</span>
      {#if logDir}<button onclick={() => logDir && openPath(logDir)}>{t('common.openFolder')}</button>{/if}
    </div>
  </section>

  <div class="filters">
    <select bind:value={sourceFilter} aria-label={t('log.col.source')}>
      <option value="all">{t('common.all')}</option>
      {#each sources as s}<option value={s.id}>{s.name}</option>{/each}
      <option value="app">{t('log.operations')}</option>
    </select>
    <select bind:value={outcomeFilter} aria-label={t('log.col.result')}>
      <option value="all">{t('log.allResults')}</option>
      {#each Object.entries(OUTCOME_KEY) as [k, key]}<option value={k}>{t(key)}</option>{/each}
    </select>
    <input type="search" placeholder={t('log.search')} bind:value={query} />
    <span class="muted">{t('common.count', { n: shown.length })}{shown.length === 1000 ? t('log.latest1000') : ''}</span>
  </div>

  <div class="table-wrap">
    <table class="log-table">
      <thead>
        <tr><th title={timeZoneLabel()}>{t('log.col.time')}</th><th>{t('log.col.source')}</th><th>{t('log.col.result')}</th><th>{t('log.col.content')}</th><th>{t('detail.status')}</th><th>{t('log.col.duration')}</th><th>{t('log.col.size')}</th></tr>
      </thead>
      <tbody>
        {#each shown as e, i (i)}
          <tr>
            <td class="mono small" title={formatDateTime(e.ts)}>{formatTime(e.ts)}</td>
            <td>{e.kind === 'app' ? t('log.operation') : sourceName(e.source)}</td>
            <td><span class="outcome {e.outcome}">{t(OUTCOME_KEY[e.outcome])}</span></td>
            <td class="log-url">
              {#if e.url}<span class="mono small">{e.method} {e.url}</span>{/if}
              {#if e.message}<div class="small" class:muted={!!e.url}>{e.message}</div>{/if}
            </td>
            <td class="num">{e.status ?? ''}</td>
            <td class="num">{e.durationMs != null ? `${e.durationMs} ms` : ''}</td>
            <td class="num">{e.bytes != null ? size(e.bytes) : ''}</td>
          </tr>
        {:else}
          <tr><td colspan="7" class="muted center">{t('log.empty')}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

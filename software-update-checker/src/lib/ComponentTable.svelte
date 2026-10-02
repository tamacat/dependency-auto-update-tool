<script lang="ts">
  import { date, displayName, severityLabel, severityRank, updateLabel } from './format';
  import { t } from './i18n';
  import type { Row, UpdateKind } from './types';

  let { rows, selectedId, onSelect }: { rows: Row[]; selectedId: string | null; onSelect: (id: string) => void } =
    $props();

  type SortKey = 'name' | 'version' | 'minor' | 'update' | 'vulns' | 'eol' | 'lastRelease';
  let sortKey = $state<SortKey>('vulns');
  let ascending = $state(false);

  const UPDATE_RANK: Record<UpdateKind, number> = { major: 4, minor: 3, patch: 2, unknown: 1, none: 0 };

  function sortValue(r: Row, key: SortKey): string | number {
    switch (key) {
      case 'name':
        return displayName(r.component).toLowerCase();
      case 'version':
        return r.component.version ?? '';
      case 'minor':
        return hasMinorUpdate(r) ? 1 : r.result?.version ? 0 : -1;
      case 'update':
        return r.result?.version ? UPDATE_RANK[r.result.version.updateKind] : -1;
      case 'vulns':
        return severityRank(r.maxSeverity) * 10_000 + (r.result?.vulnerabilities.length ?? 0);
      case 'eol':
        return r.result?.eol?.status === 'eol' ? 2 : r.result?.eol ? 1 : 0;
      case 'lastRelease':
        return r.result?.version?.lastReleaseAt ?? '';
    }
  }

  const hasMinorUpdate = (r: Row) => {
    const m = r.result?.version?.latestInMinor;
    return !!m && m !== r.component.version;
  };

  const sorted = $derived(
    [...rows].sort((a, b) => {
      const x = sortValue(a, sortKey);
      const y = sortValue(b, sortKey);
      const ord = x < y ? -1 : x > y ? 1 : 0;
      return (ascending ? ord : -ord) || displayName(a.component).localeCompare(displayName(b.component));
    }),
  );

  function sortBy(key: SortKey) {
    if (sortKey === key) ascending = !ascending;
    else {
      sortKey = key;
      ascending = key === 'name' || key === 'version';
    }
  }

  const columns = $derived<{ key: SortKey; label: string }[]>([
    { key: 'name', label: t('col.name') },
    { key: 'version', label: t('common.current') },
    { key: 'minor', label: t('col.latestInSeries') },
    { key: 'update', label: t('col.latestOverall') },
    { key: 'vulns', label: t('col.vulns') },
    { key: 'eol', label: t('col.support') },
    { key: 'lastRelease', label: t('col.lastRelease') },
  ]);

  function onKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const i = sorted.findIndex((r) => r.component.id === selectedId);
    const next = sorted[Math.min(sorted.length - 1, Math.max(0, i + (e.key === 'ArrowDown' ? 1 : -1)))];
    if (next) onSelect(next.component.id);
  }
</script>

<div class="table-wrap" tabindex="0" role="grid" aria-label={t('table.label')} onkeydown={onKey}>
  <table>
    <thead>
      <tr>
        {#each columns as col}
          <th>
            <button class="th" onclick={() => sortBy(col.key)}>
              {col.label}{#if sortKey === col.key}<span class="arrow">{ascending ? '▲' : '▼'}</span>{/if}
            </button>
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each sorted as r (r.component.id)}
        {@const c = r.component}
        {@const v = r.result?.version}
        {@const eol = r.result?.eol}
        <tr class:selected={c.id === selectedId} onclick={() => onSelect(c.id)}>
          <td class="name">
            <span class="eco">{c.ecosystem}</span>
            <span title={c.purl ?? ''}>{c.group ? `${c.group}:` : ''}<b>{c.name}</b></span>
            {#if c.direct === false}<span class="tag subtle">{t('table.transitive')}</span>{/if}
            {#if c.scope && !['compile', 'required'].includes(c.scope)}<span class="tag subtle">{c.scope}</span>{/if}
          </td>
          <td class="mono">{c.version ?? '—'}</td>
          <td>
            {#if v}
              {#if hasMinorUpdate(r)}
                <span class="mono">{v.latestInMinor}</span>
              {:else if v.latestInMinor}
                <span class="ok">{t('common.latest')}</span>
              {/if}
            {/if}
          </td>
          <td>
            {#if v}
              {#if v.updateKind === 'none'}
                <span class="ok">{t('common.latest')}</span>
              {:else}
                <span class="mono">{v.latest ?? '—'}</span>
                <span class="pill {v.updateKind}">{updateLabel(v.updateKind)}</span>
              {/if}
              {#if v.deprecated}<span class="pill danger" title={v.deprecated}>{t('cards.deprecated')}</span>{/if}
            {:else if r.result?.errors.length}
              <span class="muted" title={r.result.errors.join('\n')}>{t('cards.notFound')}</span>
            {/if}
          </td>
          <td>
            {#if r.maxSeverity}
              <span class="pill sev-{r.maxSeverity}">{severityLabel(r.maxSeverity)}</span>
              <span class="muted">{t('common.count', { n: r.result?.vulnerabilities.length ?? 0 })}</span>
            {:else if r.result}
              <span class="ok">{t('common.none')}</span>
            {/if}
          </td>
          <td>
            {#if eol}
              {#if eol.status === 'eol'}
                <span class="pill danger">EOL {date(eol.eolFrom)}</span>
              {:else if eol.status === 'supported'}
                <span class="ok">{t('common.supported')}</span>
              {:else}
                <span class="muted">{t('common.unknown')}</span>
              {/if}
            {/if}
          </td>
          <td class:warn={v?.stale}>{date(v?.lastReleaseAt)}</td>
        </tr>
      {:else}
        <tr><td colspan="7" class="muted center">{t('table.empty')}</td></tr>
      {/each}
    </tbody>
  </table>
</div>

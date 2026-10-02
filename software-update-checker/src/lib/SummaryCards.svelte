<script module lang="ts">
  import { is } from './format';
  export type Filter = 'all' | keyof typeof is;
</script>

<script lang="ts">
  import { t } from './i18n';
  import type { Row } from './types';

  let {
    rows,
    checked,
    active,
    onSelect,
  }: { rows: Row[]; checked: boolean; active: Filter; onSelect: (f: Filter) => void } = $props();

  const count = (f: (r: Row) => boolean) => rows.filter(f).length;
  const severe = $derived(rows.filter((r) => r.maxSeverity === 'critical' || r.maxSeverity === 'high').length);

  const cards = $derived<{ key: Filter; label: string; value: number; sub?: string; tone: string }[]>([
    { key: 'all', label: t('cards.all'), value: rows.length, tone: '' },
    { key: 'vulnerable', label: t('cards.vulnerable'), value: count(is.vulnerable), sub: t('cards.highOrAbove', { n: severe }), tone: 'danger' },
    { key: 'outdated', label: t('cards.outdated'), value: count(is.outdated), tone: 'warn' },
    { key: 'eol', label: t('cards.eol'), value: count(is.eol), tone: 'danger' },
    { key: 'stale', label: t('cards.stale'), value: count(is.stale), tone: 'warn' },
    { key: 'deprecated', label: t('cards.deprecated'), value: count(is.deprecated), tone: 'warn' },
    { key: 'notFound', label: t('cards.notFound'), value: count(is.notFound), tone: 'muted' },
  ]);
</script>

<div class="cards">
  {#each cards as card}
    <button
      class="card tone-{card.tone}"
      class:active={active === card.key}
      class:zero={checked && card.key !== 'all' && card.value === 0}
      disabled={!checked && card.key !== 'all'}
      onclick={() => onSelect(active === card.key ? 'all' : card.key)}
    >
      <span class="label">{card.label}</span>
      <span class="value">{checked || card.key === 'all' ? card.value : '–'}</span>
      {#if card.sub && checked}<span class="sub">{card.sub}</span>{/if}
    </button>
  {/each}
</div>

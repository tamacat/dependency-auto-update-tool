<script lang="ts">
  import { date, severityLabel, updateLabel } from './format';
  import { t } from './i18n';
  import type { Row } from './types';

  let {
    rows,
    matches,
    filtered,
    selectedId,
    onSelect,
  }: {
    rows: Row[];
    /** 絞り込み条件に合う部品の ID */
    matches: Set<string>;
    /** 絞り込みが有効か。有効なら条件に合う部品へ至る経路だけを表示する */
    filtered: boolean;
    selectedId: string | null;
    onSelect: (id: string) => void;
  } = $props();

  const byId = $derived(new Map(rows.map((r) => [r.component.id, r])));

  // ルート: 直接依存。直接/推移の情報が無ければ、どこからも参照されていない部品。
  const roots = $derived.by(() => {
    const direct = rows.filter((r) => r.component.direct === true);
    if (direct.length > 0) return direct.map((r) => r.component.id);
    const referenced = new Set(rows.flatMap((r) => r.component.dependencies));
    const top = rows.filter((r) => !referenced.has(r.component.id));
    return (top.length > 0 ? top : rows).map((r) => r.component.id);
  });

  // 部品 ID → その部品以下に条件に合う部品があるか（循環に備えて訪問中は false 扱い）
  const containsMatch = $derived.by(() => {
    const memo = new Map<string, boolean>();
    const visit = (id: string, visiting: Set<string>): boolean => {
      if (memo.has(id)) return memo.get(id)!;
      if (visiting.has(id)) return false;
      visiting.add(id);
      const children = byId.get(id)?.component.dependencies ?? [];
      const result = matches.has(id) || children.some((c) => visit(c, visiting));
      visiting.delete(id);
      memo.set(id, result);
      return result;
    };
    for (const r of rows) visit(r.component.id, new Set());
    return memo;
  });

  // 展開状態は「経路」単位（同じ部品が複数の親の下に出るため）
  let expanded = $state(new Set<string>());
  let collapsed = $state(new Set<string>());

  function isOpen(key: string, depth: number): boolean {
    if (filtered) return !collapsed.has(key);
    return expanded.has(key) || (depth === 0 && !collapsed.has(key));
  }

  function toggle(key: string, depth: number) {
    const open = isOpen(key, depth);
    const nextExpanded = new Set(expanded);
    const nextCollapsed = new Set(collapsed);
    if (open) {
      nextExpanded.delete(key);
      nextCollapsed.add(key);
    } else {
      nextCollapsed.delete(key);
      nextExpanded.add(key);
    }
    expanded = nextExpanded;
    collapsed = nextCollapsed;
  }

  function expandAll() {
    const keys = new Set<string>();
    const walk = (id: string, path: string, seen: Set<string>) => {
      if (seen.has(id)) return;
      keys.add(path);
      const next = new Set(seen).add(id);
      for (const c of byId.get(id)?.component.dependencies ?? []) walk(c, `${path}/${c}`, next);
    };
    for (const id of roots) walk(id, id, new Set());
    expanded = keys;
    collapsed = new Set();
  }

  function collapseAll() {
    expanded = new Set();
    collapsed = new Set(roots);
  }

  const visible = (id: string) => !filtered || containsMatch.get(id) === true;
  const hasTree = $derived(rows.some((r) => r.component.dependencies.length > 0));
</script>

<div class="tree-wrap">
  <div class="tree-actions">
    <button onclick={expandAll}>{t('tree.expandAll')}</button>
    <button onclick={collapseAll}>{t('tree.collapseAll')}</button>
    {#if filtered}<span class="muted small">{t('tree.filtered')}</span>{/if}
    {#if !hasTree}
      <span class="muted small">{t('tree.flat')}</span>
    {/if}
  </div>

  {#snippet node(id: string, path: string, depth: number, ancestors: Set<string>)}
    {@const r = byId.get(id)}
    {#if r && visible(id)}
      {@const c = r.component}
      {@const v = r.result?.version}
      {@const children = c.dependencies.filter((d) => byId.has(d) && visible(d))}
      {@const cycle = ancestors.has(id)}
      {@const open = !cycle && children.length > 0 && isOpen(path, depth)}
      <div
        class="tree-row"
        class:selected={id === selectedId}
        class:dim={filtered && !matches.has(id)}
        style="padding-left: {depth * 18 + 6}px"
        role="treeitem"
        aria-selected={id === selectedId}
        aria-expanded={children.length > 0 ? open : undefined}
        tabindex="-1"
        onclick={() => onSelect(id)}
        onkeydown={(e) => e.key === 'Enter' && onSelect(id)}
      >
        {#if children.length > 0 && !cycle}
          <button class="twisty" aria-label={open ? t('tree.collapse') : t('tree.expand')} onclick={(e) => { e.stopPropagation(); toggle(path, depth); }}>
            {open ? '▾' : '▸'}
          </button>
        {:else}
          <span class="twisty"></span>
        {/if}
        <span class="tree-name">{c.group ? `${c.group}:` : ''}<b>{c.name}</b></span>
        <span class="mono">{c.version ?? '—'}</span>
        {#if c.scope && !['compile', 'required'].includes(c.scope)}<span class="tag subtle">{c.scope}</span>{/if}
        {#if r.maxSeverity}<span class="pill sev-{r.maxSeverity}">{severityLabel(r.maxSeverity)} {r.result?.vulnerabilities.length}</span>{/if}
        {#if v && v.updateKind !== 'none' && v.updateKind !== 'unknown'}
          <span class="pill {v.updateKind}" title={`${t('common.latest')} ${v.latest}`}>{updateLabel(v.updateKind)}</span>
        {/if}
        {#if r.result?.eol?.status === 'eol'}<span class="pill danger">EOL {date(r.result.eol.eolFrom)}</span>{/if}
        {#if v?.stale}<span class="pill minor">{t('cards.stale')}</span>{/if}
        {#if cycle}<span class="muted small">{t('tree.cycle')}</span>{/if}
        {#if !open && children.length > 0 && !cycle}<span class="muted small">{children.length}</span>{/if}
      </div>
      {#if open}
        {@const next = new Set(ancestors).add(id)}
        {#each children as child (child)}
          {@render node(child, `${path}/${child}`, depth + 1, next)}
        {/each}
      {/if}
    {/if}
  {/snippet}

  <div class="tree" role="tree" aria-label={t('tree.label')}>
    {#each roots as id (id)}
      {@render node(id, id, 0, new Set())}
    {:else}
      <p class="muted center">{t('tree.empty')}</p>
    {/each}
  </div>
</div>

<script lang="ts">
  import { open, message, ask } from '@tauri-apps/plugin-dialog';
  import { openPath } from '@tauri-apps/plugin-opener';
  import * as api from './api';
  import { t } from './i18n';
  import { date } from './format';
  import { ruleKey, type CheckOptions, type JavaRule, type KnowledgeView, type RuleKey } from './types';

  let { options = $bindable() }: { options: CheckOptions } = $props();

  let view = $state<KnowledgeView | null>(null);
  let error = $state<string | null>(null);
  let query = $state('');
  let sourceFilter = $state<'all' | 'manual' | 'jar'>('all');
  let includeDetectedOnExport = $state(true);

  type Scope = 'version' | 'series' | 'all';
  interface Draft {
    group: string;
    artifact: string;
    scope: Scope;
    value: string;
    java: string;
    note: string;
    original: RuleKey | null;
  }
  let draft = $state<Draft | null>(null);

  async function reload() {
    try {
      view = await api.knowledgeList($state.snapshot(options.sharedKnowledgeFiles));
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  $effect(() => {
    options.sharedKnowledgeFiles.length;
    reload();
  });

  interface Item {
    rule: JavaRule;
    shared: string | null;
  }
  const items = $derived<Item[]>(
    view
      ? [
          ...view.local.map((rule) => ({ rule, shared: null })),
          ...view.shared.flatMap((f) => f.entries.map((rule) => ({ rule, shared: f.path }))),
        ]
      : [],
  );
  const shown = $derived(
    items
      .filter((i) => sourceFilter === 'all' || i.rule.source === sourceFilter)
      .filter((i) => {
        if (!query) return true;
        const r = i.rule;
        return `${r.group}:${r.artifact} ${r.version ?? ''} ${r.series ?? ''} ${r.note ?? ''}`.toLowerCase().includes(query.toLowerCase());
      })
      .sort((a, b) => `${a.rule.group}:${a.rule.artifact}`.localeCompare(`${b.rule.group}:${b.rule.artifact}`)),
  );
  const counts = $derived({
    manual: view?.local.filter((r) => r.source === 'manual').length ?? 0,
    jar: view?.local.filter((r) => r.source === 'jar').length ?? 0,
    shared: view?.shared.reduce((n, f) => n + f.entries.length, 0) ?? 0,
  });

  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const dirName = (p: string) => p.replace(/[\\/][^\\/]*$/, '');

  function scopeText(r: JavaRule) {
    const target = r.artifact === '*' ? t('kb.underGroup', { group: r.group }) : `${r.group}:${r.artifact}`;
    return { target, range: r.version ?? (r.series ? t('kb.seriesRange', { series: r.series }) : t('common.allVersions')) };
  }

  function startNew() {
    draft = { group: '', artifact: '', scope: 'series', value: '', java: '', note: '', original: null };
  }

  function startEdit(r: JavaRule) {
    draft = {
      group: r.group,
      artifact: r.artifact,
      scope: r.version ? 'version' : r.series ? 'series' : 'all',
      value: r.version ?? r.series ?? '',
      java: r.java ?? '',
      note: r.note ?? '',
      original: ruleKey(r),
    };
  }

  async function saveDraft() {
    if (!draft) return;
    const d = draft;
    const rule: JavaRule = {
      ecosystem: 'maven',
      group: d.group.trim(),
      artifact: d.artifact.trim() || '*',
      version: d.scope === 'version' ? d.value.trim() : null,
      series: d.scope === 'series' ? d.value.trim() : null,
      java: d.java.trim() || null,
      source: 'manual',
      checkedAt: new Date().toISOString(),
      note: d.note.trim() || null,
    };
    try {
      await api.knowledgeUpsert(rule, d.original);
      draft = null;
      await reload();
    } catch (e) {
      await message(String(e), { title: t('common.cannotRegister'), kind: 'error' });
    }
  }

  async function remove(r: JavaRule) {
    const { target, range } = scopeText(r);
    if (!(await ask(t('kb.confirmDelete', { target, range }), { title: t('common.delete'), kind: 'warning' }))) return;
    await api.knowledgeDelete(ruleKey(r)).catch((e) => message(String(e), { kind: 'error' }));
    await reload();
  }

  async function importFile() {
    const path = await open({ multiple: false, filters: [{ name: t('kb.fileType'), extensions: ['json'] }] });
    if (typeof path !== 'string') return;
    try {
      const s = await api.knowledgeImport(path);
      await message(
        t('kb.imported', { added: s.added, updated: s.updated }) +
          (s.skipped ? t('kb.importSkipped', { n: s.skipped }) : ''),
        { title: t('kb.importedTitle') },
      );
      await reload();
    } catch (e) {
      await message(String(e), { title: t('kb.importFailed'), kind: 'error' });
    }
  }

  async function exportFile() {
    try {
      const r = await api.knowledgeExport(includeDetectedOnExport);
      if (r) await message(t('kb.exported', { n: r.count, path: r.path }), { title: t('kb.exportedTitle') });
    } catch (e) {
      await message(String(e), { title: t('kb.exportFailed'), kind: 'error' });
    }
  }

  async function addShared() {
    const path = await open({ multiple: false, filters: [{ name: t('kb.fileType'), extensions: ['json'] }] });
    if (typeof path !== 'string' || options.sharedKnowledgeFiles.includes(path)) return;
    options.sharedKnowledgeFiles = [...options.sharedKnowledgeFiles, path];
  }

  function removeShared(path: string) {
    options.sharedKnowledgeFiles = options.sharedKnowledgeFiles.filter((p) => p !== path);
  }
</script>

<div class="knowledge">
  <section class="knowledge-head">
    <p class="muted small">
      {t('kb.intro')}
    </p>
    <div class="row">
      <span class="small">{t('kb.localFile')}</span>
      <span class="mono small break">{view?.localPath ?? t('common.notSet')}</span>
      {#if view?.localPath}<button onclick={() => view?.localPath && openPath(dirName(view.localPath))}>{t('common.openFolder')}</button>{/if}
    </div>
    {#if view?.localError}<p class="warn small">{view.localError} {t('kb.notSaving')}</p>{/if}
    <div class="row">
      <button class="primary" onclick={startNew}>{t('kb.registerManual')}</button>
      <button onclick={importFile}>{t('kb.import')}</button>
      <button onclick={exportFile}>{t('kb.export')}</button>
      <label class="small"><input type="checkbox" bind:checked={includeDetectedOnExport} /> {t('kb.includeDetected')}</label>
    </div>

    <div class="shared">
      <div class="row">
        <span class="small">{t('kb.sharedFiles')}</span>
        <button onclick={addShared}>{t('kb.add')}</button>
      </div>
      {#each view?.shared ?? [] as f (f.path)}
        <div class="row small">
          <span class="mono break">{f.path}</span>
          {#if f.error}<span class="warn">{f.error}</span>{:else}<span class="muted">{t('common.count', { n: f.entries.length })}</span>{/if}
          <button class="link" onclick={() => removeShared(f.path)}>{t('kb.remove')}</button>
        </div>
      {:else}
        <div class="muted small">{t('kb.noShared')}</div>
      {/each}
    </div>
  </section>

  {#if draft}
    <form class="draft" onsubmit={(e) => { e.preventDefault(); saveDraft(); }}>
      <label>groupId <input required bind:value={draft.group} placeholder="ch.qos.logback" /></label>
      <label>artifactId <input bind:value={draft.artifact} placeholder={t('kb.artifactPlaceholder')} /></label>
      <label>
        {t('kb.range')}
        <select bind:value={draft.scope}>
          <option value="version">{t('common.version')}</option>
          <option value="series">{t('common.series')}</option>
          <option value="all">{t('common.allVersions')}</option>
        </select>
      </label>
      {#if draft.scope !== 'all'}
        <label>{draft.scope === 'version' ? t('common.version') : t('common.series')} <input required bind:value={draft.value} placeholder={draft.scope === 'version' ? '1.4.14' : '1.4'} /></label>
      {/if}
      <label>{t('common.requiredJava')} <input bind:value={draft.java} placeholder="11" size="5" /></label>
      <label class="grow">{t('common.note')} <input bind:value={draft.note} placeholder={t('kb.notePlaceholder')} /></label>
      <button class="primary" type="submit">{draft.original ? t('common.update') : t('common.register')}</button>
      <button type="button" onclick={() => (draft = null)}>{t('common.cancel')}</button>
    </form>
  {/if}

  <div class="filters">
    <input type="search" placeholder={t('kb.search')} bind:value={query} />
    <select bind:value={sourceFilter} aria-label={t('kb.kind')}>
      <option value="all">{t('common.all')}</option>
      <option value="manual">{t('kb.manualRecords')}</option>
      <option value="jar">{t('kb.jarRecords')}</option>
    </select>
    <span class="muted small">{t('kb.counts', counts)}</span>
  </div>
  {#if error}<p class="warn small">{error}</p>{/if}

  <div class="table-wrap">
    <table class="log-table">
      <thead>
        <tr><th>{t('kb.col.target')}</th><th>{t('kb.range')}</th><th>{t('common.requiredJava')}</th><th>{t('kb.kind')}</th><th>{t('kb.col.origin')}</th><th>{t('kb.col.recorded')}</th><th>{t('common.note')}</th><th></th></tr>
      </thead>
      <tbody>
        {#each shown as item, i (i)}
          {@const r = item.rule}
          {@const s = scopeText(r)}
          <tr>
            <td class="mono small break">{s.target}</td>
            <td class="mono small">{s.range}</td>
            <td>{r.java ? `Java ${r.java}` : '—'}</td>
            <td>{#if r.source === 'manual'}<span class="pill minor">{t('common.manual')}</span>{:else}<span class="pill none">{t('common.jarDetected')}</span>{/if}</td>
            <td class="small" title={item.shared ?? ''}>{item.shared ? t('kb.sharedFrom', { file: fileName(item.shared) }) : t('kb.local')}</td>
            <td class="small" title={r.checkedAt ?? ''}>{date(r.checkedAt)}</td>
            <td class="small log-url">{r.note ?? ''}</td>
            <td class="actions-cell">
              {#if !item.shared}
                <button class="link" onclick={() => startEdit(r)}>{t('common.edit')}</button>
                <button class="link" onclick={() => remove(r)}>{t('common.delete')}</button>
              {/if}
            </td>
          </tr>
        {:else}
          <tr><td colspan="8" class="muted center">{t('kb.empty')}</td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>

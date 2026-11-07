<script lang="ts">
  import { untrack } from 'svelte';
  import { message } from '@tauri-apps/plugin-dialog';
  import * as api from './api';
  import { openExternal } from './external';
  import { date, displayName, percent, severityLabel, updateLabel } from './format';
  import { t, type MessageKey } from './i18n';
  import type { CheckOptions, Component, FixAdvice, JavaRequirement, JavaRule, KevInfo, Row } from './types';

  let {
    row,
    components,
    options,
    onSelect,
  }: { row: Row | null; components: Component[]; options: CheckOptions; onSelect: (id: string) => void } = $props();

  const byId = $derived(new Map(components.map((c) => [c.id, c])));
  // この部品を依存に持つもの（どの経路で入ってきたか）
  const dependents = $derived(row ? components.filter((c) => c.dependencies.includes(row.component.id)) : []);
  const dependencies = $derived(
    row ? row.component.dependencies.map((id) => byId.get(id)).filter((c): c is Component => !!c) : [],
  );

  // ---- Java 要件（Maven のみ）。ナレッジに記録があればそれを、無ければ Maven Central の jar から判定する
  let java = $state<Record<string, JavaRequirement>>({});
  let javaLoading = $state(false);
  let javaError = $state<string | null>(null);
  /** 手動登録の後に取り直すためのカウンター */
  let javaReload = $state(0);
  const mavenCentralEnabled = $derived(!options.disabledSources.includes('maven-central'));

  // 取得し直すきっかけは「どの部品のどのバージョンか（と参照先の設定）」だけにする
  // （取得処理の中で読む値まで追跡すると、結果を待たずに何度も再実行されるため）
  const javaRequest = $derived.by(() => {
    const c = row?.component;
    const v = row?.result?.version;
    if (!c || c.ecosystem !== 'maven' || !c.version || !v) return null;
    const versions = [...new Set([c.version, ...(v.latestInMinor ? [v.latestInMinor] : []), ...v.series.map((s) => s.latest)])];
    const settings = `${mavenCentralEnabled}|${options.sharedKnowledgeFiles.join(';')}|${javaReload}`;
    return { key: `${c.id}|${versions.join(',')}|${settings}`, componentId: c.id, versions };
  });
  const javaRequestKey = $derived(javaRequest?.key ?? null);

  $effect(() => {
    javaRequestKey;
    return untrack(loadJava);
  });

  function loadJava() {
    java = {};
    javaError = null;
    javaLoading = false;
    const req = javaRequest;
    const c = req ? byId.get(req.componentId) : undefined;
    if (!req || !c) return;
    let cancelled = false;
    javaLoading = true;
    api
      .javaRequirements($state.snapshot(c), req.versions, $state.snapshot(options))
      .then((list) => {
        if (!cancelled) java = Object.fromEntries(list.map((r) => [r.version, r]));
      })
      .catch((e) => !cancelled && (javaError = String(e)))
      .finally(() => !cancelled && (javaLoading = false));
    return () => {
      cancelled = true;
    };
  }

  const currentJava = $derived(row?.component.version ? java[row.component.version]?.java ?? null : null);

  /** 現行より新しい Java が必要か（`1.4` 形式も数値として比較）。 */
  function needsNewerJava(required: string | null | undefined): boolean {
    if (!required || !currentJava) return false;
    return parseFloat(required.replace(/^1\./, '0.')) > parseFloat(currentJava.replace(/^1\./, '0.'));
  }

  const ORIGIN_LABEL: Record<string, MessageKey | null> = {
    detected: 'java.origin.detected',
    local: 'java.origin.local',
    shared: 'java.origin.shared',
    none: null,
  };
  function originBadge(j: JavaRequirement): string {
    if (j.origin === 'local' && j.ruleSource === 'manual') return t('common.manual');
    const key = ORIGIN_LABEL[j.origin];
    return key ? t(key) : '';
  }
  function originTitle(j: JavaRequirement): string {
    const lines = [];
    if (j.origin === 'detected') lines.push(t('java.tip.detected'));
    if (j.ruleScope) lines.push(t('java.tip.rule', { scope: j.ruleScope, source: j.ruleSource === 'manual' ? t('common.manual') : t('common.jarDetected') }));
    if (j.originFile) lines.push(t('java.tip.file', { file: j.originFile }));
    if (j.classMajor) lines.push(t('java.tip.classMajor', { n: j.classMajor }));
    if (j.note) lines.push(j.note);
    return lines.join('\n');
  }

  // ---- 手動登録（その場で知見を残す）
  type ManualScope = 'version' | 'series' | 'group';
  let manual = $state<{ version: string; series: string; scope: ManualScope; java: string; note: string } | null>(null);

  function startManual(version: string, series: string, current: string | null) {
    manual = { version, series, scope: 'series', java: current ?? '', note: '' };
  }

  async function saveManual(c: Component) {
    if (!manual || !c.group) return;
    const m = manual;
    const rule: JavaRule = {
      ecosystem: 'maven',
      group: c.group,
      artifact: m.scope === 'group' ? '*' : c.name,
      version: m.scope === 'version' ? m.version : null,
      series: m.scope === 'version' ? null : m.series,
      java: m.java.trim() || null,
      source: 'manual',
      checkedAt: new Date().toISOString(),
      note: m.note.trim() || null,
    };
    try {
      await api.knowledgeUpsert(rule, null);
      manual = null;
      javaReload += 1;
    } catch (e) {
      await message(String(e), { title: t('common.cannotRegister'), kind: 'error' });
    }
  }

  /** 修正版の候補を、同じ版が続くときはまとめて表示する（同じ系列 = 同じメジャー = 最小 など）。 */
  function fixTargets(fix: FixAdvice): { labels: string[]; version: string }[] {
    const list: { labels: string[]; version: string }[] = [];
    const entries: [MessageKey, string | null][] = [
      ['detail.fixInSeries', fix.inSeries],
      ['detail.fixInMajor', fix.inMajor],
      ['detail.fixMinimum', fix.minimum],
    ];
    for (const [key, version] of entries) {
      if (!version) continue;
      const same = list.find((x) => x.version === version);
      if (same) same.labels.push(t(key));
      else list.push({ labels: [t(key)], version });
    }
    return list;
  }

  function kevTitle(k: KevInfo): string {
    const lines = [t('detail.kevTitle', { date: date(k.dateAdded) || '—', due: date(k.dueDate) || '—' })];
    if (k.ransomware) lines.push(t('detail.kevRansomware'));
    if (k.requiredAction) lines.push(k.requiredAction);
    return lines.join('\n');
  }

  const kevUrl = (cve: string) =>
    `https://www.cisa.gov/known-exploited-vulnerabilities-catalog?search_api_fulltext=${encodeURIComponent(cve)}`;
  const scorecardUrl = (repo: string) => `https://scorecard.dev/viewer/?uri=${encodeURIComponent(repo)}`;

  const REQUIREMENT_LABEL: Record<string, MessageKey> = {
    minJavaVersion: 'common.requiredJava',
    supportedJavaVersions: 'eol.req.supportedJava',
    javaVersion: 'eol.req.java',
    supportedJdkVersions: 'eol.req.supportedJdk',
  };
  const requirementText = (reqs: { key: string; value: string }[]) =>
    reqs.map((r) => `${REQUIREMENT_LABEL[r.key] ? t(REQUIREMENT_LABEL[r.key]) : r.key}: ${r.value}`).join(' / ');
</script>

<aside class="detail">
  {#if !row}
    <p class="muted center">{t('detail.placeholder')}</p>
  {:else}
    {@const c = row.component}
    {@const v = row.result?.version}
    {@const eol = row.result?.eol}
    <h2>{c.name}</h2>
    {#if c.group}<div class="muted">{c.group}</div>{/if}

    <dl>
      <dt>{t('detail.currentVersion')}</dt>
      <dd class="mono">
        {c.version ?? '—'}
        {#if v?.currentPublishedAt}<span class="muted">({date(v.currentPublishedAt)})</span>{/if}
        {#if currentJava}<span class="pill none">Java {currentJava}</span>{/if}
      </dd>
      <dt>{t('detail.ecosystem')}</dt>
      <dd>{c.ecosystem}{c.scope ? ` / ${c.scope}` : ''}{c.direct === true ? ` / ${t('detail.direct')}` : c.direct === false ? ` / ${t('detail.transitive')}` : ''}</dd>
      {#if c.purl}<dt>purl</dt><dd class="mono small break">{c.purl}</dd>{/if}
      <dt>{t('detail.license')}</dt>
      <dd>{row.licenses.join(', ') || '—'}</dd>
    </dl>

    {#if c.notes.length}
      <ul class="notes">{#each c.notes as n}<li>{n}</li>{/each}</ul>
    {/if}

    {#if row.result}
      <h3>{t('common.version')}</h3>
      {#if v}
        <dl>
          <dt>{t('col.latestInSeries')}</dt>
          <dd class="mono">
            {#if v.latestInMinor && v.latestInMinor !== c.version}
              {v.latestInMinor} <span class="muted small">← {t('detail.safestUpdate')}</span>
            {:else if v.latestInMinor}
              <span class="ok">{t('common.latest')} ({v.latestInMinor})</span>
            {:else}—{/if}
          </dd>
          <dt>{t('col.latestOverall')}</dt>
          <dd class="mono">
            {v.latest ?? '—'} {#if v.latestPublishedAt}<span class="muted">({date(v.latestPublishedAt)})</span>{/if}
            <span class="pill {v.updateKind}">{updateLabel(v.updateKind)}</span>
          </dd>
          <dt>{t('col.lastRelease')}</dt>
          <dd class:warn={v.stale}>{date(v.lastReleaseAt) || '—'} {#if v.stale}({t('cards.stale')}){/if}</dd>
          {#if v.deprecated}<dt>{t('cards.deprecated')}</dt><dd class="warn">{v.deprecated}</dd>{/if}
          {#if v.currentNotFound}<dt>{t('detail.caution')}</dt><dd class="warn">{t('detail.notInRegistry')}</dd>{/if}
        </dl>

        {#if v.series.length > 0}
          <h4>{t('detail.seriesTitle')}</h4>
          <p class="muted small">
            {t('detail.seriesNote')}
            {#if c.ecosystem === 'maven'}
              {#if !mavenCentralEnabled}
                {t('detail.javaOffline')}
              {:else if javaLoading}
                {t('detail.javaLoading')}
              {:else}
                {t('detail.javaNote')}
              {/if}
            {/if}
          </p>
          <table class="compact">
            <thead><tr><th>{t('common.series')}</th><th>{t('common.latest')}</th><th>{t('detail.published')}</th>{#if c.ecosystem === 'maven'}<th>{t('common.requiredJava')}</th>{/if}</tr></thead>
            <tbody>
              {#each v.series as s (s.series)}
                {@const j = java[s.latest]}
                <tr class:current-row={s.isCurrent}>
                  <td class="mono">{s.series}{#if s.isCurrent} <span class="tag">{t('common.current')}</span>{/if}</td>
                  <td class="mono">{s.latest}</td>
                  <td>{date(s.publishedAt)}</td>
                  {#if c.ecosystem === 'maven'}
                    <td class="java-cell" class:warn={needsNewerJava(j?.java)} title={j ? originTitle(j) : ''}>
                      {#if j?.java}Java {j.java}{#if needsNewerJava(j.java)} ↑{/if}{:else if j}<span class="muted small">{t('common.unknown')}</span>{:else if javaLoading}…{/if}
                      {#if j && originBadge(j)}<span class="origin">{originBadge(j)}</span>{/if}
                      <button class="link edit" title={t('detail.registerJavaTitle')} onclick={() => startManual(s.latest, s.series, j?.java ?? null)}>✎</button>
                    </td>
                  {/if}
                </tr>
              {/each}
            </tbody>
          </table>
          {#if javaError}<p class="warn small">{javaError}</p>{/if}
          {#if manual}
            <form class="manual" onsubmit={(e) => { e.preventDefault(); saveManual(c); }}>
              <div class="small"><b>{t('detail.registerJava')}</b> {t('detail.registerJavaNote')}</div>
              <label class="small"><input type="radio" bind:group={manual.scope} value="version" /> {t('detail.scopeVersion', { name: c.name, version: manual.version })}</label>
              <label class="small"><input type="radio" bind:group={manual.scope} value="series" /> {t('detail.scopeSeries', { name: c.name, series: manual.series })}</label>
              <label class="small"><input type="radio" bind:group={manual.scope} value="group" /> {t('detail.scopeGroup', { group: c.group ?? '', series: manual.series })}</label>
              <div class="row">
                <label class="small">Java <input bind:value={manual.java} size="4" placeholder="11" /></label>
                <input class="grow" bind:value={manual.note} placeholder={t('detail.notePlaceholder')} />
              </div>
              <div class="row">
                <button class="primary" type="submit">{t('common.register')}</button>
                <button type="button" onclick={() => (manual = null)}>{t('common.cancel')}</button>
              </div>
            </form>
          {/if}
        {/if}
      {:else}
        <p class="muted">{t('detail.noInfo')}</p>
      {/if}

      <h3>{t('col.vulns')} ({row.result.vulnerabilities.length})</h3>
      {#if row.result.vulnerabilities.some((x) => x.malicious)}
        <p class="alert">{t('detail.maliciousNote')}</p>
      {/if}
      {#if row.result.fix}
        {@const fix = row.result.fix}
        {@const targets = fixTargets(fix)}
        <dl>
          <dt>{t('detail.fixTitle')}</dt>
          <dd>
            {#each targets as target}
              <div><span class="mono">{target.version}</span> <span class="muted small">({target.labels.join(' / ')})</span></div>
            {:else}
              <span class="warn">{t('detail.fixNone')}</span>
            {/each}
            {#if fix.unfixed.length && targets.length}
              <div class="warn small">{t('detail.unfixed')}: <span class="mono">{fix.unfixed.join(', ')}</span></div>
            {/if}
          </dd>
        </dl>
      {/if}
      {#each row.result.vulnerabilities as vuln (vuln.id)}
        {@const cve = vuln.aliases.find((a) => a.startsWith('CVE-')) ?? (vuln.id.startsWith('CVE-') ? vuln.id : undefined)}
        <div class="vuln">
          <div>
            {#if vuln.malicious}<span class="pill sev-critical">{t('detail.malicious')}</span>{/if}
            {#if vuln.kev}
              {@const kev = vuln.kev}
              <button class="pill sev-critical kev" title={kevTitle(kev)} onclick={() => openExternal(kevUrl(kev.cve))}>KEV{kev.ransomware ? ' ⚠' : ''}</button>
            {/if}
            <span class="pill sev-{vuln.severity}">{severityLabel(vuln.severity)}{vuln.score != null ? ` ${vuln.score.toFixed(1)}` : ''}</span>
            <button class="link mono" onclick={() => openExternal(vuln.url)}>{vuln.id}</button>
            {#if cve}<span class="mono muted">{cve}</span>{/if}
            {#if vuln.epss}
              <span class="small epss" class:warn={vuln.epss.score >= 0.1} title={t('detail.epssTitle', { date: date(vuln.epss.date) || '—' })}>
                {t('detail.epss', { score: percent(vuln.epss.score), top: percent(Math.max(0, 1 - vuln.epss.percentile)) })}
              </span>
            {/if}
          </div>
          {#if vuln.summary}<div>{vuln.summary}</div>{/if}
          {#if vuln.fixedVersions.length}
            <div class="small">{t('detail.fixedIn')}: <span class="mono">{vuln.fixedVersions.join(', ')}</span></div>
          {/if}
        </div>
      {:else}
        <p class="muted">{t('detail.noVulns')}</p>
      {/each}

      <h3>{t('detail.support')}</h3>
      {#if eol}
        <dl>
          <dt>{t('detail.product')}</dt>
          <dd>
            <button class="link" onclick={() => openExternal(eol.link)}>{eol.productLabel}</button>
            {#if eol.matchKind === 'inferred'}<span class="muted small">({t('detail.inferred')})</span>{/if}
          </dd>
          <dt>{t('detail.status')}</dt>
          <dd class:warn={eol.status === 'eol'}>
            {eol.cycle ?? t('detail.noCycle')}:
            {eol.status === 'eol' ? t('detail.eolOn', { date: date(eol.eolFrom) }) : eol.status === 'supported' ? t('common.supported') : t('common.unknown')}
            {#if eol.status === 'supported' && eol.eolFrom}<span class="muted">{t('detail.eolPlanned', { date: date(eol.eolFrom) })}</span>{/if}
          </dd>
        </dl>
        {#if eol.cycles.length > 0}
          <table class="compact">
            <thead><tr><th>{t('detail.cycle')}</th><th>{t('detail.status')}</th><th>{t('common.latest')}</th><th>{t('detail.requirements')}</th></tr></thead>
            <tbody>
              {#each eol.cycles as cy (cy.name)}
                <tr class:current-row={cy.isCurrent}>
                  <td class="mono">{cy.name}{#if cy.isCurrent} <span class="tag">{t('common.current')}</span>{/if}</td>
                  <td class:warn={cy.status === 'eol'}>
                    {cy.status === 'eol' ? t('detail.cycleEnded', { date: date(cy.eolFrom) }) : cy.eolFrom ? t('detail.cycleUntil', { date: date(cy.eolFrom) }) : t('common.supported')}
                  </td>
                  <td class="mono">{cy.latest ?? ''}</td>
                  <td class="small">{requirementText(cy.requirements)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      {:else}
        <p class="muted">{t('detail.noProduct')}</p>
      {/if}

      {#if c.ecosystem !== 'other'}
        {@const sc = row.result.supplyChain}
        <h3>{t('detail.supplyChain')}</h3>
        {#if !options.supplyChain}
          <p class="muted small">{t('detail.supplyChainOff')}</p>
        {:else if sc}
          <dl>
            <dt>{t('detail.sourceRepo')}</dt>
            <dd class="break">
              {#if sc.sourceRepo}
                <span class="mono small">{sc.sourceRepo}</span>
                <span class="muted small">({sc.sourceRepoVerified ? t('detail.repoVerified') : t('detail.repoUnverified')})</span>
              {:else}—{/if}
            </dd>
            <dt>{t('detail.scorecard')}</dt>
            <dd>
              {#if sc.scorecard && sc.sourceRepo}
                {@const repo = sc.sourceRepo}
                <button class="link" class:warn={sc.scorecard.overall < 5} onclick={() => openExternal(scorecardUrl(repo))}>{sc.scorecard.overall.toFixed(1)} / 10</button>
                <span class="muted small">({date(sc.scorecard.date)})</span>
              {:else}—{/if}
            </dd>
            <dt>{t('detail.provenance')}</dt>
            <dd>
              {#if sc.provenance.some((p) => p.verified)}
                <span class="ok">{t('detail.provenanceVerified')}</span>
              {:else if sc.provenance.length}
                <span class="warn">{t('detail.provenanceUnverified')}</span>
              {:else}
                <span class="muted">{t('detail.provenanceNone')}</span>
              {/if}
            </dd>
          </dl>
          {#if sc.scorecard?.checks.length}
            <details>
              <summary class="small">{t('detail.scorecardChecks')}</summary>
              <table class="compact">
                <tbody>
                  {#each sc.scorecard.checks as check (check.name)}
                    <tr>
                      <td>{check.name}</td>
                      <td class="mono" class:warn={check.score != null && check.score < 5}>{check.score ?? '—'}</td>
                      <td class="small muted">{check.reason ?? ''}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </details>
          {/if}
        {:else}
          <p class="muted">{t('detail.noInfo')}</p>
        {/if}
      {/if}

      {#if row.result.errors.length}
        <h3>{t('detail.errors')}</h3>
        <ul class="notes">{#each row.result.errors as e}<li>{e}</li>{/each}</ul>
      {/if}
    {:else}
      <p class="muted">{t('detail.runCheck')}</p>
    {/if}

    {#if dependents.length}
      <h3>{t('detail.dependents')} ({dependents.length})</h3>
      <ul class="links">
        {#each dependents as d}<li><button class="link" onclick={() => onSelect(d.id)}>{displayName(d)}</button></li>{/each}
      </ul>
    {/if}
    {#if dependencies.length}
      <h3>{t('detail.dependencies')} ({dependencies.length})</h3>
      <ul class="links">
        {#each dependencies as d}<li><button class="link" onclick={() => onSelect(d.id)}>{displayName(d)}</button></li>{/each}
      </ul>
    {/if}
  {/if}
</aside>

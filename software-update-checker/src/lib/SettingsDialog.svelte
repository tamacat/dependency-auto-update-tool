<script lang="ts">
  import { getVersion } from '@tauri-apps/api/app';
  import { t } from './i18n';
  import { availableTimeZones, FONT_SCALES, isValidTimeZone, prefs, systemTimeZone } from './prefs.svelte';
  import * as api from './api';
  import { date } from './format';
  import {
    DEFAULT_NETWORK,
    DEFAULT_OPTIONS,
    NETWORK_LIMITS,
    type CheckOptions,
    type DataSource,
    type NetworkSettings,
    type RuntimeStatus,
    type RuntimeTargets,
  } from './types';
  import type { MessageKey } from './i18n';

  let {
    options = $bindable(),
    sources,
    onClose,
    onClearCache,
  }: { options: CheckOptions; sources: DataSource[]; onClose: () => void; onClearCache: () => void } = $props();

  let dialog: HTMLDialogElement;
  $effect(() => dialog.showModal());

  // ---- このソフトウェアについて
  let version = $state('');
  getVersion()
    .then((v) => (version = v))
    .catch(() => {});

  /** アプリが表示する外部データの出典とライセンス（README の「データソース」と同じ） */
  const DATA_ATTRIBUTIONS = $derived<{ name: string; url: string; license: string; licenseUrl: string }[]>([
    { name: 'deps.dev (Open Source Insights)', url: 'https://deps.dev/', license: 'CC BY 4.0', licenseUrl: 'https://creativecommons.org/licenses/by/4.0/' },
    { name: 'OSV.dev / GitHub Advisory Database', url: 'https://osv.dev/', license: 'CC BY 4.0', licenseUrl: 'https://creativecommons.org/licenses/by/4.0/' },
    { name: 'endoflife.date', url: 'https://endoflife.date/', license: 'MIT', licenseUrl: 'https://github.com/endoflife-date/endoflife.date/blob/master/LICENSE' },
    { name: 'CISA Known Exploited Vulnerabilities Catalog', url: 'https://www.cisa.gov/known-exploited-vulnerabilities-catalog', license: 'CC0 1.0', licenseUrl: 'https://github.com/cisagov/kev-data/blob/develop/LICENSE' },
    { name: 'EPSS (FIRST)', url: 'https://www.first.org/epss/', license: t('about.epssTerms'), licenseUrl: 'https://www.first.org/epss/faq' },
  ]);

  let noticesDialog: HTMLDialogElement;
  let notices = $state<string | null>(null);

  async function showNotices() {
    // 約 600KB あるので、開いたときに初めて読み込む（別ファイルとして同梱される）
    if (notices === null) notices = (await import('../../THIRD-PARTY-NOTICES.txt?raw')).default;
    noticesDialog.showModal();
  }

  const NETWORK_FIELDS: { key: keyof NetworkSettings; label: MessageKey; unit: MessageKey }[] = [
    { key: 'maxRetries', label: 'net.maxRetries', unit: 'unit.times' },
    { key: 'retryBaseMs', label: 'net.retryBaseMs', unit: 'unit.ms' },
    { key: 'maxRetryWaitMs', label: 'net.maxRetryWaitMs', unit: 'unit.ms' },
    { key: 'timeoutSecs', label: 'net.timeoutSecs', unit: 'unit.seconds' },
    { key: 'maxConcurrency', label: 'net.maxConcurrency', unit: 'unit.requests' },
    { key: 'minIntervalMs', label: 'net.minIntervalMs', unit: 'unit.ms' },
    { key: 'maxRequestsPerMinute', label: 'net.maxRequestsPerMinute', unit: 'unit.requests' },
    { key: 'maxSameRequestPerMinute', label: 'net.maxSameRequestPerMinute', unit: 'unit.times' },
    { key: 'failureThreshold', label: 'net.failureThreshold', unit: 'unit.times' },
    { key: 'cooldownSecs', label: 'net.cooldownSecs', unit: 'unit.seconds' },
  ];

  const timeZones = availableTimeZones();
  let timeZoneInput = $state(prefs.timeZone === 'system' ? '' : prefs.timeZone);
  const timeZoneInvalid = $derived(timeZoneInput.trim() !== '' && !isValidTimeZone(timeZoneInput.trim()));

  function setTimeZoneMode(mode: 'system' | 'custom') {
    if (mode === 'system') {
      prefs.timeZone = 'system';
    } else {
      const tz = timeZoneInput.trim() || systemTimeZone();
      timeZoneInput = tz;
      prefs.timeZone = tz;
    }
  }

  function applyTimeZoneInput() {
    const tz = timeZoneInput.trim();
    if (tz && isValidTimeZone(tz)) prefs.timeZone = tz;
  }

  function setNetwork(key: keyof NetworkSettings, raw: string) {
    const [min, max] = NETWORK_LIMITS[key];
    const n = Math.round(Number(raw));
    if (Number.isFinite(n)) options.network = { ...options.network, [key]: Math.min(max, Math.max(min, n)) };
  }

  function setEnabled(id: string, enabled: boolean) {
    const rest = options.disabledSources.filter((d) => d !== id);
    options.disabledSources = enabled ? rest : [...rest, id];
  }

  // ---- 使う実行環境。入力が落ち着いてからサポート状況を調べる
  const RUNTIMES: { key: keyof RuntimeTargets; label: string; placeholder: string }[] = [
    { key: 'java', label: 'Java', placeholder: '21' },
    { key: 'node', label: 'Node.js', placeholder: '20.11' },
    { key: 'python', label: 'Python', placeholder: '3.12' },
  ];
  let runtimeStatus = $state<Record<string, RuntimeStatus>>({});
  const runtimeKey = $derived(JSON.stringify([options.runtimeTargets, options.disabledSources.includes('endoflife.date')]));
  $effect(() => {
    runtimeKey;
    let cancelled = false;
    const timer = setTimeout(() => {
      api
        .runtimeStatus($state.snapshot(options))
        .then((list) => !cancelled && (runtimeStatus = Object.fromEntries(list.map((s) => [s.runtime, s]))))
        .catch(() => !cancelled && (runtimeStatus = {}));
    }, 500);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  });

  function runtimeStatusText(s: RuntimeStatus | undefined): { text: string; warn: boolean } | null {
    if (!s) return null;
    if (s.runtime === 'java') return { text: t('settings.runtimeJavaNote'), warn: false };
    if (s.error) return { text: s.error, warn: false };
    const e = s.eol;
    if (!e || !e.cycle) return { text: t('settings.runtimeNoCycle'), warn: false };
    if (e.status === 'eol') return { text: t('settings.runtimeEol', { cycle: e.cycle, date: date(e.eolFrom) }), warn: true };
    if (e.eolFrom) return { text: t('settings.runtimeUntil', { cycle: e.cycle, date: date(e.eolFrom) }), warn: false };
    return { text: t('settings.runtimeSupported', { cycle: e.cycle }), warn: false };
  }

  function resetAll() {
    Object.assign(options, {
      ...DEFAULT_OPTIONS,
      disabledSources: [],
      network: { ...DEFAULT_NETWORK },
      // 共有ナレッジの指定と使っている実行環境は判定の設定ではないので残す
      sharedKnowledgeFiles: options.sharedKnowledgeFiles,
      runtimeTargets: options.runtimeTargets,
    });
  }
</script>

<!-- 画面の高さを超えないよう、見出しとボタン行を固定して本文だけをスクロールさせる -->
<dialog bind:this={dialog} onclose={onClose} class="settings" aria-labelledby="settings-title">
  <header class="dialog-head">
    <h2 id="settings-title">{t('settings.title')}</h2>
    <button class="icon" aria-label={t('common.close')} title={t('common.close')} onclick={() => dialog.close()}>×</button>
  </header>

  <div class="dialog-body">
    <fieldset>
      <legend>{t('settings.display')}</legend>
      <div class="form-grid">
        <label for="pref-locale">{t('settings.language')}</label>
        <select id="pref-locale" bind:value={prefs.locale}>
          <option value="ja">日本語</option>
          <option value="en">English</option>
        </select>

        <label for="pref-font">{t('settings.fontSize')}</label>
        <span class="row">
          <select id="pref-font" bind:value={prefs.fontScale}>
            {#each FONT_SCALES as scale}
              <option value={scale}>{Math.round(scale * 100)}%{scale === 1 ? ` (${t('settings.default')})` : ''}</option>
            {/each}
          </select>
          <span class="muted small">{t('settings.fontSample')}</span>
        </span>

        <span>{t('settings.timeZone')}</span>
        <span class="tz">
          <label class="small">
            <input type="radio" name="tz" checked={prefs.timeZone === 'system'} onchange={() => setTimeZoneMode('system')} />
            {t('settings.timeZoneSystem', { zone: systemTimeZone() })}
          </label>
          <label class="small">
            <input type="radio" name="tz" checked={prefs.timeZone !== 'system'} onchange={() => setTimeZoneMode('custom')} />
            {t('settings.timeZoneCustom')}
            <input
              list="time-zones"
              bind:value={timeZoneInput}
              onchange={applyTimeZoneInput}
              disabled={prefs.timeZone === 'system'}
              placeholder="Asia/Tokyo"
              aria-invalid={timeZoneInvalid}
            />
          </label>
          <datalist id="time-zones">
            {#each timeZones as tz}<option value={tz}></option>{/each}
          </datalist>
          {#if timeZoneInvalid}<span class="warn small">{t('settings.timeZoneInvalid')}</span>{/if}
          <span class="muted small">{t('settings.timeZoneNote')}</span>
        </span>
      </div>
    </fieldset>

    <fieldset>
      <legend>{t('settings.sources')}</legend>
      <p class="muted small">{t('settings.sourcesNote')}</p>
      {#each sources as s (s.id)}
        <label class="source">
          <input
            type="checkbox"
            checked={!options.disabledSources.includes(s.id)}
            onchange={(e) => setEnabled(s.id, e.currentTarget.checked)}
          />
          <span>
            <b>{s.name}</b> <span class="mono small muted">{s.hosts.join(', ') || t('settings.localProcess')}</span>
            <span class="small">{s.purpose}</span>
            <span class="small muted">{t('settings.sends', { what: s.sends })}</span>
          </span>
        </label>
      {/each}
    </fieldset>

    <fieldset>
      <legend>{t('settings.network')}</legend>
      <p class="muted small">{t('settings.networkNote')}</p>
      <div class="network-grid">
        {#each NETWORK_FIELDS as f (f.key)}
          {@const [min, max] = NETWORK_LIMITS[f.key]}
          <label for="net-{f.key}" class="small">{t(f.label)}</label>
          <span class="small">
            <input
              id="net-{f.key}"
              type="number"
              {min}
              {max}
              value={options.network[f.key]}
              onchange={(e) => setNetwork(f.key, e.currentTarget.value)}
            />
            {t(f.unit)} <span class="muted">({min}–{max})</span>
          </span>
        {/each}
      </div>
      <button onclick={() => (options.network = { ...DEFAULT_NETWORK })}>{t('settings.networkReset')}</button>
    </fieldset>

    <fieldset>
      <legend>{t('settings.judgement')}</legend>
      <label><input type="checkbox" bind:checked={options.includePrerelease} /> {t('settings.includePrerelease')}</label>
      <label><input type="checkbox" bind:checked={options.supplyChain} /> {t('settings.supplyChain')}</label>
      <label>
        {t('settings.staleBefore')}
        <input type="number" min="1" max="20" bind:value={options.staleYears} />
        {t('settings.staleAfter')}
      </label>
    </fieldset>

    <fieldset>
      <legend>{t('settings.runtimes')}</legend>
      <p class="muted small">{t('settings.runtimesNote')}</p>
      <div class="form-grid runtimes">
        {#each RUNTIMES as r (r.key)}
          {@const status = runtimeStatusText(runtimeStatus[r.key])}
          <label for="runtime-{r.key}">{r.label}</label>
          <span class="row">
            <input id="runtime-{r.key}" bind:value={options.runtimeTargets[r.key]} placeholder={r.placeholder} size="8" />
            {#if status && options.runtimeTargets[r.key].trim()}
              <span class="small" class:warn={status.warn} class:muted={!status.warn}>{status.text}</span>
            {/if}
          </span>
        {/each}
      </div>
    </fieldset>

    <fieldset>
      <legend>{t('settings.cache')}</legend>
      <label><input type="checkbox" bind:checked={options.useCache} /> {t('settings.useCache')}</label>
      <button onclick={onClearCache}>{t('settings.clearCache')}</button>
    </fieldset>

    <fieldset>
      <legend>{t('about.title')}</legend>
      <div class="form-grid">
        <span>{t('about.version')}</span>
        <span class="mono">Software Update Checker {version}</span>
        <span>{t('about.license')}</span>
        <span>Apache License 2.0 <span class="muted small">(Copyright 2026 tamacat.org)</span></span>
        <span>{t('about.source')}</span>
        <span class="mono small break">https://github.com/tamacat/dependency-auto-update-tool</span>
      </div>
      <div class="small">{t('about.dataSources')}</div>
      <ul class="links small">
        {#each DATA_ATTRIBUTIONS as d (d.name)}
          <li>{d.name} <span class="muted">({d.url})</span> — {d.license} <span class="muted">({d.licenseUrl})</span></li>
        {/each}
      </ul>
      <p class="muted small">{t('about.dataNote')}</p>
      <button onclick={showNotices}>{t('about.showNotices')}</button>
    </fieldset>
  </div>

  <footer class="dialog-actions">
    <button onclick={resetAll}>{t('settings.resetAll')}</button>
    <button class="primary" onclick={() => dialog.close()}>{t('common.close')}</button>
  </footer>

  <!-- 第三者ライセンスの全文（exe 単体で配布されても表記が失われないよう、アプリに同梱している） -->
  <dialog bind:this={noticesDialog} class="settings notices" aria-labelledby="notices-title">
    <header class="dialog-head">
      <h2 id="notices-title">{t('about.noticesTitle')}</h2>
      <button class="icon" aria-label={t('common.close')} title={t('common.close')} onclick={() => noticesDialog.close()}>×</button>
    </header>
    <div class="dialog-body">
      <pre class="notices-text">{notices ?? ''}</pre>
    </div>
    <footer class="dialog-actions">
      <button class="primary" onclick={() => noticesDialog.close()}>{t('common.close')}</button>
    </footer>
  </dialog>
</dialog>

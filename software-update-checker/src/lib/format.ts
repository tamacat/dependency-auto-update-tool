import { t, type MessageKey } from './i18n';
import { formatDate } from './prefs.svelte';
import type { Component, Row, Severity, UpdateKind } from './types';

export const SEVERITY_ORDER: Severity[] = ['critical', 'high', 'medium', 'low', 'unknown'];

const SEVERITY_KEY: Record<Severity, MessageKey> = {
  critical: 'severity.critical',
  high: 'severity.high',
  medium: 'severity.medium',
  low: 'severity.low',
  unknown: 'severity.unknown',
};

const UPDATE_KEY: Record<UpdateKind, MessageKey> = {
  none: 'update.none',
  patch: 'update.patch',
  minor: 'update.minor',
  major: 'update.major',
  unknown: 'update.unknown',
};

const PHASE_KEY: Record<string, MessageKey> = {
  versions: 'phase.versions',
  licenses: 'phase.licenses',
  vulnerabilities: 'phase.vulnerabilities',
  vulnerabilityDetails: 'phase.vulnerabilityDetails',
  eol: 'phase.eol',
  done: 'phase.done',
};

export const severityLabel = (s: Severity) => t(SEVERITY_KEY[s]);
export const updateLabel = (k: UpdateKind) => t(UPDATE_KEY[k]);
export const phaseLabel = (p: string) => (PHASE_KEY[p] ? t(PHASE_KEY[p]) : p);

export function displayName(c: Component): string {
  return c.group ? `${c.group}:${c.name}` : c.name;
}

/** 日付（YYYY-MM-DD）。日時なら設定のタイムゾーンでの日付にする。 */
export function date(s: string | null | undefined): string {
  return formatDate(s);
}

export function severityRank(s: Severity | null): number {
  return s ? 5 - SEVERITY_ORDER.indexOf(s) : 0;
}

export function maxSeverity(severities: Severity[]): Severity | null {
  let best: Severity | null = null;
  for (const s of severities) {
    if (severityRank(s) > severityRank(best)) best = s;
  }
  return best;
}

/** 行に「要対応」の印を付ける条件（ダッシュボードとフィルターで共通）。 */
export const is = {
  vulnerable: (r: Row) => (r.result?.vulnerabilities.length ?? 0) > 0,
  outdated: (r: Row) => {
    const k = r.result?.version?.updateKind;
    return k === 'major' || k === 'minor' || k === 'patch';
  },
  eol: (r: Row) => r.result?.eol?.status === 'eol',
  stale: (r: Row) => r.result?.version?.stale === true,
  deprecated: (r: Row) => !!r.result?.version?.deprecated,
  notFound: (r: Row) => r.result?.version?.currentNotFound === true || (r.result?.errors.length ?? 0) > 0,
};

import { date, displayName } from './format';
import type { ImportResult, Row } from './types';

function csvCell(value: unknown): string {
  const s = value == null ? '' : String(value);
  return /[",\r\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

export function toCsv(rows: Row[]): string {
  const header = [
    'name', 'ecosystem', 'version', 'scope', 'direct',
    'latest', 'latestInMajor', 'latestInMinor', 'updateKind', 'lastReleaseAt', 'stale', 'deprecated',
    'vulnerabilityCount', 'maxSeverity', 'vulnerabilityIds',
    'eolProduct', 'eolCycle', 'eolStatus', 'eolFrom',
    'licenses', 'purl', 'errors',
  ];
  const lines = [header.join(',')];
  for (const r of rows) {
    const c = r.component;
    const v = r.result?.version;
    const e = r.result?.eol;
    const vulns = r.result?.vulnerabilities ?? [];
    lines.push(
      [
        displayName(c), c.ecosystem, c.version, c.scope, c.direct,
        v?.latest, v?.latestInMajor, v?.latestInMinor, v?.updateKind, date(v?.lastReleaseAt), v?.stale, v?.deprecated,
        vulns.length, r.maxSeverity, vulns.map((x) => x.aliases.find((a) => a.startsWith('CVE-')) ?? x.id).join(' '),
        e?.product, e?.cycle, e?.status, e?.eolFrom,
        r.licenses.join(' | '), c.purl, r.result?.errors.join(' | '),
      ]
        .map(csvCell)
        .join(','),
    );
  }
  // Excel で文字化けしないよう BOM を付ける
  return '﻿' + lines.join('\r\n');
}

export function toJson(source: ImportResult, rows: Row[]): string {
  return JSON.stringify(
    {
      generatedAt: new Date().toISOString(),
      source: {
        path: source.sourcePath,
        format: source.format,
        projectName: source.projectName,
        includesTransitive: source.includesTransitive,
      },
      components: rows.map((r) => ({ ...r.component, check: r.result })),
    },
    null,
    2,
  );
}

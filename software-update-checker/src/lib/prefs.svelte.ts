// 表示の設定（言語・文字サイズ・タイムゾーン）。チェックの設定（CheckOptions）とは別に、
// この端末の表示の好みとしてブラウザ領域に保存する。

export type Locale = 'ja' | 'en';

export interface DisplayPrefs {
  locale: Locale;
  /** 文字サイズの倍率（0.85〜1.5） */
  fontScale: number;
  /** 'system'（OS の設定）または IANA のタイムゾーン名（Asia/Tokyo、UTC など） */
  timeZone: string;
}

export const FONT_SCALES = [0.85, 1, 1.15, 1.3, 1.5] as const;

const KEY = 'software-update-checker.display';

function defaultLocale(): Locale {
  try {
    return navigator.language.toLowerCase().startsWith('ja') ? 'ja' : 'en';
  } catch {
    return 'ja';
  }
}

export function isValidTimeZone(tz: string): boolean {
  if (tz === 'system') return true;
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

function load(): DisplayPrefs {
  const fallback: DisplayPrefs = { locale: defaultLocale(), fontScale: 1, timeZone: 'system' };
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? 'null');
    if (!saved) return fallback;
    return {
      locale: saved.locale === 'en' ? 'en' : saved.locale === 'ja' ? 'ja' : fallback.locale,
      fontScale: typeof saved.fontScale === 'number' ? Math.min(1.5, Math.max(0.85, saved.fontScale)) : 1,
      timeZone: typeof saved.timeZone === 'string' && isValidTimeZone(saved.timeZone) ? saved.timeZone : 'system',
    };
  } catch {
    return fallback;
  }
}

export const prefs = $state<DisplayPrefs>(load());

export function savePrefs() {
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    /* 保存領域が使えなくても表示は続ける */
  }
}

/** OS のタイムゾーン名 */
export function systemTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    return 'UTC';
  }
}

/** 選択肢に出すタイムゾーン（実行環境が知っているものすべて。無ければ代表的なもの） */
export function availableTimeZones(): string[] {
  try {
    const list = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone');
    if (list && list.length > 0) return list.includes('UTC') ? list : ['UTC', ...list];
  } catch {
    /* 下の代表的な一覧を使う */
  }
  return ['UTC', 'Asia/Tokyo', 'Asia/Shanghai', 'Asia/Kolkata', 'Europe/London', 'Europe/Berlin', 'America/New_York', 'America/Los_Angeles', 'Australia/Sydney'];
}

function zone(): string | undefined {
  return prefs.timeZone === 'system' ? undefined : prefs.timeZone;
}

function parts(value: Date, options: Intl.DateTimeFormatOptions): Record<string, string> {
  const out: Record<string, string> = {};
  for (const p of new Intl.DateTimeFormat('en-US', { ...options, timeZone: zone(), hourCycle: 'h23' }).formatToParts(value)) {
    out[p.type] = p.value;
  }
  return out;
}

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * 日付を YYYY-MM-DD で。日時（ISO 8601）なら選択中のタイムゾーンの日付にする。
 * 日付だけの値（EOL の日付など）はタイムゾーンに関係ないのでそのまま。
 */
export function formatDate(value: string | null | undefined): string {
  if (!value) return '';
  if (DATE_ONLY.test(value)) return value;
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value.slice(0, 10);
  const p = parts(d, { year: 'numeric', month: '2-digit', day: '2-digit' });
  return `${p.year}-${p.month}-${p.day}`;
}

/** 時刻を HH:mm:ss で（選択中のタイムゾーン）。 */
export function formatTime(value: string | null | undefined): string {
  if (!value) return '';
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  const p = parts(d, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  return `${p.hour}:${p.minute}:${p.second}`;
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return '';
  return DATE_ONLY.test(value) ? value : `${formatDate(value)} ${formatTime(value)}`;
}

/** 画面に出すタイムゾーン名（「システム」の場合は実際の名前） */
export function timeZoneLabel(): string {
  return prefs.timeZone === 'system' ? systemTimeZone() : prefs.timeZone;
}

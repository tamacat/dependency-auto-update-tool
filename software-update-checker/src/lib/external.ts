import { message } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { t } from './i18n';

/** 開いてよい外部ページ（src-tauri/capabilities/default.json の opener の許可と同じ）。 */
const ALLOWED = ['https://osv.dev/', 'https://endoflife.date/'];

/** 外部ページを既定のブラウザで開く。許可していない URL は開かない。 */
export async function openExternal(url: string) {
  if (!ALLOWED.some((prefix) => url.startsWith(prefix))) {
    await message(t('external.blocked', { url }), { kind: 'warning' });
    return;
  }
  try {
    await openUrl(url);
  } catch (e) {
    await message(String(e), { title: t('external.failed'), kind: 'error' });
  }
}

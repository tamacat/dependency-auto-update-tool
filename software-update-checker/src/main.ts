import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';

// 開発時に通常のブラウザで開いた場合は、模擬バックエンドで画面だけ確認できるようにする
if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) {
  (await import('./lib/mock')).install();
}

const app = mount(App, { target: document.getElementById('app')! });

export default app;

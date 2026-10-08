import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri の開発サーバー設定。ポートは tauri.conf.json の devUrl と合わせる。
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'es2022',
    // 第三者ライセンス表記（約 600KB のテキスト）を別ファイルで同梱しているため、警告の基準を上げる
    chunkSizeWarningLimit: 700,
  },
});

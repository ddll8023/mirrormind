/** 配置独立的主窗口与快速记录入口，开发资源仅绑定本机。 */
import { fileURLToPath } from 'node:url'
import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true,
    hmr: { host: '127.0.0.1', port: 1421 },
    watch: { ignored: ['**/src-tauri/**', '**/data/**'] },
  },
  build: {
    target: 'es2022',
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        capture: fileURLToPath(new URL('./capture.html', import.meta.url)),
      },
    },
  },
})

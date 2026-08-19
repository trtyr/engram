
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': import.meta.dirname + '/src',
    },
  },
  // 开发时后端在 :8080（compose 或 cargo run）
  server: {
    proxy: {
      '/api': 'http://localhost:8080',
    },
  },
})

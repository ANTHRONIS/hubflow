import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 3000,
    // Direct connections to 127.0.0.1:8080 are used in the frontend code;
    // the Rust server already has CorsLayer::permissive() so no proxy needed.
  },
})

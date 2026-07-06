import { defineConfig } from 'vite'
import { svelte } from '@sveltejs/vite-plugin-svelte'

export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/api': 'http://localhost:8400',
      '/events': { target: 'ws://localhost:8400', ws: true },
    }
  },
  build: {
    outDir: 'dist'
  }
})

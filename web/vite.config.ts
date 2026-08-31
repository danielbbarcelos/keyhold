import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))

/**
 * The bundle is compiled into the keyhold binary, so two things are non-negotiable here.
 *
 * `assetsInlineLimit: 0` keeps assets as real files rather than `data:` URLs, because the CSP the
 * binary serves does not allow inline anything and a silently inlined asset would be a blank page
 * nobody could explain.
 *
 * Nothing may be fetched at runtime. There is no CDN, no font host, no analytics endpoint: the
 * page the vault renders on must be entirely present in the binary, or the offline promise is a
 * promise about a good day.
 */
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': resolve(here, 'src'),
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/[name]-[hash].js',
        chunkFileNames: 'assets/[name]-[hash].js',
        assetFileNames: 'assets/[name]-[hash][extname]',
      },
    },
  },
  server: {
    port: 5273,
    proxy: {
      '/api': 'http://127.0.0.1:5274',
      '/health': 'http://127.0.0.1:5274',
    },
  },
})

import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tsconfigPaths from 'vite-tsconfig-paths'
import tailwindcss from '@tailwindcss/vite'
import { TanStackRouterVite } from '@tanstack/router-plugin/vite'

export default defineConfig({
  plugins: [
    react(),
    tsconfigPaths(),
    tailwindcss(),
    TanStackRouterVite()
  ],
  server: {
    port: 3001,
    proxy: {
      '/api': {
        target: 'http://localhost:8001',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://localhost:8001',
        ws: true
      }
    }
  },
  build: {
    outDir: 'dist',
    sourcemap: true
  }
})

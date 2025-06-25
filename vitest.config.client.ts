import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: 'jsdom',
    setupFiles: ['./client/src/test-utils/setup.ts'],
    include: ['client/**/*.test.{ts,tsx}'],
    exclude: ['**/node_modules/**', '**/dist/**'],
  },
  resolve: {
    alias: {
      '@': new URL('./client/src', import.meta.url).pathname,
      '@shared': new URL('./shared', import.meta.url).pathname,
    },
  },
})
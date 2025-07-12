import path from 'node:path'
import react from '@vitejs/plugin-react'
import tsconfigPaths from 'vite-tsconfig-paths'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  plugins: [react(), tsconfigPaths()],
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: [path.resolve(__dirname, './src/test-utils/setup.ts')],
    include: ['src/**/*.test.{ts,tsx}', 'src/**/__tests__/**/*.{ts,tsx}'],
    exclude: ['**/test-helpers.ts', '**/test-helpers.tsx', '**/node_modules/**'],
    coverage: {
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        'src/test-utils/**',
        'src/**/*.test.{ts,tsx}',
        'src/**/__tests__/**',
        'src/types/**',
        'src/main.tsx',
        'src/routeTree.gen.ts',
      ],
    },
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
      '@moatless/shared': path.resolve(__dirname, '../../packages/shared'),
      '@test-utils': path.resolve(__dirname, './src/test-utils'),
    },
  },
  define: {
    'process.env.NODE_ENV': JSON.stringify('test'),
  },
})

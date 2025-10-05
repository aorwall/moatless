import path from 'node:path'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    setupFiles: ['./src/test/setup.ts'],
    environment: 'node',
    include: [
      '**/__tests__/**/*.test.{ts,js}',
      '**/*.test.{ts,js}',
      '**/*.unit.test.{ts,js}',
      '**/*.integration.test.{ts,js}',
    ],
    exclude: ['**/node_modules/**', '**/dist/**', '**/coverage/**'],
    testTimeout: 30000,
    hookTimeout: 10000,
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      exclude: [
        'node_modules/',
        'test/',
        '**/*.test.ts',
        '**/__tests__/**',
        '**/types/**',
        '**/*.d.ts',
        'dist/',
      ],
    },
  },
  resolve: {
    alias: {
      '@server': path.resolve(__dirname, './src'),
      '@moatless/types': path.resolve(__dirname, '../../packages/types/src'),
    },
  },
})

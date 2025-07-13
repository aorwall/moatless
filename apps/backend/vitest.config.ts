import path from 'node:path'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    // Setup file to run before all tests
    setupFiles: ['./test/setup.ts'],

    // Test environment
    environment: 'node',

    // Include test files
    include: ['**/__tests__/**/*.test.{ts,js}', '**/*.test.{ts,js}'],

    // Exclude patterns
    exclude: ['**/node_modules/**', '**/dist/**', '**/coverage/**'],

    // Test timeout (30 seconds for server tests that might involve file operations)
    testTimeout: 30000,

    // Hook timeout for setup/teardown
    hookTimeout: 10000,

    // Run tests in sequence to avoid conflicts with temp directories
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },

    // Coverage configuration
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
      '@server': path.resolve(__dirname, '.'),
      '@moatless/types': path.resolve(__dirname, '../../packages/types'),
    },
  },
})

import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['src/**/*.test.ts', 'src/**/*.integration.test.ts'],
    testTimeout: 60000, // 60 second timeout for integration tests
    hookTimeout: 30000,
    // Run tests sequentially since we're testing a single server
    pool: 'forks',
    poolOptions: {
      forks: {
        singleFork: true,
      },
    },
  },
  resolve: {
    alias: {
      '@': './src',
    },
  },
})

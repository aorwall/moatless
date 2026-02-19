import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['**/*.e2e.test.ts'],
    testTimeout: 600_000, // 10 minutes per test
    hookTimeout: 120_000, // 2 minutes for setup/teardown
    pool: 'forks',
    poolOptions: {
      forks: { singleFork: true },
    },
  },
})

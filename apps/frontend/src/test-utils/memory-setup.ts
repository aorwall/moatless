import { afterEach, vi } from 'vitest'

// Aggressive cleanup for memory-heavy tests
afterEach(() => {
  // Clear DOM completely
  document.body.innerHTML = ''
  document.head.innerHTML = '<title>Test</title>'

  // Clear all timers and intervals
  vi.clearAllTimers()
  vi.clearAllMocks()

  // Force garbage collection if available
  if (global.gc) {
    global.gc()
  }
})

// Reduce test timeout for memory tests
vi.setConfig({ testTimeout: 30000 })

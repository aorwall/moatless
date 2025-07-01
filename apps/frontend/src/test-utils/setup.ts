import '@testing-library/jest-dom'
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

// React 19 compatibility: Set the act environment flag
// This tells React Testing Library that we're in a test environment
if (!globalThis.IS_REACT_ACT_ENVIRONMENT) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true
}

// Mock browser APIs that are not available in jsdom
global.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

// Cleanup after each test
afterEach(() => {
  cleanup()
})

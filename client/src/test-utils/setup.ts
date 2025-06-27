import '@testing-library/jest-dom'
import { cleanup } from '@testing-library/react'
import { afterEach } from 'vitest'

// Import React to ensure it's available in the test environment
import * as React from 'react'

// Make React available globally for JSX
globalThis.React = React

// Cleanup after each test
afterEach(() => {
  cleanup()
})

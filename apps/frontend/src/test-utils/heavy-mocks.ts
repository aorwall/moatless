import React from 'react'
import { vi } from 'vitest'

// Mock heavy dependencies aggressively
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(() => Promise.resolve({ svg: '<svg>mocked</svg>' })),
  },
}))

vi.mock('@git-diff-view/react', () => ({
  DiffView: () => React.createElement('div', { 'data-testid': 'mock-diff' }, 'Mocked Diff'),
  parseDiff: vi.fn(() => []),
}))

vi.mock('react-markdown', () => ({
  default: ({ children }: any) =>
    React.createElement('div', { 'data-testid': 'mock-markdown' }, children),
}))

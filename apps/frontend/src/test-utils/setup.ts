import '@testing-library/jest-dom'
import { cleanup } from '@testing-library/react'
import React from 'react'
import { afterEach, vi } from 'vitest'

// React 19 compatibility: Set the act environment flag
// This tells React Testing Library that we're in a test environment
if (!(globalThis as any).IS_REACT_ACT_ENVIRONMENT) {
  ;(globalThis as any).IS_REACT_ACT_ENVIRONMENT = true
}

// Mock browser APIs that are not available in jsdom
global.ResizeObserver = class ResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

// Mock heavy dependencies to improve test performance
vi.mock('@git-diff-view/react', () => ({
  DiffView: ({ children }: { children?: React.ReactNode }) =>
    React.createElement('div', { 'data-testid': 'mock-diff-view' }, children),
  parseDiff: vi.fn(() => []),
}))

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: React.ReactNode }) =>
    React.createElement('div', { 'data-testid': 'mock-markdown' }, children),
}))

vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn(() => Promise.resolve({ svg: '<svg></svg>' })),
  },
}))

vi.mock('react-resizable-panels', () => ({
  Panel: ({ children, ...props }: any) =>
    React.createElement('div', { 'data-testid': 'mock-panel', ...props }, children),
  PanelGroup: ({ children, ...props }: any) =>
    React.createElement('div', { 'data-testid': 'mock-panel-group', ...props }, children),
  PanelResizeHandle: (props: any) =>
    React.createElement('div', { 'data-testid': 'mock-resize-handle', ...props }),
}))

// Mock tooltip components to avoid accessibility test failures
vi.mock('@radix-ui/react-tooltip', () => ({
  Provider: ({ children }: { children: React.ReactNode }) => children,
  Root: ({ children }: { children: React.ReactNode }) => children,
  Trigger: ({ children, ...props }: any) => React.createElement('div', props, children),
  Content: ({ children, ...props }: any) => React.createElement('div', props, children),
  TooltipProvider: ({ children }: { children: React.ReactNode }) => children,
}))

// Increase test timeout for heavy operations
vi.setConfig({ testTimeout: 10000 })

// Cleanup after each test
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

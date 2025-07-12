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

// Only mock the most problematic components globally
// Individual tests can add their own mocks as needed

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
  Portal: ({ children }: { children: React.ReactNode }) => children,
  Arrow: () => null,
  Trigger: ({ children, asChild, ...props }: any) => {
    // Filter out asChild to prevent React warnings
    const { asChild: _, ...domProps } = props
    return React.createElement('div', domProps, children)
  },
  Content: ({ children, asChild, ...props }: any) => {
    // Filter out asChild to prevent React warnings
    const { asChild: _, ...domProps } = props
    return React.createElement('div', domProps, children)
  },
  TooltipProvider: ({ children }: { children: React.ReactNode }) => children,
  Tooltip: ({ children }: { children: React.ReactNode }) => children,
  TooltipTrigger: ({ children, asChild, ...props }: any) => {
    // Filter out asChild to prevent React warnings
    const { asChild: _, ...domProps } = props
    return React.createElement('div', domProps, children)
  },
  TooltipContent: ({ children }: { children: React.ReactNode }) => children,
  TooltipArrow: () => null,
}))

// Increase test timeout for heavy operations
vi.setConfig({ testTimeout: 10000 })

// Cleanup after each test
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

import '@testing-library/jest-dom'
import { cleanup } from '@testing-library/react'
import React from 'react'
import { afterEach, vi } from 'vitest'

// Mock mermaid to avoid CSS import issues
vi.mock('mermaid', () => ({
  default: {
    initialize: vi.fn(),
    render: vi.fn().mockResolvedValue({ svg: '<svg></svg>', bindFunctions: vi.fn() }),
    parse: vi.fn(),
    parseError: vi.fn(),
    mermaidAPI: {
      render: vi.fn(),
    },
  },
}))

// Mock streamdown to avoid katex CSS import issues
vi.mock('streamdown', () => ({
  Streamdown: ({ children }: { children?: React.ReactNode }) =>
    React.createElement('div', { 'data-testid': 'mock-streamdown' }, children),
}))

// React 19 compatibility: Set the act environment flag
// This tells React Testing Library that we're in a test environment
interface ReactActEnvironmentGlobal {
  IS_REACT_ACT_ENVIRONMENT?: boolean
}

const globalWithReactAct = globalThis as typeof globalThis & ReactActEnvironmentGlobal

if (!globalWithReactAct.IS_REACT_ACT_ENVIRONMENT) {
  globalWithReactAct.IS_REACT_ACT_ENVIRONMENT = true
}

// Mock browser APIs that are not available in jsdom
global.ResizeObserver = class ResizeObserver {
  observe() {
    /* noop */
  }

  unobserve() {
    /* noop */
  }

  disconnect() {
    /* noop */
  }
}

// jsdom doesn't implement matchMedia; many components (use-mobile, theme
// helpers) call it on mount, so stub it with the standard MediaQueryList shape.
if (typeof window !== 'undefined' && typeof window.matchMedia !== 'function') {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    configurable: true,
    value: (query: string) => {
      const noop = () => {
        /* MediaQueryList listener stub */
      }
      return {
        matches: false,
        media: query,
        onchange: null,
        addListener: noop,
        removeListener: noop,
        addEventListener: noop,
        removeEventListener: noop,
        dispatchEvent: () => false,
      }
    },
  })
}

// jsdom in some environments lacks a working localStorage; provide a minimal
// in-memory shim so components that read user prefs (theme, recent tasks) work.
if (typeof window !== 'undefined' && typeof window.localStorage?.getItem !== 'function') {
  const store = new Map<string, string>()
  Object.defineProperty(window, 'localStorage', {
    writable: true,
    configurable: true,
    value: {
      getItem: (key: string) => (store.has(key) ? (store.get(key) ?? null) : null),
      setItem: (key: string, value: string) => {
        store.set(key, String(value))
      },
      removeItem: (key: string) => {
        store.delete(key)
      },
      clear: () => store.clear(),
      key: (index: number) => Array.from(store.keys())[index] ?? null,
      get length() {
        return store.size
      },
    } satisfies Storage,
  })
}

// Only mock the most problematic components globally
// Individual tests can add their own mocks as needed

vi.mock('react-resizable-panels', () => ({
  Panel: ({ children, ...props }: any) =>
    React.createElement('div', { 'data-testid': 'mock-panel', ...props }, children),
  PanelGroup: ({ children, ...props }: any) =>
    React.createElement('div', { 'data-testid': 'mock-panel-group', ...props }, children),
  PanelResizeHandle: (props: any) =>
    React.createElement('div', {
      'data-testid': 'mock-resize-handle',
      ...props,
    }),
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
vi.setConfig({ testTimeout: 10_000 })

// Cleanup after each test
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

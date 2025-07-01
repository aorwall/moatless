import type { MockedFunction } from 'vitest'
import { vi } from 'vitest'

// Mock for SSE connections
export class MockSSEConnection {
  url: string
  options: unknown
  listeners: Map<string, Array<(data: unknown) => void>> = new Map()
  connected = false

  constructor(url: string, options?: unknown) {
    this.url = url
    this.options = options
    this.connected = true
  }

  addEventListener(event: string, callback: (data: unknown) => void) {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, [])
    }
    this.listeners.get(event)!.push(callback)
  }

  removeEventListener(event: string, callback: (data: unknown) => void) {
    const listeners = this.listeners.get(event) || []
    const index = listeners.indexOf(callback)
    if (index > -1) {
      listeners.splice(index, 1)
    }
  }

  close() {
    this.connected = false
    this.listeners.clear()
  }

  // Test helper to emit events
  emit(event: string, data: unknown) {
    const listeners = this.listeners.get(event) || []
    listeners.forEach((callback) => callback({ data }))
  }
}

// Create mocked hooks
export function createMockUseTask() {
  return vi.fn(() => ({
    data: undefined,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }))
}

export function createMockUseTaskEvents() {
  return vi.fn(() => ({
    data: [],
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }))
}

export function createMockUseTaskWorkspaceStatus() {
  return vi.fn(() => ({
    data: { status: 'idle', message: 'Idle' },
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }))
}

// Mock EventSource for SSE
export function mockEventSource() {
  const MockEventSourceConstructor = vi.fn().mockImplementation((url: string) => {
    return new MockSSEConnection(url)
  })

  ;(global as Record<string, unknown>).EventSource = MockEventSourceConstructor

  return MockEventSourceConstructor
}

// Mock fetch for API calls
export function mockFetch() {
  const fetchMock = vi.fn()
  // biome-ignore lint/suspicious/noExplicitAny: Mock for testing
  ;(global as any).fetch = fetchMock
  return fetchMock
}

// Helper to create a mock query client
export function createMockQueryClient() {
  return {
    invalidateQueries: vi.fn().mockResolvedValue(undefined),
    refetchQueries: vi.fn().mockResolvedValue(undefined),
    setQueryData: vi.fn(),
    getQueryData: vi.fn(),
    getQueryState: vi.fn(),
  }
}

// Mock for WebSocket
export class MockWebSocket {
  url: string
  readyState: number = 1 // OPEN
  listeners: Map<string, Array<(event: MessageEvent | Event | CloseEvent) => void>> = new Map()

  constructor(url: string) {
    this.url = url
  }

  addEventListener(event: string, callback: (event: MessageEvent | Event | CloseEvent) => void) {
    if (!this.listeners.has(event)) {
      this.listeners.set(event, [])
    }
    this.listeners.get(event)!.push(callback)
  }

  removeEventListener(event: string, callback: (event: MessageEvent | Event | CloseEvent) => void) {
    const listeners = this.listeners.get(event) || []
    const index = listeners.indexOf(callback)
    if (index > -1) {
      listeners.splice(index, 1)
    }
  }

  send(_data: string) {
    // Mock implementation
  }

  close() {
    this.readyState = 3 // CLOSED
    this.emit('close', {})
  }

  // Test helper to emit events
  emit(event: string, data: unknown) {
    const listeners = this.listeners.get(event) || []
    listeners.forEach((callback) => callback(data as MessageEvent | Event | CloseEvent))
  }
}

// Mock WebSocket
export function mockWebSocket() {
  const MockWebSocketConstructor = vi.fn().mockImplementation((url: string) => {
    return new MockWebSocket(url)
  })

  ;(global as Record<string, unknown>).WebSocket = MockWebSocketConstructor

  return MockWebSocketConstructor
}

// Mock localStorage
export function mockLocalStorage() {
  const storage: Record<string, string> = {}

  return {
    getItem: vi.fn((key: string) => storage[key] || null),
    setItem: vi.fn((key: string, value: string) => {
      storage[key] = value
    }),
    removeItem: vi.fn((key: string) => {
      delete storage[key]
    }),
    clear: vi.fn(() => {
      Object.keys(storage).forEach((key) => delete storage[key])
    }),
  }
}

// Mock for zustand stores
export function createMockStoreHook<T>(initialState: T) {
  let state = initialState
  const listeners = new Set<() => void>()

  const setState = (newState: Partial<T> | ((state: T) => Partial<T>)) => {
    if (typeof newState === 'function') {
      state = { ...state, ...newState(state) }
    } else {
      state = { ...state, ...newState }
    }
    listeners.forEach((listener) => listener())
  }

  const getState = () => state

  const subscribe = (listener: () => void) => {
    listeners.add(listener)
    return () => listeners.delete(listener)
  }

  const useStore = (selector?: (state: T) => unknown) => {
    if (selector) {
      return selector(state)
    }
    return state
  }

  useStore.setState = setState
  useStore.getState = getState
  useStore.subscribe = subscribe

  return useStore
}

// Re-export factories for convenience
export { createMockTask, createMockEvent as createMockTaskEvent } from './factories'

// Type helper for mocked functions
export type Mocked<T> = T extends (...args: unknown[]) => unknown
  ? MockedFunction<T>
  : T extends object
    ? { [K in keyof T]: Mocked<T[K]> }
    : T

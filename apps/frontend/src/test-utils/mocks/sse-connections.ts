import { EventEmitter } from 'node:events'
import { vi } from 'vitest'

/**
 * SSE/WebSocket mocks for testing real-time connections
 */

/**
 * Mock EventSource for testing SSE connections
 */
export class MockEventSource extends EventEmitter {
  public url: string
  public readyState = 0 // CONNECTING
  public withCredentials = false
  public onerror: ((event: Event) => void) | null = null
  public onmessage: ((event: MessageEvent) => void) | null = null
  public onopen: ((event: Event) => void) | null = null
  public close = vi.fn(() => {
    this.readyState = 2 // CLOSED
  })

  public static CONNECTING = 0
  public static OPEN = 1
  public static CLOSED = 2

  constructor(url: string, eventSourceInitDict?: EventSourceInit) {
    super()
    this.url = url
    if (eventSourceInitDict?.withCredentials) {
      this.withCredentials = eventSourceInitDict.withCredentials
    }
  }

  /**
   * Simulate opening the connection
   */
  triggerOpen() {
    this.readyState = 1 // OPEN
    if (this.onopen) {
      this.onopen(new Event('open'))
    }
    this.emit('open')
  }

  /**
   * Simulate receiving a message
   */
  triggerMessage(event: MessageEvent | { data: string; type?: string }) {
    const messageEvent = event instanceof MessageEvent ? event : new MessageEvent('message', event)
    if (this.onmessage) {
      this.onmessage(messageEvent)
    }
    this.emit('message', messageEvent)
  }

  /**
   * Simulate an error
   */
  triggerError(error?: Event) {
    const errorEvent = error || new Event('error')
    if (this.onerror) {
      this.onerror(errorEvent)
    }
    this.emit('error', errorEvent)
  }

  /**
   * Helper to simulate a complete SSE flow
   */
  simulateConnection() {
    setTimeout(() => this.triggerOpen(), 0)
  }

  /**
   * Helper to simulate sending data
   */
  simulateData(data: unknown, type = 'message') {
    setTimeout(() => {
      this.triggerMessage({
        data: typeof data === 'string' ? data : JSON.stringify(data),
        type,
      })
    }, 0)
  }
}

/**
 * Mock WebSocket for testing WebSocket connections
 */
export class MockWebSocket extends EventEmitter {
  public url: string
  public readyState = 0 // CONNECTING
  public protocol = ''
  public extensions = ''
  public binaryType: BinaryType = 'blob'
  public bufferedAmount = 0

  public onopen: ((event: Event) => void) | null = null
  public onclose: ((event: CloseEvent) => void) | null = null
  public onerror: ((event: Event) => void) | null = null
  public onmessage: ((event: MessageEvent) => void) | null = null

  public close = vi.fn((code?: number, reason?: string) => {
    this.readyState = 3 // CLOSED
    const closeEvent = new CloseEvent('close', {
      ...(code !== undefined && { code }),
      ...(reason !== undefined && { reason }),
    })
    if (this.onclose) {
      this.onclose(closeEvent)
    }
    this.emit('close', closeEvent)
  })

  public send = vi.fn((data: string | ArrayBufferLike | Blob | ArrayBufferView) => {
    if (this.readyState !== 1) {
      throw new Error('WebSocket is not open')
    }
    this.emit('send', data)
  })

  public static CONNECTING = 0
  public static OPEN = 1
  public static CLOSING = 2
  public static CLOSED = 3

  constructor(url: string, protocols?: string | string[]) {
    super()
    this.url = url
    if (protocols) {
      this.protocol = Array.isArray(protocols) ? (protocols[0] ?? '') : protocols
    }
  }

  /**
   * Simulate opening the connection
   */
  triggerOpen() {
    this.readyState = 1 // OPEN
    const event = new Event('open')
    if (this.onopen) {
      this.onopen(event)
    }
    this.emit('open', event)
  }

  /**
   * Simulate receiving a message
   */
  triggerMessage(data: string | object) {
    const messageData = typeof data === 'string' ? data : JSON.stringify(data)
    const event = new MessageEvent('message', { data: messageData })
    if (this.onmessage) {
      this.onmessage(event)
    }
    this.emit('message', event)
  }

  /**
   * Simulate an error
   */
  triggerError(error?: Event) {
    const errorEvent = error || new Event('error')
    if (this.onerror) {
      this.onerror(errorEvent)
    }
    this.emit('error', errorEvent)
  }

  /**
   * Simulate closing the connection
   */
  triggerClose(code = 1000, reason = 'Normal closure') {
    this.readyState = 3 // CLOSED
    const event = new CloseEvent('close', { code, reason })
    if (this.onclose) {
      this.onclose(event)
    }
    this.emit('close', event)
  }
}

/**
 * Helper to create and setup mock SSE connection
 */
export function createMockSSEConnection() {
  const mockEventSource = new MockEventSource('test-url')
  const EventSourceSpy = vi.fn(() => mockEventSource)

  // Replace global EventSource
  const originalEventSource = global.EventSource
  global.EventSource = EventSourceSpy as unknown as typeof EventSource

  return {
    mockEventSource,
    EventSourceSpy,
    restore: () => {
      global.EventSource = originalEventSource
    },
  }
}

/**
 * Helper to create and setup mock WebSocket connection
 */
export function createMockWebSocketConnection() {
  const mockWebSocket = new MockWebSocket('ws://test-url')
  const WebSocketSpy = vi.fn(() => mockWebSocket)

  // Replace global WebSocket
  const originalWebSocket = global.WebSocket
  global.WebSocket = WebSocketSpy as unknown as typeof WebSocket

  return {
    mockWebSocket,
    WebSocketSpy,
    restore: () => {
      global.WebSocket = originalWebSocket
    },
  }
}

/**
 * Mock SSE hook return value
 */
export function createMockSSEHook(overrides?: {
  isConnected?: boolean
  isConnecting?: boolean
  error?: Error | null
  disconnect?: () => void
}) {
  return {
    isConnected: false,
    isConnecting: false,
    error: null,
    disconnect: vi.fn(),
    ...overrides,
  }
}

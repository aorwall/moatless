import { vi } from 'vitest'

interface MockSSEConnection {
  url: string
  readyState: number
  close: () => void
  dispatchEvent: (event: Event) => void
  addEventListener: (type: string, listener: EventListener) => void
  removeEventListener: (type: string, listener: EventListener) => void
}

export function createSSEMock() {
  const connections: MockSSEConnection[] = []
  let emitCallbacks: Array<(event: { type: string; data: string }) => void> = []

  class MockEventSource implements MockSSEConnection {
    url: string
    readyState: number = 1 // OPEN
    onopen: ((event: Event) => void) | null = null
    onmessage: ((event: MessageEvent) => void) | null = null
    onerror: ((event: Event) => void) | null = null
    close = vi.fn()
    private listeners: Map<string, Set<EventListener>> = new Map()

    constructor(url: string) {
      this.url = url
      connections.push(this)

      // Simulate connection open
      setTimeout(() => {
        if (this.onopen) {
          this.onopen(new Event('open'))
        }
        this.dispatchEvent(new Event('open'))
      }, 0)
    }

    addEventListener(type: string, listener: EventListener) {
      if (!this.listeners.has(type)) {
        this.listeners.set(type, new Set())
      }
      this.listeners.get(type)!.add(listener)
    }

    removeEventListener(type: string, listener: EventListener) {
      this.listeners.get(type)?.delete(listener)
    }

    dispatchEvent(event: Event) {
      const listeners = this.listeners.get(event.type)
      if (listeners) {
        listeners.forEach(listener => listener(event))
      }

      // Also dispatch to inline handlers
      if (event.type === 'message' && this.onmessage) {
        this.onmessage(event as MessageEvent)
      } else if (event.type === 'error' && this.onerror) {
        this.onerror(event)
      } else if (event.type === 'open' && this.onopen) {
        this.onopen(event)
      }
    }
  }

  const emit = (event: { type: string; data: string }) => {
    connections.forEach(connection => {
      if (connection.readyState === 1) {
        const messageEvent = new MessageEvent('message', {
          data: event.data,
        })
        connection.dispatchEvent(messageEvent)
      }
    })

    emitCallbacks.forEach(callback => callback(event))
  }

  const onEmit = (callback: (event: { type: string; data: string }) => void) => {
    emitCallbacks.push(callback)
    return () => {
      emitCallbacks = emitCallbacks.filter(cb => cb !== callback)
    }
  }

  const cleanup = () => {
    connections.forEach(conn => {
      conn.readyState = 2 // CLOSED
      conn.close()
    })
    connections.length = 0
    emitCallbacks = []
  }

  return {
    EventSource: MockEventSource,
    connections,
    emit,
    onEmit,
    cleanup,
  }
}
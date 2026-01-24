import type { InspectorOutboundMessage, InternalLogger } from './types'

/** Save original console methods before interception */
const originalConsole = {
  log: console.log,
  warn: console.warn,
  error: console.error,
  info: console.info,
  debug: console.debug,
}

/** Internal logger that always uses original console (bypasses interception) */
export const internalLogger: InternalLogger = {
  log: (...args: unknown[]) => originalConsole.log(...args),
  warn: (...args: unknown[]) => originalConsole.warn(...args),
  error: (...args: unknown[]) => originalConsole.error(...args),
  info: (...args: unknown[]) => originalConsole.info(...args),
  debug: (...args: unknown[]) => originalConsole.debug(...args),
}

/** Send a message to the parent window via postMessage */
export function sendMessage(message: InspectorOutboundMessage): void {
  try {
    const envelope = { ...message, source: 'preview-iframe' }
    if (message.type !== 'HEARTBEAT') {
      internalLogger.log('[Preview Inspector] Sending message to parent:', envelope)
    }
    window.parent.postMessage(envelope, '*')
  } catch (error) {
    internalLogger.error('preview-inspector: could not post to parent', error)
  }
}

/** Create a debounced version of a function */
export function debounce<T extends (...args: unknown[]) => void>(
  fn: T,
  delay: number,
): (...args: Parameters<T>) => void {
  let timer: ReturnType<typeof setTimeout> | undefined
  return (...args: Parameters<T>) => {
    if (timer !== undefined) {
      clearTimeout(timer)
    }
    timer = setTimeout(() => fn(...args), delay)
  }
}

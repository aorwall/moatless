import { sendMessage } from './communication'

type ConsoleLevel = 'log' | 'warn' | 'error' | 'info' | 'debug'

const originalConsole: Record<ConsoleLevel, (...args: unknown[]) => void> = {
  log: console.log,
  warn: console.warn,
  error: console.error,
  info: console.info,
  debug: console.debug,
}

/** Install console interceptors that forward messages to the parent window */
export function installConsoleInterceptors(): void {
  const levels: ConsoleLevel[] = ['log', 'warn', 'error', 'info', 'debug']

  for (const level of levels) {
    console[level] = (...args: unknown[]) => {
      // Call the original console method
      originalConsole[level].apply(console, args)

      // Capture stack trace for errors and warnings
      const stack =
        level === 'error' || level === 'warn'
          ? new Error().stack?.split('\n').slice(2).join('\n')
          : undefined

      // Serialize arguments to a readable message
      const message = args
        .map((arg) => {
          if (typeof arg === 'object') {
            try {
              return JSON.stringify(arg, null, 2)
            } catch {
              return String(arg)
            }
          }
          return String(arg)
        })
        .join(' ')

      sendMessage({
        type: 'console',
        level,
        message,
        args,
        stack,
      })
    }
  }
}

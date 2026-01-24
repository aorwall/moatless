import { sendMessage } from './communication'

/** Install global error and unhandled rejection handlers */
export function installErrorHandlers(): void {
  window.addEventListener('error', (event) => {
    sendMessage({
      type: 'console',
      level: 'error',
      message: `${event.message} at ${event.filename}:${event.lineno}:${event.colno}`,
      args: [event.message],
      stack: event.error?.stack,
    })
  })

  window.addEventListener('unhandledrejection', (event) => {
    const stack =
      event.reason?.stack ||
      (event.reason instanceof Error ? String(event.reason.stack) : undefined)

    sendMessage({
      type: 'console',
      level: 'error',
      message: `Unhandled Promise Rejection: ${String(event.reason)}`,
      args: [String(event.reason)],
      stack,
    })
  })
}

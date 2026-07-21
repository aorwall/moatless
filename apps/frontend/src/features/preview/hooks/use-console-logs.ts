import { useState } from 'react'

export interface PreviewConsoleLog {
  id: string
  timestamp: string
  level: 'log' | 'info' | 'warn' | 'error' | 'debug'
  message: string
  args?: unknown[]
  stack?: string
}

/** Console messages forwarded from the preview iframe's injected script. */
export function useConsoleLogs() {
  const [consoleLogs, setConsoleLogs] = useState<PreviewConsoleLog[]>([])

  const appendConsoleMessage = (data: MessageEvent['data']) => {
    const { level, message, args, stack } = data

    setConsoleLogs((prev) => [
      ...prev,
      {
        id: `${Date.now()}-${Math.random()}`,
        timestamp: new Date().toISOString(),
        level,
        message,
        args,
        stack,
      },
    ])
  }

  const clearConsoleLogs = () => {
    setConsoleLogs([])
  }

  return { consoleLogs, appendConsoleMessage, clearConsoleLogs }
}

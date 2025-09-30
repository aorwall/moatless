import { useEffect, useRef, useState } from 'react'
import { ScrollArea } from '@/lib/components/ui/scroll-area'

interface ConsoleLog {
  id: string
  timestamp: string
  level: 'log' | 'info' | 'warn' | 'error' | 'debug' | 'route'
  message: string
  args?: unknown[]
  stack?: string
}

interface ConsoleMessage {
  type: 'console'
  level: ConsoleLog['level']
  message: string
  args?: unknown[]
  stack?: string
}

export function ConsoleLogs() {
  const [logs, setLogs] = useState<ConsoleLog[]>([])
  const scrollAreaRef = useRef<HTMLDivElement>(null)
  const shouldAutoScroll = useRef(true)

  // Listen for console messages from the preview iframe
  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      // Only process messages from the preview iframe
      if (event.data?.source === 'preview-iframe') {
        if (event.data.type === 'console') {
          const { level, message, args, stack } = event.data as ConsoleMessage & { source: string }

          const newLog: ConsoleLog = {
            id: `${Date.now()}-${Math.random()}`,
            timestamp: new Date().toISOString(),
            level,
            message,
            args,
            stack,
          }

          setLogs((prev) => [...prev, newLog])
        } else if (event.data.type === 'ROUTE_CHANGE') {
          const { path } = event.data.payload
          const routeLog: ConsoleLog = {
            id: `${Date.now()}-${Math.random()}`,
            timestamp: new Date().toISOString(),
            level: 'route',
            message: `Navigated to ${path}`,
            args: [path],
          }

          setLogs((prev) => [...prev, routeLog])
        }
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [])

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (shouldAutoScroll.current && scrollAreaRef.current) {
      const scrollContainer = scrollAreaRef.current.querySelector(
        '[data-radix-scroll-area-viewport]',
      )
      if (scrollContainer) {
        // Use requestAnimationFrame to ensure the DOM has updated
        requestAnimationFrame(() => {
          scrollContainer.scrollTop = scrollContainer.scrollHeight
        })
      }
    }
  })

  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const scrollContainer = e.currentTarget.querySelector('[data-radix-scroll-area-viewport]')
    if (!scrollContainer) return

    const { scrollTop, scrollHeight, clientHeight } = scrollContainer
    const isNearBottom = scrollHeight - scrollTop - clientHeight < 50

    shouldAutoScroll.current = isNearBottom
  }

  const getLevelColor = (level: ConsoleLog['level']) => {
    switch (level) {
      case 'error':
        return 'text-status-error'
      case 'warn':
        return 'text-status-warning'
      case 'info':
        return 'text-status-info'
      case 'debug':
        return 'text-muted-foreground'
      case 'route':
        return 'text-status-success'
      default:
        return 'text-foreground'
    }
  }

  const formatMessage = (log: ConsoleLog) => {
    const timestamp = new Date(log.timestamp).toLocaleTimeString()
    if (log.level === 'route') {
      return `[${timestamp}] [🧭 NAVIGATION] ${log.message}`
    }
    const prefix = `[${timestamp}] [${log.level.toUpperCase()}]`
    return `${prefix} ${log.message}`
  }

  const formatStackTrace = (stack: string) => {
    // Split by newlines and filter out the console call itself and other noise
    const lines = stack.split('\n').filter((line) => {
      const trimmed = line.trim()
      // Filter out console call and internal preview script lines
      return (
        trimmed &&
        !trimmed.includes('console.<computed>') &&
        !trimmed.includes('Preview Inspector') &&
        !trimmed.includes('hookConsole')
      )
    })

    return lines.join('\n').trim()
  }

  if (logs.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center">
        <div className="text-muted-foreground">
          <p className="text-lg font-medium">No console output</p>
          <p className="mt-2 text-sm">Console logs from the preview app will appear here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="h-full w-full flex flex-col bg-background">
      <ScrollArea ref={scrollAreaRef} className="flex-1 w-full" onScrollCapture={handleScroll}>
        <div className="p-4 space-y-1">
          {logs.map((log) => (
            <div
              key={log.id}
              className="text-xs leading-relaxed font-mono whitespace-pre-wrap break-all"
            >
              <span className={getLevelColor(log.level)}>{formatMessage(log)}</span>
              {log.stack && (
                <div className="mt-1 pl-4 text-muted-foreground text-xs">
                  {formatStackTrace(log.stack)}
                </div>
              )}
            </div>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}

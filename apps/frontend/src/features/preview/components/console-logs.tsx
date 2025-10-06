import { ArrowDown } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/lib/components/ui/button'
import { ScrollArea } from '@/lib/components/ui/scroll-area'
import { useAutoScroll } from '@/lib/hooks/use-auto-scroll'

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

interface ConsoleLogsProps {
  logs?: ConsoleLog[]
}

export function ConsoleLogs({ logs: propLogs }: ConsoleLogsProps = {}) {
  const [internalLogs, setInternalLogs] = useState<ConsoleLog[]>([])
  const bottomRef = useRef<HTMLDivElement>(null)

  // Use prop logs if provided, otherwise use internal state
  const logs = propLogs ?? internalLogs

  // Use auto-scroll hook for proper scroll management
  const {
    scrollRef: scrollAreaRef,
    isAtBottom,
    scrollToBottom,
  } = useAutoScroll({
    scrollOnMount: true,
    dependencies: [logs], // Re-evaluate scroll when logs change
  })

  // Listen for console messages from the preview iframe (only if logs not provided via props)
  useEffect(() => {
    // Skip if using prop logs
    if (propLogs) {
      return
    }

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

          setInternalLogs((prev) => [...prev, newLog])
        } else if (event.data.type === 'ROUTE_CHANGE') {
          const { path } = event.data.payload
          const routeLog: ConsoleLog = {
            id: `${Date.now()}-${Math.random()}`,
            timestamp: new Date().toISOString(),
            level: 'route',
            message: `Navigated to ${path}`,
            args: [path],
          }

          setInternalLogs((prev) => [...prev, routeLog])
        }
      }
    }

    window.addEventListener('message', handleMessage)
    return () => window.removeEventListener('message', handleMessage)
  }, [propLogs])

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
    <div className="h-full w-full bg-background relative">
      <ScrollArea ref={scrollAreaRef} className="h-full w-full">
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
          <div ref={bottomRef} />
        </div>
      </ScrollArea>

      {/* Scroll to bottom button */}
      {!isAtBottom && (
        <div className="absolute bottom-4 right-4 z-10">
          <Button
            variant="outline"
            size="sm"
            onClick={scrollToBottom}
            className="shadow-lg bg-background/80 backdrop-blur-sm border-border/50 hover:bg-background/90 flex items-center gap-2"
          >
            <ArrowDown className="h-4 w-4" />
            Scroll to bottom
          </Button>
        </div>
      )}
    </div>
  )
}
